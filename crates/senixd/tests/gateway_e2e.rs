use std::{
    fmt::Write as _,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    pin::Pin,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use axum::body::Bytes;
use axum::http::Response;
use openssl::{
    asn1::Asn1Time,
    bn::BigNum,
    hash::MessageDigest,
    pkey::PKey,
    rsa::Rsa,
    ssl::{SslAcceptor, SslConnector, SslFiletype, SslMethod, SslVerifyMode},
    x509::{X509, X509NameBuilder},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn demo_rejects_invalid_ports_before_starting_containers() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/demo.sh");
    for variable in ["SENIX_DEMO_PROXY_PORT", "SENIX_DEMO_ADMIN_PORT"] {
        for port in [
            "0",
            "65536",
            "8080/path",
            "not-a-port",
            "99999999999999999999",
        ] {
            let output = Command::new("sh")
                .arg(&script)
                .env(variable, port)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2));
            assert!(String::from_utf8_lossy(&output.stderr).contains("Demo ports"));
        }
    }
}

#[test]
fn healthcheck_command_checks_the_admin_process() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);

    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let healthy = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["healthcheck", "--address", &admin.to_string()])
        .status()
        .unwrap();
    assert!(healthy.success());

    let unavailable = free_address();
    let unhealthy = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "healthcheck",
            "--address",
            &unavailable.to_string(),
            "--timeout-ms",
            "100",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!unhealthy.success());
    drop(senix);
}

