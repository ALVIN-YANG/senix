use std::{fs, net::SocketAddr, path::Path, pin::Pin, sync::Arc, time::Duration};

use axum::{
    body::Bytes,
    http::{HeaderMap, Request, Response, Version},
};
use openssl::ssl::{Ssl, SslAcceptor, SslConnector, SslFiletype, SslMethod};
use tokio::{net::TcpListener, task::JoinHandle};

use super::{
    admin_json_with_bearer, assert_long_lived_duration_count, bootstrap_owner_key, free_address,
    proxy_get, spawn_backend, spawn_senix_with_tls, wait_for_health_state, wait_for_operation_idle,
    wait_until_ready, write_config, write_test_certificate,
};

#[test]
fn http2_grpc_preserves_bidirectional_messages_and_trailers_during_drain() {
    run_grpc_drain_round_trip("0");
}

#[test]
fn http2_grpc_preserves_error_trailers_with_http_200_during_drain() {
    run_grpc_drain_round_trip("7");
}

fn run_grpc_drain_round_trip(grpc_status: &'static str) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    write_test_certificate(&cert, &key);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let backend_a = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let backend_b = spawn_backend("B", Duration::ZERO);
    write_config(&config, backend_a, backend_b);
    let mut gateway: serde_json::Value =
        serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    gateway["routes"][0]["backends"][0]["tls"] = serde_json::json!({
        "server_name": "example.test", "verify_certificate": false, "alpn": "http2"
    });
    fs::write(&config, serde_json::to_vec_pretty(&gateway).unwrap()).unwrap();
    let owner_key = bootstrap_owner_key(&db);
    let proxy = free_address();
    let tls = free_address();
    let admin = free_address();
    let senix = spawn_senix_with_tls(proxy, tls, admin, &db, &config, &cert, &key);
    wait_until_ready(admin, proxy);
    wait_until_ready(admin, tls);
    wait_for_health_state(admin, "instance-b", "HEALTHY", &owner_key);

    let acceptor = h2_acceptor(&cert, &key);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(15), async {
                let listener = TcpListener::from_std(listener).unwrap();
                let backend = tokio::spawn(serve_grpc_connection(listener, acceptor, grpc_status));
                exercise_grpc_stream(tls, admin, proxy, &cert, &owner_key, grpc_status).await;
                backend.await.unwrap();
            })
            .await
            .expect("gRPC round trip did not finish within 15 seconds");
        });
    drop(senix);
}

async fn exercise_grpc_stream(
    tls: SocketAddr,
    admin: SocketAddr,
    proxy: SocketAddr,
    cert: &Path,
    owner_key: &str,
    grpc_status: &str,
) {
    let (client, driver) = connect_h2(tls, cert).await;
    let mut client = client.ready().await.unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("https://example.test/proof.Echo/Chat")
        .header("content-type", "application/grpc+proto")
        .header("te", "trailers")
        .header("x-client-proof", "metadata-preserved")
        .body(())
        .unwrap();
    let (response, mut outgoing) = client.send_request(request, false).unwrap();
    let first = grpc_frame("before drain 你好");
    // Split inside the five-byte gRPC prefix; HTTP/2 DATA boundaries are independent.
    outgoing.send_data(first.slice(..2), false).unwrap();
    outgoing.send_data(first.slice(2..), false).unwrap();
    let response = response.await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.version(), Version::HTTP_2);
    assert_eq!(response.headers()["content-type"], "application/grpc+proto");
    assert_eq!(response.headers()["x-server-proof"], "headers-preserved");
    let mut incoming = response.into_body();
    assert_eq!(read_grpc_frame(&mut incoming).await, first);

    let operation = admin_json_with_bearer(
        admin,
        "POST",
        "/api/v1/instances/instance-a/drain",
        Some("drain-real-grpc"),
        Some(serde_json::json!({"timeout_ms": 10_000})),
        owner_key,
    );
    assert_eq!(operation["status"], "DRAINING");
    assert_eq!(operation["ordinary_in_flight"], 0);
    assert_eq!(operation["long_lived_in_flight"], 1);
    assert_long_lived_duration_count(admin, owner_key, 0);
    for _ in 0..3 {
        assert_eq!(proxy_get(proxy, "/after-grpc-drain"), "B");
    }
    let second = grpc_frame("after drain 摘流后");
    outgoing.send_data(second.clone(), true).unwrap();
    assert_eq!(read_grpc_frame(&mut incoming).await, second);
    assert!(incoming.data().await.is_none());
    let trailers = incoming
        .trailers()
        .await
        .unwrap()
        .expect("missing gRPC trailers");
    assert_eq!(trailers["grpc-status"], grpc_status);
    assert_eq!(trailers["grpc-message"], "done%20%E5%AE%8C%E6%88%90");
    assert_eq!(trailers["proof-bin"], "AAEC/w==");
    assert!(incoming.is_end_stream());
    wait_for_operation_idle(
        admin,
        operation["operation_id"].as_str().unwrap(),
        "DRAINED",
        owner_key,
    );
    assert_long_lived_duration_count(admin, owner_key, 1);
    drop(client);
    driver.abort();
    let _ = driver.await;
}