#[cfg(unix)]
#[test]
fn installer_keeps_the_previous_binary_when_a_verified_release_cannot_run() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    let target = match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        current => panic!("unsupported installer test platform: {current:?}"),
    };
    let package = format!("senix-v0.3.1-{target}");
    let asset = format!("{package}.tar.gz");
    let staged = directory.path().join(&package);
    fs::create_dir(&staged).unwrap();
    let candidate = staged.join("senixd");
    // A valid checksum does not prove that the OS loader can execute a release.
    fs::write(
        &candidate,
        "#!/bin/sh\necho 'synthetic loader failure' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755)).unwrap();
    let archive = directory.path().join(&asset);
    assert!(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(directory.path())
            .arg(&package)
            .status()
            .unwrap()
            .success()
    );
    let archive_bytes = fs::read(&archive).unwrap();
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(&archive_bytes) {
        write!(&mut digest, "{byte:02x}").unwrap();
    }
    let checksum = format!("{digest}  {asset}\n");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for stream in listener.incoming().take(2) {
            let mut stream = stream.unwrap();
            let request = read_headers(&mut stream);
            if request.starts_with(&format!("GET /releases/download/v0.3.1/{asset} ")) {
                write_test_http_response(&mut stream, &archive_bytes);
            } else {
                assert!(request.starts_with("GET /releases/download/v0.3.1/checksums.txt "));
                write_test_http_response(&mut stream, checksum.as_bytes());
            }
        }
    });
    let installed = directory.path().join("installed");
    fs::create_dir(&installed).unwrap();
    let original = b"#!/bin/sh\necho 'previous working Senix'\n";
    fs::write(installed.join("senixd"), original).unwrap();
    fs::set_permissions(installed.join("senixd"), fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new("sh")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh"))
        .args(["--version", "v0.3.1", "--install-dir"])
        .arg(&installed)
        .env(
            "SENIX_RELEASE_BASE_URL",
            format!("http://{address}/releases"),
        )
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(installed.join("senixd")).unwrap(), original);
    assert!(String::from_utf8_lossy(&output.stderr).contains("synthetic loader failure"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Installed "));
    let previous = Command::new(installed.join("senixd"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(previous.status.success());
    assert_eq!(previous.stdout, b"previous working Senix\n");
}

#[cfg(unix)]
#[test]
fn installer_retries_an_interrupted_trusted_mirror_download() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    let target = match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        current => panic!("unsupported installer test platform: {current:?}"),
    };
    let version = "v0.3.1";
    let package = format!("senix-{version}-{target}");
    let asset = format!("{package}.tar.gz");
    let package_directory = directory.path().join(&package);
    fs::create_dir(&package_directory).unwrap();
    let fake_binary = package_directory.join("senixd");
    fs::write(&fake_binary, "#!/bin/sh\necho 'senixd 0.3.1'\n").unwrap();
    fs::set_permissions(&fake_binary, fs::Permissions::from_mode(0o755)).unwrap();
    let archive = directory.path().join(&asset);
    let packaged = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .arg("-C")
        .arg(directory.path())
        .arg(&package)
        .status()
        .unwrap();
    assert!(packaged.success());
    let archive_bytes = fs::read(&archive).unwrap();
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(&archive_bytes) {
        write!(&mut digest, "{byte:02x}").unwrap();
    }
    let checksum = format!("{digest}  {asset}\n");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed_attempts = Arc::clone(&attempts);
    let asset_path = format!("/releases/download/{version}/{asset}");
    let server = thread::spawn(move || {
        for stream in listener.incoming().take(8) {
            let mut stream = stream.unwrap();
            let request = read_headers(&mut stream);
            let path = request.split_ascii_whitespace().nth(1).unwrap_or_default();
            if path == asset_path {
                if observed_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    continue;
                }
                write_test_http_response(&mut stream, &archive_bytes);
            } else if path == format!("/releases/download/{version}/checksums.txt") {
                write_test_http_response(&mut stream, checksum.as_bytes());
                return;
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        }
        panic!("installer did not complete its mirror requests");
    });

    let install_directory = directory.path().join("installed");
    fs::create_dir(&install_directory).unwrap();
    let installer_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
    let output = Command::new("sh")
        .arg(installer_path)
        .args(["--version", version, "--install-dir"])
        .arg(&install_directory)
        .env(
            "SENIX_RELEASE_BASE_URL",
            format!("http://{address}/releases"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "installer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let version_output = Command::new(install_directory.join("senixd"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(version_output.stdout, b"senixd 0.3.1\n");
}

#[cfg(unix)]
#[test]
fn configured_shutdown_budget_bounds_sigterm_exit() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);

    let child = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "--listen",
            &proxy.to_string(),
            "--admin-listen",
            &admin.to_string(),
            "--db",
            db.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--shutdown-grace-seconds",
            "0",
            "--shutdown-timeout-seconds",
            "1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut senix = ChildGuard(child);
    wait_until_ready(admin, proxy);

    let signal = Command::new("kill")
        .args(["-TERM", &senix.0.id().to_string()])
        .status()
        .unwrap();
    assert!(signal.success());
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = senix.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "senixd ignored its configured SIGTERM shutdown budget"
        );
        thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn proxy_reaches_an_explicit_https_upstream() {
    let backend = spawn_tls_backend("TLS_BACKEND");
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_https_backend_config(&config, backend);
    let owner_key = bootstrap_owner_key(&db);

    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    wait_for_health_state(admin, "instance-a", "HEALTHY", &owner_key);

    let response = raw_http(
        proxy,
        "GET / HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(response.split("\r\n\r\n").nth(1), Some("TLS_BACKEND"));
    drop(senix);
}

#[test]
fn proxy_reaches_an_http2_only_https_upstream() {
    let backend = spawn_h2_tls_backend();
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_h2_https_backend_config(&config, backend);
    let owner_key = bootstrap_owner_key(&db);

    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    wait_for_health_state(admin, "instance-a", "HEALTHY", &owner_key);

    let response = raw_http(
        proxy,
        "GET / HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("H2_BACKEND"), "{response}");
    drop(senix);
}

#[test]
fn most_specific_path_prefix_wins_regardless_of_config_order() {
    let root_backend = spawn_backend("ROOT", Duration::ZERO);
    let api_backend = spawn_backend("API", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_overlapping_route_config(&config, root_backend, api_backend);

    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    assert_eq!(proxy_get(proxy, "/other"), "ROOT");
    assert_eq!(proxy_get(proxy, "/api/users"), "API");
    assert_eq!(proxy_get(proxy, "/apix"), "ROOT");
    drop(senix);
}

#[test]
fn exact_host_wins_over_a_single_label_wildcard() {
    let wildcard_backend = spawn_backend("WILDCARD", Duration::ZERO);
    let exact_backend = spawn_backend("EXACT", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_wildcard_route_config(&config, wildcard_backend, exact_backend);

    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    assert_eq!(proxy_get_host(proxy, "foo.example.test", "/"), "WILDCARD");
    assert_eq!(proxy_get_host(proxy, "api.example.test", "/"), "EXACT");
    let bare = raw_http(
        proxy,
        "GET / HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n",
    );
    assert!(bare.starts_with("HTTP/1.1 404"), "{bare}");
    drop(senix);
}

#[test]
fn bootstrap_owner_key_protects_every_management_route() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);

    let api_key = bootstrap_owner_key(&db);
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let (status, denied) = admin_response(admin, "GET", "/api/v1/instances/instance-a", None, None);
    assert_eq!(status, 401);
    assert_eq!(denied["code"], "AUTHENTICATION_REQUIRED");

    let (status, instance) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/instances/instance-a",
        None,
        None,
        Some(&api_key),
    );
    assert_eq!(status, 200);
    assert_eq!(instance["id"], "instance-a");
    drop(senix);
}

#[test]
fn prometheus_metrics_are_scoped_and_cover_the_real_proxy_path() {
    let backend = spawn_backend("METRICS", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);

    bootstrap_owner_key(&db);
    bootstrap_owner_account(&db, "admin", "correct horse battery staple");
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    let cookie = login_owner_cookie(admin, "correct horse battery staple");
    let metrics_key =
        issue_global_api_key_with_cookie(admin, &cookie, "prometheus", &["metrics.read"]);

    let denied = raw_http(
        admin,
        "GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");

    for _ in 0..2 {
        let response = raw_http(
            proxy,
            "GET / HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n",
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }

    let metrics = raw_http(
        admin,
        &format!(
            "GET /metrics HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {metrics_key}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(metrics.starts_with("HTTP/1.1 200"), "{metrics}");
    assert_eq!(
        response_header(&metrics, "content-type"),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    assert_eq!(response_header(&metrics, "cache-control"), Some("no-store"));
    let body = metrics.split("\r\n\r\n").nth(1).unwrap();
    assert!(body.contains("senix_proxy_requests_total 2\n"), "{body}");
    assert!(
        body.contains("senix_proxy_responses_total{class=\"2xx\"} 2\n"),
        "{body}"
    );
    assert!(body.contains("senix_proxy_errors_total 0\n"), "{body}");
    assert_request_duration_histogram(body, 2);
    assert!(body.contains("senix_config_snapshot_version 1\n"), "{body}");
    assert!(
        body.contains("senix_certificate_store_enabled 0\n"),
        "{body}"
    );
    assert!(body.contains("senix_certificates_active 0\n"), "{body}");
    assert!(body.contains("senix_certificates_expired 0\n"), "{body}");
    assert!(
        body.contains("senix_instance_in_flight{instance_id=\"instance-a\",kind=\"ordinary\"} 0\n"),
        "{body}"
    );
    assert!(!body.contains(&metrics_key));
    let missing = raw_http(
        proxy,
        "GET / HTTP/1.1\r\nHost: missing.test\r\nConnection: close\r\n\r\n",
    );
    assert!(missing.starts_with("HTTP/1.1 404"));
    let after_error = raw_http(
        admin,
        &format!(
            "GET /metrics HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {metrics_key}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_request_duration_histogram(&after_error, 3);
    assert!(after_error.contains("senix_proxy_responses_total{class=\"4xx\"} 1\n"));
    drop(senix);
}

fn assert_request_duration_histogram(body: &str, expected: u64) {
    let buckets = body
        .lines()
        .filter(|line| {
            line.starts_with("senix_proxy_request_duration_seconds_bucket{kind=\"ordinary\",")
        })
        .map(|line| {
            line.split_whitespace()
                .last()
                .unwrap()
                .parse::<u64>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(!buckets.is_empty(), "{body}");
    assert!(buckets.windows(2).all(|window| window[0] <= window[1]));
    assert_eq!(buckets.last(), Some(&expected));
    assert!(body.contains(&format!(
        "senix_proxy_request_duration_seconds_count{{kind=\"ordinary\"}} {expected}\n"
    )));
    let sum = body
        .lines()
        .find_map(|line| {
            line.strip_prefix("senix_proxy_request_duration_seconds_sum{kind=\"ordinary\"} ")
        })
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!(sum > 0.0 && sum.is_finite());
}

#[test]
fn json_access_logs_are_parseable_without_request_secrets() {
    let backend = spawn_backend("LOGS", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    let log = dir.path().join("output.jsonl");
    write_single_backend_config(&config, backend);
    let senix = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_senixd"))
            .args([
                "--log-format",
                "json",
                "--listen",
                &proxy.to_string(),
                "--admin-listen",
                &admin.to_string(),
                "--db",
                db.to_str().unwrap(),
                "--config",
                config.to_str().unwrap(),
            ])
            .env("RUST_LOG", "senixd=info,senix_pingora::access=debug")
            .stdout(fs::File::create(&log).unwrap())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_until_ready(admin, proxy);
    let response = raw_http(
        proxy,
        "GET /private-path?token=query-secret HTTP/1.1\r\nHost: example.test\r\nAuthorization: Bearer header-secret\r\nCookie: session=cookie-secret\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 200"));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = fs::read_to_string(&log).unwrap();
        let records = output
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        if let Some(access) = records
            .iter()
            .find(|record| record["fields"]["message"] == "proxy_request_completed")
        {
            assert_eq!(access["fields"]["status"], 200);
            assert_eq!(access["fields"]["instance_id"], "instance-a");
            assert_eq!(access["fields"]["long_lived"], false);
            assert!(access["fields"]["duration_seconds"].as_f64().unwrap() > 0.0);
            for secret in [
                "private-path",
                "query-secret",
                "header-secret",
                "cookie-secret",
            ] {
                assert!(!output.contains(secret), "request secret in log: {secret}");
            }
            break;
        }
        assert!(Instant::now() < deadline, "access log was not emitted");
        thread::sleep(Duration::from_millis(20));
    }
    drop(senix);
}

#[test]
fn owner_can_login_to_embedded_admin_and_manage_keys_with_csrf_protection() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);

    let bootstrap_key = bootstrap_owner_key(&db);
    bootstrap_owner_account(&db, "admin", "correct horse battery staple");
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let (status, revoked_bootstrap) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/instances/instance-a",
        None,
        None,
        Some(&bootstrap_key),
    );
    assert_eq!(status, 401);
    assert_eq!(revoked_bootstrap["code"], "CREDENTIAL_REVOKED");

    assert_embedded_admin_page(admin);
    assert_owner_login_denied(admin, "wrong password");
    let cookie = login_owner_cookie(admin, "correct horse battery staple");

    let (status, session, _) = admin_response_with_headers(
        admin,
        "GET",
        "/api/v1/auth/session",
        None,
        &[("Cookie", &cookie)],
    );
    assert_eq!(status, 200);
    assert_eq!(session["username"], "admin");

    let issue_body = json!({
        "label": "dashboard-readonly",
        "actions": ["instance.read"],
        "instance_ids": ["instance-a"],
        "all_resources": false
    });
    let (status, denied, _) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/credentials",
        Some(issue_body.clone()),
        &[("Cookie", &cookie)],
    );
    assert_eq!(status, 403);
    assert_eq!(denied["code"], "CSRF_REQUIRED");

    let (status, issued, _) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/credentials",
        Some(issue_body),
        &[("Cookie", &cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 201);
    assert!(issued["api_key"].as_str().unwrap().starts_with("snx_"));

    let (status, audit, _) = admin_response_with_headers(
        admin,
        "GET",
        "/api/v1/audit-events",
        None,
        &[("Cookie", &cookie)],
    );
    assert_eq!(status, 200);
    assert!(audit.to_string().contains("owner.login"));
    assert!(audit.to_string().contains("credential.issue"));

    let (status, _, logout_response) = admin_response_with_headers(
        admin,
        "DELETE",
        "/api/v1/auth/session",
        None,
        &[("Cookie", &cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 204);
    assert!(
        response_header(&logout_response, "set-cookie")
            .unwrap()
            .contains("Max-Age=0")
    );
    let (status, expired_session, _) = admin_response_with_headers(
        admin,
        "GET",
        "/api/v1/auth/session",
        None,
        &[("Cookie", &cookie)],
    );
    assert_eq!(status, 401);
    assert_eq!(expired_session["code"], "INVALID_OWNER_SESSION");

    reset_owner_password(&db, "new correct horse battery staple");
    assert_owner_login_denied(admin, "correct horse battery staple");
    let reset_cookie = login_owner_cookie(admin, "new correct horse battery staple");
    assert!(reset_cookie.starts_with("senix_session=snxs_"));
    drop(senix);
}

#[test]
fn repeated_owner_login_failures_are_rate_limited_by_peer_address() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);

    bootstrap_owner_key(&db);
    bootstrap_owner_account(&db, "admin", "correct horse battery staple");
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    for _ in 0..5 {
        assert_owner_login_denied(admin, "wrong password");
    }
    let (status, limited, response) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/auth/login",
        Some(json!({
            "username": "admin",
            "password": "correct horse battery staple"
        })),
        &[("X-Forwarded-For", "203.0.113.9")],
    );
    assert_eq!(status, 429);
    assert_eq!(limited["code"], "LOGIN_RATE_LIMITED");
    assert!(
        response_header(&response, "retry-after")
            .unwrap()
            .parse::<u64>()
            .is_ok_and(|seconds| seconds > 0)
    );
    assert_eq!(
        response_header(&response, "cache-control"),
        Some("no-store")
    );
    drop(senix);
}

#[test]
fn an_ai_key_can_apply_only_the_exact_change_plan_approved_by_the_owner() {
    let backend = spawn_backend("A", Duration::ZERO);
    let api_backend = spawn_backend("B", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_overlapping_route_config(&config, backend, api_backend);

    bootstrap_owner_key(&db);
    bootstrap_owner_account(&db, "admin", "correct horse battery staple");
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    let cookie = login_owner_cookie(admin, "correct horse battery staple");

    let apply_key = issue_global_api_key_with_cookie(
        admin,
        &cookie,
        "config-agent",
        &["change.read", "change.plan", "change.apply"],
    );
    assert_change_apply_tools(admin, &apply_key);
    let snapshot = call_mcp_tool(admin, &apply_key, "get_current_config", &json!({}));
    let snapshot = &snapshot["structuredContent"];
    assert_eq!(snapshot["version"], 1);
    let untouched_route = snapshot["config"]["routes"][0].clone();
    let mut candidate = snapshot["config"].clone();
    candidate["routes"][1]["host"] = json!("approved.test");
    let result = call_mcp_tool(
        admin,
        &apply_key,
        "plan_change",
        &json!({"expected_version": 1, "candidate": candidate}),
    );
    let planned = &result["structuredContent"];
    assert_eq!(planned["status"], "PLANNED");
    assert_eq!(planned["created_by"]["label"], "config-agent");
    assert_eq!(planned["candidate_digest"].as_str().unwrap().len(), 64);
    assert_eq!(planned["diff"]["changed_routes"], json!(["route-api"]));
    assert_eq!(planned["diff"]["removed_routes"], json!([]));
    assert_eq!(proxy_get(proxy, "/api/before-approval"), "B");
    let change_id = planned["change_id"].as_str().unwrap();

    let missing_approval = call_mcp_tool(
        admin,
        &apply_key,
        "apply_approved_change",
        &json!({"change_id": change_id}),
    );
    assert_eq!(missing_approval["isError"], true);
    assert_eq!(
        missing_approval["structuredContent"]["code"],
        "CHANGE_APPROVAL_REQUIRED"
    );

    let (status, approved, _) = admin_response_with_headers(
        admin,
        "POST",
        &format!("/api/v1/changes/{change_id}/approve"),
        None,
        &[("Cookie", &cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 200);
    assert_eq!(approved["status"], "APPROVED");

    let (status, denied_approval) = admin_response_with_bearer(
        admin,
        "POST",
        &format!("/api/v1/changes/{change_id}/approve"),
        None,
        None,
        Some(&apply_key),
    );
    assert_eq!(status, 403);
    assert_eq!(denied_approval["evidence"]["action"], "change.approve");

    let applied = call_mcp_tool(
        admin,
        &apply_key,
        "apply_approved_change",
        &json!({"change_id": change_id}),
    );
    assert_eq!(applied["structuredContent"]["version"], 2);
    let replayed = call_mcp_tool(
        admin,
        &apply_key,
        "apply_approved_change",
        &json!({"change_id": change_id}),
    );
    assert_eq!(replayed, applied);
    let (status, rest_current) =
        admin_response_with_bearer(admin, "GET", "/api/v1/config", None, None, Some(&apply_key));
    assert_eq!(status, 200);
    let result = call_mcp_tool(admin, &apply_key, "get_current_config", &json!({}));
    let current = &result["structuredContent"];
    assert_eq!(*current, rest_current);
    assert_eq!(current["version"], 2);
    assert_eq!(current["config"]["routes"][0], untouched_route);
    assert_eq!(current["config"]["routes"][1]["host"], "approved.test");
    assert_eq!(
        proxy_get_host(proxy, "approved.test", "/api/after-approval"),
        "B"
    );
    assert_eq!(proxy_get(proxy, "/unrelated-route"), "A");
    assert_eq!(proxy_get(proxy, "/api/old-route"), "A");
    drop(senix);
}

#[test]
fn mcp_config_reads_enforce_global_scope_read_only_permissions_and_revocation() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);
    let owner_key = bootstrap_owner_key(&db);
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let (status, issued) = admin_response_with_bearer(
        admin,
        "POST",
        "/api/v1/credentials",
        None,
        Some(json!({"label": "config-reader", "actions": ["change.read"], "all_resources": true})),
        Some(&owner_key),
    );
    assert_eq!(status, 201);
    let reader = issued["api_key"].as_str().unwrap();
    let result = call_mcp_tool(admin, reader, "get_current_config", &json!({}));
    assert_eq!(result["structuredContent"]["version"], 1);
    let candidate = result["structuredContent"]["config"].clone();

    let (_, catalog) = mcp_request(admin, Some(reader), 1, "tools/list", &json!({}));
    let names = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"get_current_config"));
    assert!(!names.contains(&"plan_change"));
    assert!(!names.contains(&"apply_approved_change"));
    for (name, arguments) in [
        (
            "plan_change",
            json!({"expected_version": 1, "candidate": candidate}),
        ),
        (
            "apply_approved_change",
            json!({"change_id": uuid::Uuid::new_v4()}),
        ),
    ] {
        let denied = call_mcp_tool(admin, reader, name, &arguments);
        assert_eq!(denied["isError"], true);
        assert_eq!(denied["structuredContent"]["code"], "FORBIDDEN");
    }

    let (_, scoped_key) = issue_api_key(
        admin,
        &owner_key,
        "instance-reader",
        &["instance.read"],
        &["instance-a"],
    );
    let denied = call_mcp_tool(admin, &scoped_key, "get_current_config", &json!({}));
    assert_eq!(denied["isError"], true);
    assert_eq!(denied["structuredContent"]["code"], "FORBIDDEN");
    let (status, _) = admin_response_with_bearer(
        admin,
        "DELETE",
        &format!(
            "/api/v1/credentials/{}",
            issued["credential_id"].as_str().unwrap()
        ),
        None,
        None,
        Some(&owner_key),
    );
    assert_eq!(status, 204);
    let (status, _) = mcp_request(
        admin,
        Some(reader),
        2,
        "tools/call",
        &json!({"name": "get_current_config", "arguments": {}}),
    );
    assert_eq!(status, 401);
    drop(senix);
}

#[test]
fn mcp_rejects_a_candidate_from_an_old_read_and_preserves_concurrent_routes_on_retry() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);
    bootstrap_owner_key(&db);
    bootstrap_owner_account(&db, "admin", "correct horse battery staple");
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    let cookie = login_owner_cookie(admin, "correct horse battery staple");
    let key = issue_global_api_key_with_cookie(
        admin,
        &cookie,
        "config-agent",
        &["change.read", "change.plan", "change.apply"],
    );
    let read = call_mcp_tool(admin, &key, "get_current_config", &json!({}));
    let mut candidate = read["structuredContent"]["config"].clone();
    candidate["routes"][0]["host"] = json!("ai.test");
    assert_config_plan_contract(admin, &key, &candidate);

    // REST v1 still accepts a raw GatewayConfig, with no new version envelope.
    let mut owner_config = read["structuredContent"]["config"].clone();
    let mut added_route = owner_config["routes"][0].clone();
    added_route["id"] = json!("owner-added-route");
    added_route["host"] = json!("owner.test");
    owner_config["routes"]
        .as_array_mut()
        .unwrap()
        .push(added_route.clone());
    assert_eq!(
        publish_rest_config_as_owner(admin, &cookie, owner_config.clone()),
        2
    );
    assert_eq!(proxy_get_host(proxy, "owner.test", "/"), "A");

    let before = call_mcp_tool(admin, &key, "list_changes", &json!({}));
    let denied = call_mcp_tool(
        admin,
        &key,
        "plan_change",
        &json!({"expected_version": read["structuredContent"]["version"], "candidate": candidate}),
    );
    assert_eq!(denied["isError"], true);
    assert_eq!(
        denied["structuredContent"]["code"],
        "CONFIG_VERSION_MISMATCH"
    );
    assert_eq!(
        denied["structuredContent"]["evidence"],
        json!({"expected_version": 1, "current_version": 2})
    );
    assert_eq!(
        call_mcp_tool(admin, &key, "list_changes", &json!({})),
        before
    );
    let current = call_mcp_tool(admin, &key, "get_current_config", &json!({}));
    assert_eq!(current["structuredContent"]["config"], owner_config);
    let mut merged = current["structuredContent"]["config"].clone();
    merged["routes"][0]["host"] = json!("ai.test");
    let plan = call_mcp_tool(
        admin,
        &key,
        "plan_change",
        &json!({"expected_version": current["structuredContent"]["version"], "candidate": merged}),
    );
    assert_eq!(plan["structuredContent"]["base_version"], 2);
    assert_eq!(
        plan["structuredContent"]["diff"]["removed_routes"],
        json!([])
    );
    let id = &plan["structuredContent"]["change_id"];
    let unapproved = call_mcp_tool(
        admin,
        &key,
        "apply_approved_change",
        &json!({"change_id": id}),
    );
    assert_eq!(
        unapproved["structuredContent"]["code"],
        "CHANGE_APPROVAL_REQUIRED"
    );
    approve_config_as_owner(admin, &cookie, id.as_str().unwrap());
    let applied = call_mcp_tool(
        admin,
        &key,
        "apply_approved_change",
        &json!({"change_id": id}),
    );
    assert_eq!(applied["structuredContent"]["version"], 3);
    let current = call_mcp_tool(admin, &key, "get_current_config", &json!({}));
    assert_eq!(current["structuredContent"]["config"], merged);
    assert_eq!(
        current["structuredContent"]["config"]["routes"][1],
        added_route
    );
    assert_eq!(proxy_get_host(proxy, "ai.test", "/"), "A");
    assert_eq!(proxy_get_host(proxy, "owner.test", "/"), "A");
    drop(senix);
}

#[test]
fn mcp_rejects_an_approved_config_plan_after_another_snapshot_is_published() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);
    bootstrap_owner_key(&db);
    bootstrap_owner_account(&db, "admin", "correct horse battery staple");
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    let cookie = login_owner_cookie(admin, "correct horse battery staple");
    let key = issue_global_api_key_with_cookie(
        admin,
        &cookie,
        "config-agent",
        &["change.read", "change.plan", "change.apply"],
    );
    let snapshot = call_mcp_tool(admin, &key, "get_current_config", &json!({}));
    let mut candidate = snapshot["structuredContent"]["config"].clone();
    candidate["routes"][0]["host"] = json!("first.test");
    let first = call_mcp_tool(
        admin,
        &key,
        "plan_change",
        &json!({"expected_version": 1, "candidate": candidate}),
    );
    candidate["routes"][0]["host"] = json!("stale.test");
    let second = call_mcp_tool(
        admin,
        &key,
        "plan_change",
        &json!({"expected_version": 1, "candidate": candidate}),
    );
    for plan in [&first, &second] {
        assert_eq!(plan["structuredContent"]["base_version"], 1);
        let id = plan["structuredContent"]["change_id"].as_str().unwrap();
        let (status, _, _) = admin_response_with_headers(
            admin,
            "POST",
            &format!("/api/v1/changes/{id}/approve"),
            None,
            &[("Cookie", &cookie), ("X-Senix-CSRF", "1")],
        );
        assert_eq!(status, 200);
    }
    let applied = call_mcp_tool(
        admin,
        &key,
        "apply_approved_change",
        &json!({"change_id": first["structuredContent"]["change_id"]}),
    );
    assert_eq!(applied["structuredContent"]["version"], 2);
    let denied = call_mcp_tool(
        admin,
        &key,
        "apply_approved_change",
        &json!({"change_id": second["structuredContent"]["change_id"]}),
    );
    assert_eq!(denied["isError"], true);
    assert_eq!(denied["structuredContent"]["code"], "STALE_PLAN");
    let current = call_mcp_tool(admin, &key, "get_current_config", &json!({}));
    assert_eq!(current["structuredContent"]["version"], 2);
    assert_eq!(
        current["structuredContent"]["config"]["routes"][0]["host"],
        "first.test"
    );
    assert_eq!(proxy_get_host(proxy, "first.test", "/"), "A");
    drop(senix);
}

#[test]
fn restricted_key_is_scoped_revocable_and_audited_without_secret_leakage() {
    let backend_a = spawn_backend("A", Duration::ZERO);
    let backend_b = spawn_backend("B", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_config(&config, backend_a, backend_b);

    let owner_key = bootstrap_owner_key(&db);
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let (credential_id, deploy_key) = issue_api_key(
        admin,
        &owner_key,
        "deploy-instance-a",
        &["instance.read", "instance.drain"],
        &["instance-a"],
    );

    let (status, instance) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/instances/instance-a",
        None,
        None,
        Some(&deploy_key),
    );
    assert_eq!(status, 200);
    assert_eq!(instance["id"], "instance-a");

    let (status, denied_instance) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/instances/instance-b",
        None,
        None,
        Some(&deploy_key),
    );
    assert_eq!(status, 403);
    assert_eq!(denied_instance["code"], "FORBIDDEN");

    let (status, drained) = admin_response_with_bearer(
        admin,
        "POST",
        "/api/v1/instances/instance-a/drain",
        Some("restricted-drain-a"),
        Some(json!({})),
        Some(&deploy_key),
    );
    assert_eq!(status, 202);
    assert_eq!(drained["status"], "DRAINED");

    let (status, denied_action) = admin_response_with_bearer(
        admin,
        "POST",
        "/api/v1/instances/instance-a/rejoin",
        Some("restricted-rejoin-a"),
        Some(json!({"generation": 2, "weight": 100})),
        Some(&deploy_key),
    );
    assert_eq!(status, 403);
    assert_eq!(denied_action["evidence"]["action"], "instance.rejoin");

    assert_key_not_listed(admin, &owner_key, &deploy_key);

    let (status, _) = admin_response_with_bearer(
        admin,
        "DELETE",
        &format!("/api/v1/credentials/{credential_id}"),
        None,
        None,
        Some(&owner_key),
    );
    assert_eq!(status, 204);

    let (status, revoked) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/instances/instance-a",
        None,
        None,
        Some(&deploy_key),
    );
    assert_eq!(status, 401);
    assert_eq!(revoked["code"], "CREDENTIAL_REVOKED");

    let (status, audit) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/audit-events",
        None,
        None,
        Some(&owner_key),
    );
    assert_eq!(status, 200);
    let serialized = audit.to_string();
    assert!(!serialized.contains(&owner_key));
    assert!(!serialized.contains(&deploy_key));
    assert!(serialized.contains("credential.issue"));
    assert!(serialized.contains("instance.drain"));
    assert!(serialized.contains("DENIED"));
    assert!(serialized.contains("credential.revoke"));
    drop(senix);
}