fn h2_acceptor(cert: &Path, key: &Path) -> Arc<SslAcceptor> {
    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor.set_certificate_chain_file(cert).unwrap();
    acceptor
        .set_private_key_file(key, SslFiletype::PEM)
        .unwrap();
    acceptor.check_private_key().unwrap();
    acceptor.set_alpn_select_callback(|_, protocols| {
        openssl::ssl::select_next_proto(b"\x02h2", protocols).ok_or(openssl::ssl::AlpnError::NOACK)
    });
    Arc::new(acceptor.build())
}

async fn connect_h2(
    address: SocketAddr,
    cert: &Path,
) -> (
    h2::client::SendRequest<Bytes>,
    JoinHandle<Result<(), h2::Error>>,
) {
    let mut connector = SslConnector::builder(SslMethod::tls()).unwrap();
    connector.set_ca_file(cert).unwrap();
    connector.set_alpn_protos(b"\x02h2").unwrap();
    let ssl = connector
        .build()
        .configure()
        .unwrap()
        .into_ssl("example.test")
        .unwrap();
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut stream = tokio_openssl::SslStream::new(ssl, stream).unwrap();
    Pin::new(&mut stream).connect().await.unwrap();
    assert_eq!(
        stream.ssl().selected_alpn_protocol(),
        Some(b"h2".as_slice())
    );
    let (client, connection) = h2::client::handshake(stream).await.unwrap();
    (client, tokio::spawn(connection))
}

async fn serve_grpc_connection(
    listener: TcpListener,
    acceptor: Arc<SslAcceptor>,
    grpc_status: &'static str,
) {
    let (stream, _) = listener.accept().await.unwrap();
    let ssl = Ssl::new(acceptor.context()).unwrap();
    let mut stream = tokio_openssl::SslStream::new(ssl, stream).unwrap();
    Pin::new(&mut stream).accept().await.unwrap();
    assert_eq!(
        stream.ssl().selected_alpn_protocol(),
        Some(b"h2".as_slice())
    );
    let mut connection = h2::server::handshake(stream).await.unwrap();
    let (request, response) = connection.accept().await.unwrap().unwrap();
    let handler = tokio::spawn(echo_grpc_stream(request, response, grpc_status));
    connection.graceful_shutdown();
    if let Some(request) = connection.accept().await {
        panic!("unexpected extra HTTP/2 stream: {request:?}");
    }
    handler.await.unwrap();
}

async fn echo_grpc_stream(
    mut request: Request<h2::RecvStream>,
    mut respond: h2::server::SendResponse<Bytes>,
    grpc_status: &str,
) {
    assert_eq!(request.method(), "POST");
    assert_eq!(request.uri().path(), "/proof.Echo/Chat");
    assert_eq!(request.version(), Version::HTTP_2);
    assert_eq!(request.headers()["content-type"], "application/grpc+proto");
    assert_eq!(request.headers()["te"], "trailers");
    assert_eq!(request.headers()["x-client-proof"], "metadata-preserved");
    let first = read_grpc_frame(request.body_mut()).await;
    assert_eq!(first, grpc_frame("before drain 你好"));
    let response = Response::builder()
        .status(200)
        .header("content-type", "application/grpc+proto")
        .header("x-server-proof", "headers-preserved")
        .body(())
        .unwrap();
    let mut outgoing = respond.send_response(response, false).unwrap();
    outgoing.send_data(first.slice(..3), false).unwrap();
    outgoing.send_data(first.slice(3..), false).unwrap();
    let second = read_grpc_frame(request.body_mut()).await;
    assert_eq!(second, grpc_frame("after drain 摘流后"));
    outgoing.send_data(second, false).unwrap();
    assert!(request.body_mut().data().await.is_none());
    let mut trailers = HeaderMap::new();
    trailers.insert("grpc-status", grpc_status.parse().unwrap());
    trailers.insert("grpc-message", "done%20%E5%AE%8C%E6%88%90".parse().unwrap());
    trailers.insert("proof-bin", "AAEC/w==".parse().unwrap());
    outgoing.send_trailers(trailers).unwrap();
}

// Fixture protobuf: message Echo { string text = 1; }, without compression.
fn grpc_frame(text: &str) -> Bytes {
    let text_len = u8::try_from(text.len()).unwrap();
    assert!(text_len < 128, "fixture uses a one-byte protobuf varint");
    let message_len = u32::from(text_len) + 2;
    let mut frame = vec![0];
    frame.extend_from_slice(&message_len.to_be_bytes());
    frame.extend_from_slice(&[0x0a, text_len]);
    frame.extend_from_slice(text.as_bytes());
    Bytes::from(frame)
}

async fn read_grpc_frame(body: &mut h2::RecvStream) -> Bytes {
    let mut frame = Vec::new();
    loop {
        let chunk = body
            .data()
            .await
            .expect("gRPC ended before a full message")
            .unwrap();
        frame.extend_from_slice(&chunk);
        body.flow_control().release_capacity(chunk.len()).unwrap();
        assert!(
            frame.len() <= 16_384,
            "gRPC fixture message exceeds test limit"
        );
        if frame.len() >= 5 {
            assert_eq!(frame[0], 0, "fixture does not negotiate compression");
            let size =
                usize::try_from(u32::from_be_bytes(frame[1..5].try_into().unwrap())).unwrap();
            if frame.len() >= size + 5 {
                // The next message is gated on this one's response, so it cannot be coalesced here.
                assert_eq!(frame.len(), size + 5);
                return Bytes::from(frame);
            }
        }
    }
}