#[test]
fn stateless_mcp_reuses_the_same_key_scope_and_traffic_controller() {
    let backend_a = spawn_backend("A", Duration::ZERO);
    let backend_b = spawn_backend("B", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_config(&config, backend_a, backend_b);

    let owner_key = bootstrap_owner_key(&db);
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    let (_, mcp_key) = issue_api_key(
        admin,
        &owner_key,
        "mcp-instance-a",
        &["instance.read", "instance.drain"],
        &["instance-a"],
    );

    let (status, unauthenticated) = mcp_request(
        admin,
        None,
        1,
        "initialize",
        &json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "senix-e2e", "version": "1.0"}
        }),
    );
    assert_eq!(status, 401);
    assert_eq!(unauthenticated["code"], "AUTHENTICATION_REQUIRED");

    let (status, initialized) = mcp_request(
        admin,
        Some(&mcp_key),
        2,
        "initialize",
        &json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "senix-e2e", "version": "1.0"}
        }),
    );
    assert_eq!(status, 200);
    assert_eq!(initialized["result"]["serverInfo"]["name"], "senix");

    let (_, tools) = mcp_request(admin, Some(&mcp_key), 3, "tools/list", &json!({}));
    let tool_names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"get_instance_health"));
    assert!(tool_names.contains(&"drain_instance"));
    assert!(!tool_names.contains(&"rejoin_instance"));
    assert!(!tool_names.contains(&"diagnose_request"));
    assert!(!tool_names.contains(&"get_current_config"));
    assert!(!tool_names.contains(&"create_api_key"));
    assert_modern_tool_catalog_is_private(admin, &mcp_key);

    let (_, instance_a) = mcp_request(
        admin,
        Some(&mcp_key),
        4,
        "tools/call",
        &json!({"name": "get_instance_health", "arguments": {"instance_id": "instance-a"}}),
    );
    assert_eq!(
        instance_a["result"]["structuredContent"]["id"],
        "instance-a"
    );

    let (_, instance_b) = mcp_request(
        admin,
        Some(&mcp_key),
        5,
        "tools/call",
        &json!({"name": "get_instance_health", "arguments": {"instance_id": "instance-b"}}),
    );
    assert_eq!(instance_b["result"]["isError"], true);
    assert_eq!(
        instance_b["result"]["structuredContent"]["code"],
        "FORBIDDEN"
    );

    let (_, drained) = mcp_request(
        admin,
        Some(&mcp_key),
        6,
        "tools/call",
        &json!({
            "name": "drain_instance",
            "arguments": {
                "instance_id": "instance-a",
                "timeout_ms": 1000,
                "force": false,
                "idempotency_key": "mcp-drain-a"
            }
        }),
    );
    assert_eq!(drained["result"]["structuredContent"]["status"], "DRAINED");
    assert_eq!(proxy_get(proxy, "/after-mcp-drain"), "B");
    drop(senix);
}

#[test]
fn pingora_drains_one_backend_and_restores_that_state_after_restart() {
    let backend_a = spawn_backend("A", Duration::from_millis(600));
    let backend_b = spawn_backend("B", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_config(&config, backend_a, backend_b);
    let owner_key = bootstrap_owner_key(&db);

    let mut senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let slow = thread::spawn(move || proxy_get(proxy, "/slow"));
    thread::sleep(Duration::from_millis(120));

    let drained = admin_json_with_bearer(
        admin,
        "POST",
        "/api/v1/instances/instance-a/drain",
        Some("deploy-42"),
        Some(json!({"timeout_ms": 1_000})),
        &owner_key,
    );
    assert_eq!(drained["status"], "DRAINING");
    assert_eq!(drained["ordinary_in_flight"], 1);
    let operation_id = drained["operation_id"].as_str().unwrap().to_owned();

    for _ in 0..6 {
        assert_eq!(proxy_get(proxy, "/new"), "B");
    }
    assert_eq!(slow.join().unwrap(), "A");
    wait_for_operation_state(admin, &operation_id, "DRAINED", &owner_key);

    senix.0.kill().unwrap();
    senix.0.wait().unwrap();

    let proxy_after_restart = free_address();
    let admin_after_restart = free_address();
    senix = spawn_senix(proxy_after_restart, admin_after_restart, &db, &config);
    wait_until_ready(admin_after_restart, proxy_after_restart);
    let restored_operation = admin_json_with_bearer(
        admin_after_restart,
        "GET",
        &format!("/api/v1/operations/{operation_id}"),
        None,
        None,
        &owner_key,
    );
    assert_eq!(restored_operation["status"], "DRAINED");
    assert_eq!(restored_operation["ordinary_in_flight"], 0);
    for _ in 0..4 {
        assert_eq!(proxy_get(proxy_after_restart, "/after-restart"), "B");
    }

    let rejoined = admin_json_with_bearer(
        admin_after_restart,
        "POST",
        "/api/v1/instances/instance-a/rejoin",
        Some("rejoin-42"),
        Some(json!({"generation": 2, "weight": 100})),
        &owner_key,
    );
    assert_eq!(rejoined["traffic"], "SERVING");
    assert_eq!(rejoined["generation"], 2);

    let mut bodies = Vec::new();
    for _ in 0..4 {
        bodies.push(proxy_get(proxy_after_restart, "/rejoined"));
    }
    bodies.sort();
    assert_eq!(bodies, ["A", "A", "B", "B"]);
    drop(senix);
}

#[test]
fn drain_identifies_a_grpc_stream_as_long_lived() {
    let backend_a = spawn_backend("A", Duration::from_millis(600));
    let backend_b = spawn_backend("B", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_config(&config, backend_a, backend_b);
    let owner_key = bootstrap_owner_key(&db);

    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let stream = thread::spawn(move || {
        raw_http(
            proxy,
            "GET /slow HTTP/1.1\r\nHost: example.test\r\nContent-Type: application/grpc\r\nConnection: close\r\n\r\n",
        )
    });
    thread::sleep(Duration::from_millis(120));

    let operation = admin_json_with_bearer(
        admin,
        "POST",
        "/api/v1/instances/instance-a/drain",
        Some("drain-grpc-stream"),
        Some(json!({"timeout_ms": 1_000})),
        &owner_key,
    );
    assert_eq!(operation["status"], "DRAINING");
    assert_eq!(operation["ordinary_in_flight"], 0);
    assert_eq!(operation["long_lived_in_flight"], 1);
    let before_completion = raw_http(
        admin,
        &format!(
            "GET /metrics HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {owner_key}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(
        before_completion
            .contains("senix_proxy_request_duration_seconds_count{kind=\"long_lived\"} 0\n")
    );

    assert!(stream.join().unwrap().ends_with("\r\n\r\nA"));
    wait_for_operation_state(
        admin,
        operation["operation_id"].as_str().unwrap(),
        "DRAINED",
        &owner_key,
    );
    let after_completion = raw_http(
        admin,
        &format!(
            "GET /metrics HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {owner_key}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(
        after_completion
            .contains("senix_proxy_request_duration_seconds_count{kind=\"long_lived\"} 1\n")
    );
    assert!(
        after_completion
            .contains("senix_proxy_request_duration_seconds_count{kind=\"ordinary\"} 0\n")
    );
    drop(senix);
}

#[test]
fn active_http_health_checks_remove_and_restore_a_backend() {
    let (backend_a, backend_a_healthy) = spawn_controllable_backend("A");
    let backend_b = spawn_backend("B", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_health_config(&config, backend_a, backend_b);
    let owner_key = bootstrap_owner_key(&db);

    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);
    wait_for_health_state(admin, "instance-a", "HEALTHY", &owner_key);
    wait_for_health_state(admin, "instance-b", "HEALTHY", &owner_key);

    backend_a_healthy.store(false, Ordering::SeqCst);
    wait_for_health_state(admin, "instance-a", "UNHEALTHY", &owner_key);
    for _ in 0..6 {
        assert_eq!(proxy_get(proxy, "/while-unhealthy"), "B");
    }

    backend_a_healthy.store(true, Ordering::SeqCst);
    wait_for_health_state(admin, "instance-a", "HEALTHY", &owner_key);
    let mut bodies = Vec::new();
    for _ in 0..4 {
        bodies.push(proxy_get(proxy, "/after-recovery"));
    }
    bodies.sort();
    assert_eq!(bodies, ["A", "A", "B", "B"]);
    drop(senix);
}

#[test]
fn drain_api_requires_force_for_the_last_backend_and_returns_structured_evidence() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    write_single_backend_config(&config, backend);
    let owner_key = bootstrap_owner_key(&db);
    let senix = spawn_senix(proxy, admin, &db, &config);
    wait_until_ready(admin, proxy);

    let (status, blocked) = admin_response_with_bearer(
        admin,
        "POST",
        "/api/v1/instances/instance-a/drain",
        Some("single-backend-drain"),
        Some(json!({})),
        Some(&owner_key),
    );
    assert_eq!(status, 409);
    assert_eq!(blocked["code"], "LAST_AVAILABLE_BACKEND");
    assert_eq!(blocked["evidence"]["instance_id"], "instance-a");
    assert_eq!(blocked["evidence"]["route_id"], "route-main");

    let (status, forced) = admin_response_with_bearer(
        admin,
        "POST",
        "/api/v1/instances/instance-a/drain",
        Some("single-backend-forced-drain"),
        Some(json!({"force": true})),
        Some(&owner_key),
    );
    assert_eq!(status, 202);
    assert_eq!(forced["status"], "DRAINED");
    drop(senix);
}

#[test]
fn pingora_terminates_tls_with_a_configured_certificate() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let tls = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    write_single_backend_config(&config, backend);
    write_test_certificate(&cert, &key);
    let owner_key = bootstrap_owner_key(&db);

    let senix = spawn_senix_with_tls(proxy, tls, admin, &db, &config, &cert, &key);
    wait_until_ready(admin, proxy);

    let response = tls_get(tls, "example.test", "/secure");
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("\r\n\r\nA"));
    let metrics = raw_http(
        admin,
        &format!(
            "GET /metrics HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {owner_key}\r\nConnection: close\r\n\r\n"
        ),
    );
    let body = metrics.split("\r\n\r\n").nth(1).unwrap();
    assert!(
        body.contains("senix_certificate_store_enabled 0\n"),
        "{body}"
    );
    assert!(body.contains("senix_certificates_active 1\n"), "{body}");
    assert!(
        body.contains("senix_certificates_expiring_within_30_days 1\n"),
        "{body}"
    );
    drop(senix);
}

#[test]
fn pingora_restores_an_encrypted_managed_certificate_after_restart() {
    let backend = spawn_backend("A", Duration::ZERO);
    let proxy = free_address();
    let tls = free_address();
    let admin = free_address();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let config = dir.path().join("gateway.json");
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    let secret_key_file = dir.path().join("secret.key");
    write_single_backend_config(&config, backend);
    write_test_certificate(&cert, &key);
    let encoded_secret_key = senix_core::SecretVault::generate_base64();
    fs::write(&secret_key_file, &encoded_secret_key).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&secret_key_file, fs::Permissions::from_mode(0o600)).unwrap();
    }

    let certificate_pem = fs::read(&cert).unwrap();
    let private_key_pem = fs::read(&key).unwrap();
    let prepared =
        senix_pingora::TlsCertificateRegistry::prepare_pem(&certificate_pem, &private_key_pem)
            .unwrap();
    {
        let store = Arc::new(senix_core::SqliteStateStore::open(&db).unwrap());
        let certificates = senix_core::CertificateController::new(
            store,
            senix_core::SecretVault::from_base64(&encoded_secret_key).unwrap(),
        );
        certificates
            .replace(senix_core::CertificateMaterial {
                domains: prepared.domains().to_vec(),
                certificate_chain_pem: Arc::from(certificate_pem),
                private_key_pem: senix_core::SecretBytes::new(private_key_pem),
                not_before_ms: prepared.not_before_ms(),
                not_after_ms: prepared.not_after_ms(),
            })
            .unwrap();
    }
    let owner_key = bootstrap_owner_key(&db);
    let senix = spawn_senix_with_managed_tls(proxy, tls, admin, &db, &config, &secret_key_file);
    wait_until_ready(admin, proxy);

    let response = tls_get(tls, "example.test", "/managed");
    assert!(response.starts_with("HTTP/1.1 200"));
    let certificates =
        admin_json_with_bearer(admin, "GET", "/api/v1/certificates", None, None, &owner_key);
    assert_eq!(certificates.as_array().unwrap().len(), 1);
    assert_eq!(certificates[0]["domains"], json!(["example.test"]));
    assert!(certificates[0].get("private_key_pem").is_none());

    let (status, issued) = admin_response_with_bearer(
        admin,
        "POST",
        "/api/v1/credentials",
        None,
        Some(json!({
            "label": "mcp-certificate-reader",
            "actions": ["certificate.read"],
            "instance_ids": [],
            "all_resources": true
        })),
        Some(&owner_key),
    );
    assert_eq!(status, 201);
    let mcp_key = issued["api_key"].as_str().unwrap();
    let (_, tools) = mcp_request(admin, Some(mcp_key), 81, "tools/list", &json!({}));
    let tool_names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"list_certificates"));
    assert!(!tool_names.contains(&"issue_certificate"));
    let (_, listed) = mcp_request(
        admin,
        Some(mcp_key),
        82,
        "tools/call",
        &json!({"name": "list_certificates", "arguments": {}}),
    );
    let structured = &listed["result"]["structuredContent"];
    assert_eq!(structured.as_array().unwrap().len(), 1);
    assert!(structured.to_string().contains("example.test"));
    assert!(!structured.to_string().contains("PRIVATE KEY"));
    drop(senix);
}

#[test]
fn secret_key_generation_never_overwrites_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("secret.key");
    let first = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["secret-key", "generate", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(first.status.success());
    let original = fs::read_to_string(&output).unwrap();
    assert!(senix_core::SecretVault::from_base64(original.trim()).is_ok());

    let second = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["secret-key", "generate", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert_eq!(fs::read_to_string(&output).unwrap(), original);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn backup_cli_verifies_the_master_key_and_restores_without_overwriting() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("senix.db");
    let backup = directory.path().join("senix.backup.db");
    let restored = directory.path().join("restored.db");
    let secret_key = directory.path().join("secret.key");
    let wrong_key = directory.path().join("wrong.key");

    let encoded_key = senix_core::SecretVault::generate_base64();
    write_private_file(&secret_key, &encoded_key);
    write_private_file(&wrong_key, &senix_core::SecretVault::generate_base64());
    let store = Arc::new(senix_core::SqliteStateStore::open(&db).unwrap());
    let security = senix_core::SecurityController::new(Arc::clone(&store));
    let owner_key = security.bootstrap_owner_key("backup-owner").unwrap();
    let certificates = senix_core::CertificateController::new(
        Arc::clone(&store),
        senix_core::SecretVault::from_base64(&encoded_key).unwrap(),
    );
    certificates
        .save_secret("acme-account", b"encrypted-account-state")
        .unwrap();
    drop(certificates);
    drop(security);
    drop(store);

    let created = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["backup", "create", "--db"])
        .arg(&db)
        .arg("--output")
        .arg(&backup)
        .arg("--secret-key-file")
        .arg(&secret_key)
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "backup create failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(backup.is_file());
    let backup_before = fs::read(&backup).unwrap();

    let verified = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["backup", "verify", "--input"])
        .arg(&backup)
        .arg("--secret-key-file")
        .arg(&secret_key)
        .output()
        .unwrap();
    assert!(
        verified.status.success(),
        "backup verify failed: {}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let report: Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["master_key_verified"], true);

    let wrong = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["backup", "verify", "--input"])
        .arg(&backup)
        .arg("--secret-key-file")
        .arg(&wrong_key)
        .output()
        .unwrap();
    assert!(!wrong.status.success());

    let overwrite = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["backup", "create", "--db"])
        .arg(&db)
        .arg("--output")
        .arg(&backup)
        .arg("--secret-key-file")
        .arg(&secret_key)
        .output()
        .unwrap();
    assert!(!overwrite.status.success());
    assert_eq!(fs::read(&backup).unwrap(), backup_before);

    let restored_output = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args(["backup", "restore", "--input"])
        .arg(&backup)
        .arg("--db")
        .arg(&restored)
        .arg("--secret-key-file")
        .arg(&secret_key)
        .output()
        .unwrap();
    assert!(
        restored_output.status.success(),
        "backup restore failed: {}",
        String::from_utf8_lossy(&restored_output.stderr)
    );
    assert_restored_backup(&restored, &owner_key.api_key, &encoded_key);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&restored).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

fn assert_restored_backup(restored: &Path, owner_key: &str, encoded_key: &str) {
    let store = Arc::new(senix_core::SqliteStateStore::open(restored).unwrap());
    senix_core::SecurityController::new(Arc::clone(&store))
        .authenticate(owner_key)
        .unwrap();
    let certificates = senix_core::CertificateController::new(
        store,
        senix_core::SecretVault::from_base64(encoded_key).unwrap(),
    );
    assert_eq!(
        certificates
            .load_secret("acme-account")
            .unwrap()
            .unwrap()
            .expose(),
        b"encrypted-account-state"
    );
}

fn write_private_file(path: &Path, value: &str) {
    fs::write(path, format!("{value}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn spawn_senix(proxy: SocketAddr, admin: SocketAddr, db: &Path, config: &Path) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "--listen",
            &proxy.to_string(),
            "--admin-listen",
            &admin.to_string(),
            "--db",
            db.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    ChildGuard(child)
}

fn spawn_senix_with_tls(
    proxy: SocketAddr,
    tls: SocketAddr,
    admin: SocketAddr,
    db: &Path,
    config: &Path,
    cert: &Path,
    key: &Path,
) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "--listen",
            &proxy.to_string(),
            "--tls-listen",
            &tls.to_string(),
            "--tls-cert",
            cert.to_str().unwrap(),
            "--tls-key",
            key.to_str().unwrap(),
            "--admin-listen",
            &admin.to_string(),
            "--db",
            db.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    ChildGuard(child)
}

fn spawn_senix_with_managed_tls(
    proxy: SocketAddr,
    tls: SocketAddr,
    admin: SocketAddr,
    db: &Path,
    config: &Path,
    secret_key_file: &Path,
) -> ChildGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "--listen",
            &proxy.to_string(),
            "--tls-listen",
            &tls.to_string(),
            "--secret-key-file",
            secret_key_file.to_str().unwrap(),
            "--admin-listen",
            &admin.to_string(),
            "--db",
            db.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    ChildGuard(child)
}

fn write_test_certificate(cert: &Path, key: &Path) {
    let key_pair = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "example.test").unwrap();
    let name = name.build();
    let serial = BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap();
    let not_before = Asn1Time::days_from_now(0).unwrap();
    let not_after = Asn1Time::days_from_now(1).unwrap();
    let mut certificate = X509::builder().unwrap();
    certificate.set_version(2).unwrap();
    certificate.set_serial_number(&serial).unwrap();
    certificate.set_subject_name(&name).unwrap();
    certificate.set_issuer_name(&name).unwrap();
    certificate.set_pubkey(&key_pair).unwrap();
    certificate.set_not_before(&not_before).unwrap();
    certificate.set_not_after(&not_after).unwrap();
    certificate
        .sign(&key_pair, MessageDigest::sha256())
        .unwrap();
    fs::write(cert, certificate.build().to_pem().unwrap()).unwrap();
    fs::write(key, key_pair.private_key_to_pem_pkcs8().unwrap()).unwrap();
}

fn tls_get(address: SocketAddr, host: &str, path: &str) -> String {
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_verify(SslVerifyMode::NONE);
    let connector = builder.build();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(tcp) = TcpStream::connect_timeout(&address, Duration::from_millis(100))
            && let Ok(mut stream) = connector.connect(host, tcp)
        {
            stream
                .write_all(
                    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "TLS listener did not become ready"
        );
        thread::sleep(Duration::from_millis(40));
    }
}

fn bootstrap_owner_key(db: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "credential",
            "bootstrap",
            "--db",
            db.to_str().unwrap(),
            "--label",
            "e2e-owner",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "bootstrap failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let issued: Value = serde_json::from_slice(&output.stdout).unwrap();
    issued["api_key"].as_str().unwrap().to_owned()
}

fn assert_embedded_admin_page(admin: SocketAddr) {
    let response = raw_http(
        admin,
        "GET /admin/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("Senix control desk"));
    assert!(response_header(&response, "content-security-policy").is_some());

    let script = raw_http(
        admin,
        "GET /admin/admin.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(script.starts_with("HTTP/1.1 200"));
    assert!(script.contains("开始摘流"));
    assert!(script.contains("以新代次回接"));
    assert!(script.contains("调整权重"));
    assert!(script.contains("禁用实例"));
    assert!(script.contains("请求诊断"));
}

fn assert_owner_login_denied(admin: SocketAddr, password: &str) {
    let (status, denied, _) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/auth/login",
        Some(json!({"username": "admin", "password": password})),
        &[],
    );
    assert_eq!(status, 401);
    assert_eq!(denied["code"], "INVALID_OWNER_LOGIN");
}

fn reset_owner_password(db: &Path, password: &str) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "owner",
            "reset-password",
            "--db",
            db.to_str().unwrap(),
            "--password-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(password.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "owner password reset failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn login_owner_cookie(admin: SocketAddr, password: &str) -> String {
    let (status, login, response) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/auth/login",
        Some(json!({"username": "admin", "password": password})),
        &[],
    );
    assert_eq!(status, 200);
    assert_eq!(login["username"], "admin");
    let set_cookie = response_header(&response, "set-cookie").unwrap();
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));
    set_cookie.split(';').next().unwrap().to_owned()
}

fn bootstrap_owner_account(db: &Path, username: &str, password: &str) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_senixd"))
        .args([
            "owner",
            "bootstrap",
            "--db",
            db.to_str().unwrap(),
            "--username",
            username,
            "--password-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(password.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "owner bootstrap failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_config(path: &Path, backend_a: SocketAddr, backend_b: SocketAddr) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "routes": [{
                "id": "route-main",
                "host": "example.test",
                "path_prefix": "/",
                "backends": [
                    {"id": "instance-a", "address": backend_a, "generation": 1, "weight": 100},
                    {
                        "id": "instance-b",
                        "address": backend_b,
                        "generation": 1,
                        "weight": 100,
                        "health_check": {
                            "protocol": "tcp",
                            "interval_ms": 50,
                            "timeout_ms": 200,
                            "healthy_threshold": 1,
                            "unhealthy_threshold": 2
                        }
                    }
                ]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_health_config(path: &Path, backend_a: SocketAddr, backend_b: SocketAddr) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "routes": [{
                "id": "route-main",
                "host": "example.test",
                "path_prefix": "/",
                "backends": [
                    {
                        "id": "instance-a",
                        "address": backend_a,
                        "generation": 1,
                        "weight": 100,
                        "health_check": {
                            "protocol": "http",
                            "path": "/health",
                            "interval_ms": 50,
                            "timeout_ms": 200,
                            "healthy_threshold": 2,
                            "unhealthy_threshold": 2
                        }
                    },
                    {"id": "instance-b", "address": backend_b, "generation": 1, "weight": 100}
                ]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_single_backend_config(path: &Path, backend: SocketAddr) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "routes": [{
                "id": "route-main",
                "host": "example.test",
                "path_prefix": "/",
                "backends": [
                    {"id": "instance-a", "address": backend, "generation": 1, "weight": 100}
                ]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_https_backend_config(path: &Path, backend: SocketAddr) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "routes": [{
                "id": "route-main",
                "host": "example.test",
                "path_prefix": "/",
                "backends": [{
                    "id": "instance-a",
                    "address": backend,
                    "generation": 1,
                    "weight": 100,
                    "tls": {
                        "server_name": "backend.test",
                        "verify_certificate": false,
                        "alpn": "http1"
                    },
                    "health_check": {
                        "protocol": "http",
                        "path": "/health",
                        "interval_ms": 50,
                        "timeout_ms": 500,
                        "healthy_threshold": 1,
                        "unhealthy_threshold": 1
                    }
                }]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_h2_https_backend_config(path: &Path, backend: SocketAddr) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "routes": [{
                "id": "route-main",
                "host": "example.test",
                "path_prefix": "/",
                "backends": [{
                    "id": "instance-a",
                    "address": backend,
                    "generation": 1,
                    "weight": 100,
                    "tls": {
                        "server_name": "backend.test",
                        "verify_certificate": false,
                        "alpn": "http2"
                    },
                    "health_check": {
                        "protocol": "http",
                        "path": "/health",
                        "interval_ms": 50,
                        "timeout_ms": 500,
                        "healthy_threshold": 1,
                        "unhealthy_threshold": 1
                    }
                }]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_overlapping_route_config(path: &Path, root: SocketAddr, api: SocketAddr) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "routes": [
                {
                    "id": "route-root",
                    "host": "example.test",
                    "path_prefix": "/",
                    "backends": [
                        {"id": "instance-root", "address": root, "generation": 1, "weight": 100}
                    ]
                },
                {
                    "id": "route-api",
                    "host": "example.test",
                    "path_prefix": "/api",
                    "backends": [
                        {"id": "instance-api", "address": api, "generation": 1, "weight": 100}
                    ]
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_wildcard_route_config(path: &Path, wildcard: SocketAddr, exact: SocketAddr) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "routes": [
                {
                    "id": "route-wildcard",
                    "host": "*.example.test",
                    "path_prefix": "/",
                    "backends": [
                        {"id": "instance-wildcard", "address": wildcard, "generation": 1, "weight": 100}
                    ]
                },
                {
                    "id": "route-exact",
                    "host": "api.example.test",
                    "path_prefix": "/",
                    "backends": [
                        {"id": "instance-exact", "address": exact, "generation": 1, "weight": 100}
                    ]
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn spawn_backend(label: &'static str, slow: Duration) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            thread::spawn(move || {
                let request = read_headers(&mut stream);
                if request.starts_with("GET /slow ") {
                    thread::sleep(slow);
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    label.len(),
                    label
                );
                stream.write_all(response.as_bytes()).unwrap();
            });
        }
    });
    address
}

fn spawn_tls_backend(label: &'static str) -> SocketAddr {
    let identity = tempfile::tempdir().unwrap();
    let cert = identity.path().join("backend.crt");
    let key = identity.path().join("backend.key");
    write_test_certificate(&cert, &key);
    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor.set_certificate_chain_file(cert).unwrap();
    acceptor
        .set_private_key_file(key, SslFiletype::PEM)
        .unwrap();
    acceptor.check_private_key().unwrap();
    let acceptor = Arc::new(acceptor.build());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let acceptor = Arc::clone(&acceptor);
            let stream = stream.unwrap();
            thread::spawn(move || {
                let Ok(mut stream) = acceptor.accept(stream) else {
                    return;
                };
                let _request = read_headers(&mut stream);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    label.len(),
                    label
                );
                stream.write_all(response.as_bytes()).unwrap();
            });
        }
    });
    address
}

fn spawn_h2_tls_backend() -> SocketAddr {
    let identity = tempfile::tempdir().unwrap();
    let cert = identity.path().join("backend.crt");
    let key = identity.path().join("backend.key");
    write_test_certificate(&cert, &key);
    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor.set_certificate_chain_file(cert).unwrap();
    acceptor
        .set_private_key_file(key, SslFiletype::PEM)
        .unwrap();
    acceptor.set_alpn_select_callback(|_, protocols| {
        openssl::ssl::select_next_proto(b"\x02h2", protocols).ok_or(openssl::ssl::AlpnError::NOACK)
    });
    let acceptor = Arc::new(acceptor.build());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let acceptor = Arc::clone(&acceptor);
                tokio::spawn(async move {
                    let ssl = openssl::ssl::Ssl::new(acceptor.context()).unwrap();
                    let mut stream = tokio_openssl::SslStream::new(ssl, stream).unwrap();
                    if Pin::new(&mut stream).accept().await.is_err() {
                        return;
                    }
                    let Ok(mut connection) = h2::server::handshake(stream).await else {
                        return;
                    };
                    while let Some(Ok((_request, mut respond))) = connection.accept().await {
                        let response = Response::builder().status(200).body(()).unwrap();
                        let Ok(mut body) = respond.send_response(response, false) else {
                            return;
                        };
                        let _ = body.send_data(Bytes::from_static(b"H2_BACKEND"), true);
                    }
                });
            }
        });
    });
    address
}

fn spawn_controllable_backend(label: &'static str) -> (SocketAddr, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let healthy = Arc::new(AtomicBool::new(true));
    let backend_health = Arc::clone(&healthy);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let backend_health = Arc::clone(&backend_health);
            thread::spawn(move || {
                let request = read_headers(&mut stream);
                let is_health_check = request.starts_with("GET /health ");
                let is_healthy = backend_health.load(Ordering::SeqCst);
                let (status, body) = if is_health_check && !is_healthy {
                    ("503 Service Unavailable", "unhealthy")
                } else {
                    ("200 OK", label)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            });
        }
    });
    (address, healthy)
}

fn wait_until_ready(admin: SocketAddr, proxy: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let admin_ready = TcpStream::connect_timeout(&admin, Duration::from_millis(100)).is_ok();
        let proxy_ready = TcpStream::connect_timeout(&proxy, Duration::from_millis(100)).is_ok();
        if admin_ready && proxy_ready {
            return;
        }
        assert!(Instant::now() < deadline, "senixd did not become ready");
        thread::sleep(Duration::from_millis(40));
    }
}

fn wait_for_operation_state(admin: SocketAddr, operation_id: &str, expected: &str, bearer: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let operation = admin_json_with_bearer(
            admin,
            "GET",
            &format!("/api/v1/operations/{operation_id}"),
            None,
            None,
            bearer,
        );
        if operation["status"] == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "operation never became {expected}; last state: {operation}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_health_state(admin: SocketAddr, id: &str, expected: &str, bearer: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let state = admin_json_with_bearer(
            admin,
            "GET",
            &format!("/api/v1/instances/{id}"),
            None,
            None,
            bearer,
        );
        if state["health"] == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "instance health never became {expected}; last state: {state}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn proxy_get(address: SocketAddr, path: &str) -> String {
    proxy_get_host(address, "example.test", path)
}

fn proxy_get_host(address: SocketAddr, host: &str, path: &str) -> String {
    let response = raw_http(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
    );
    response.split("\r\n\r\n").nth(1).unwrap().to_owned()
}

fn admin_json_with_bearer(
    address: SocketAddr,
    method: &str,
    path: &str,
    idempotency_key: Option<&str>,
    body: Option<Value>,
    bearer: &str,
) -> Value {
    admin_response_with_bearer(address, method, path, idempotency_key, body, Some(bearer)).1
}

fn admin_response(
    address: SocketAddr,
    method: &str,
    path: &str,
    idempotency_key: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    admin_response_with_bearer(address, method, path, idempotency_key, body, None)
}

fn admin_response_with_bearer(
    address: SocketAddr,
    method: &str,
    path: &str,
    idempotency_key: Option<&str>,
    body: Option<Value>,
    bearer: Option<&str>,
) -> (u16, Value) {
    let key = idempotency_key.map_or_else(String::new, |key| format!("Idempotency-Key: {key}\r\n"));
    let authorization = bearer.map_or_else(String::new, |key| {
        format!("Authorization: Bearer {key}\r\n")
    });
    let headers = format!("{key}{authorization}");
    let (status, body, _) = admin_response_with_raw_headers(address, method, path, body, &headers);
    (status, body)
}

fn admin_response_with_headers(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (u16, Value, String) {
    let mut rendered_headers = String::new();
    for (name, value) in headers {
        write!(&mut rendered_headers, "{name}: {value}\r\n").unwrap();
    }
    admin_response_with_raw_headers(address, method, path, body, &rendered_headers)
}

fn admin_response_with_raw_headers(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: Option<Value>,
    headers: &str,
) -> (u16, Value, String) {
    let body = body.map_or_else(String::new, |body| body.to_string());
    let response = raw_http(
        address,
        &format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
            body.len()
        ),
    );
    let status = response
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let raw_body = response.split("\r\n\r\n").nth(1).unwrap();
    let body = if raw_body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(raw_body).unwrap()
    };
    (status, body, response)
}

fn response_header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    response
        .split("\r\n\r\n")
        .next()?
        .lines()
        .skip(1)
        .find_map(|line| {
            let (header_name, value) = line.split_once(':')?;
            header_name
                .eq_ignore_ascii_case(name)
                .then_some(value.trim())
        })
}

fn issue_api_key(
    admin: SocketAddr,
    owner_key: &str,
    label: &str,
    actions: &[&str],
    instance_ids: &[&str],
) -> (String, String) {
    let (status, issued) = admin_response_with_bearer(
        admin,
        "POST",
        "/api/v1/credentials",
        None,
        Some(json!({
            "label": label,
            "actions": actions,
            "instance_ids": instance_ids,
            "all_resources": false
        })),
        Some(owner_key),
    );
    assert_eq!(status, 201);
    (
        issued["credential_id"].as_str().unwrap().to_owned(),
        issued["api_key"].as_str().unwrap().to_owned(),
    )
}

fn issue_global_api_key_with_cookie(
    admin: SocketAddr,
    cookie: &str,
    label: &str,
    actions: &[&str],
) -> String {
    let (status, issued, _) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/credentials",
        Some(json!({
            "label": label,
            "actions": actions,
            "instance_ids": [],
            "all_resources": true
        })),
        &[("Cookie", cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 201);
    issued["api_key"].as_str().unwrap().to_owned()
}

fn assert_change_apply_tools(admin: SocketAddr, api_key: &str) {
    let (_, tools) = mcp_request(admin, Some(api_key), 40, "tools/list", &json!({}));
    let tool_names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"list_changes"));
    assert!(tool_names.contains(&"get_current_config"));
    assert!(tool_names.contains(&"apply_approved_change"));
    assert!(!tool_names.contains(&"approve_change"));
}

fn approve_config_as_owner(admin: SocketAddr, cookie: &str, id: &str) {
    let (status, plan, _) = admin_response_with_headers(
        admin,
        "POST",
        &format!("/api/v1/changes/{id}/approve"),
        None,
        &[("Cookie", cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 200);
    assert_eq!(plan["status"], "APPROVED");
}

fn publish_rest_config_as_owner(admin: SocketAddr, cookie: &str, config: Value) -> u64 {
    let headers = [("Cookie", cookie), ("X-Senix-CSRF", "1")];
    let (status, plan, _) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/changes/plan",
        Some(config),
        &headers,
    );
    assert_eq!(status, 201);
    let id = plan["change_id"].as_str().unwrap();
    approve_config_as_owner(admin, cookie, id);
    let (status, applied, _) = admin_response_with_headers(
        admin,
        "POST",
        &format!("/api/v1/changes/{id}/apply"),
        None,
        &headers,
    );
    assert_eq!(status, 200);
    applied["version"].as_u64().unwrap()
}

fn assert_config_plan_contract(admin: SocketAddr, api_key: &str, candidate: &Value) {
    // rmcp reports argument deserialization as a tool error, before domain planning.
    for arguments in [
        json!({"candidate": candidate}),
        json!({"expected_version": 1, "candidate": {"routes": "invalid"}}),
    ] {
        let rejected = call_mcp_tool(admin, api_key, "plan_change", &arguments);
        assert_eq!(rejected["isError"], true);
        assert!(
            rejected["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("failed to deserialize parameters")
        );
    }

    let (_, catalog) = mcp_request(admin, Some(api_key), 40, "tools/list", &json!({}));
    let tool = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "plan_change")
        .unwrap();
    let schema = &tool["inputSchema"];
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("expected_version")));
    assert!(required.contains(&json!("candidate")));
    assert_eq!(schema["properties"]["expected_version"]["type"], "integer");
    let resolve = |value: &Value| {
        // Schemars may inline a schema or refer to a shared definition.
        value.get("$ref").and_then(Value::as_str).map_or_else(
            || value.clone(),
            |reference| {
                schema
                    .pointer(reference.strip_prefix('#').unwrap())
                    .unwrap()
                    .clone()
            },
        )
    };
    let candidate = resolve(&schema["properties"]["candidate"]);
    assert_eq!(candidate["type"], "object");
    assert_eq!(candidate["additionalProperties"], false);
    let route = resolve(&candidate["properties"]["routes"]["items"]);
    for field in ["id", "host", "path_prefix", "backends"] {
        assert!(
            route["required"]
                .as_array()
                .unwrap()
                .contains(&json!(field))
        );
    }
    let backend = resolve(&route["properties"]["backends"]["items"]);
    for field in ["id", "address", "generation", "weight"] {
        assert!(
            backend["required"]
                .as_array()
                .unwrap()
                .contains(&json!(field))
        );
    }
    assert_eq!(backend["properties"]["address"]["type"], "string");
    assert!(backend["properties"].get("tls").is_some());
    assert!(backend["properties"].get("health_check").is_some());
}

fn call_mcp_tool(admin: SocketAddr, api_key: &str, name: &str, arguments: &Value) -> Value {
    let (status, response) = mcp_request(
        admin,
        Some(api_key),
        42,
        "tools/call",
        &json!({"name": name, "arguments": arguments}),
    );
    assert_eq!(status, 200);
    assert!(response.get("error").is_none(), "{response}");
    response["result"].clone()
}

fn assert_key_not_listed(admin: SocketAddr, owner_key: &str, api_key: &str) {
    let (status, credentials) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/credentials",
        None,
        None,
        Some(owner_key),
    );
    assert_eq!(status, 200);
    assert!(!credentials.to_string().contains(api_key));
}

fn assert_modern_tool_catalog_is_private(admin: SocketAddr, api_key: &str) {
    let (_, tools) = mcp_request_for_version(
        admin,
        Some(api_key),
        30,
        "tools/list",
        &json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {"name": "senix-e2e", "version": "1.0"},
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }),
        "2026-07-28",
    );
    assert_eq!(tools["result"]["ttlMs"], 0);
    assert_eq!(tools["result"]["cacheScope"], "private");
}

fn mcp_request(
    address: SocketAddr,
    bearer: Option<&str>,
    id: u64,
    method: &str,
    params: &Value,
) -> (u16, Value) {
    mcp_request_for_version(address, bearer, id, method, params, "2025-11-25")
}

fn mcp_request_for_version(
    address: SocketAddr,
    bearer: Option<&str>,
    id: u64,
    method: &str,
    params: &Value,
    protocol_version: &str,
) -> (u16, Value) {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
    .to_string();
    let authorization = bearer.map_or_else(String::new, |key| {
        format!("Authorization: Bearer {key}\r\n")
    });
    let response = raw_http(
        address,
        &format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nMCP-Protocol-Version: {protocol_version}\r\nMcp-Method: {method}\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    );
    let status = response
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let raw_body = response.split("\r\n\r\n").nth(1).unwrap();
    let body = if raw_body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(raw_body).unwrap()
    };
    (status, body)
}

fn raw_http(address: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn write_test_http_response(stream: &mut impl Write, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
}

fn read_headers(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        request.extend_from_slice(&buffer[..count]);
        if count == 0 || request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(request).unwrap()
}

fn free_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}
