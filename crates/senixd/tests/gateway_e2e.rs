use std::{
    fmt::Write as _,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use openssl::{
    asn1::Asn1Time,
    bn::BigNum,
    hash::MessageDigest,
    pkey::PKey,
    rsa::Rsa,
    ssl::{SslConnector, SslMethod, SslVerifyMode},
    x509::{X509, X509NameBuilder},
};
use serde_json::{Value, json};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
fn an_ai_key_can_apply_only_the_exact_change_plan_approved_by_the_owner() {
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

    let mut candidate: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    candidate["routes"][0]["host"] = json!("approved.test");
    let (status, planned, _) = admin_response_with_headers(
        admin,
        "POST",
        "/api/v1/changes/plan",
        Some(candidate),
        &[("Cookie", &cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 201);
    assert_eq!(planned["status"], "PLANNED");
    assert_eq!(planned["created_by"]["label"], "admin");
    assert_eq!(planned["candidate_digest"].as_str().unwrap().len(), 64);
    let change_id = planned["change_id"].as_str().unwrap();

    let (status, missing_approval, _) = admin_response_with_headers(
        admin,
        "POST",
        &format!("/api/v1/changes/{change_id}/apply"),
        None,
        &[("Cookie", &cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 409);
    assert_eq!(missing_approval["code"], "CHANGE_APPROVAL_REQUIRED");

    let (status, approved, _) = admin_response_with_headers(
        admin,
        "POST",
        &format!("/api/v1/changes/{change_id}/approve"),
        None,
        &[("Cookie", &cookie), ("X-Senix-CSRF", "1")],
    );
    assert_eq!(status, 200);
    assert_eq!(approved["status"], "APPROVED");

    let apply_key = issue_global_api_key_with_cookie(
        admin,
        &cookie,
        "config-agent",
        &["change.read", "change.apply"],
    );

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

    assert_change_apply_tools(admin, &apply_key);

    let (_, applied) = mcp_request(
        admin,
        Some(&apply_key),
        41,
        "tools/call",
        &json!({
            "name": "apply_approved_change",
            "arguments": {"change_id": change_id}
        }),
    );
    assert_eq!(applied["result"]["structuredContent"]["version"], 2);
    let (status, replayed) = admin_response_with_bearer(
        admin,
        "POST",
        &format!("/api/v1/changes/{change_id}/apply"),
        None,
        None,
        Some(&apply_key),
    );
    assert_eq!(status, 200);
    assert_eq!(replayed["version"], 2);

    let (status, changes) = admin_response_with_bearer(
        admin,
        "GET",
        "/api/v1/changes",
        None,
        None,
        Some(&apply_key),
    );
    assert_eq!(status, 200);
    assert_eq!(changes[0]["status"], "APPLIED");
    assert_eq!(changes[0]["applied_version"], 2);
    let (status, current) =
        admin_response_with_bearer(admin, "GET", "/api/v1/config", None, None, Some(&apply_key));
    assert_eq!(status, 200);
    assert_eq!(current["version"], 2);
    assert_eq!(current["config"]["routes"][0]["host"], "approved.test");
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

    assert!(stream.join().unwrap().ends_with("\r\n\r\nA"));
    wait_for_operation_state(
        admin,
        operation["operation_id"].as_str().unwrap(),
        "DRAINED",
        &owner_key,
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

    let senix = spawn_senix_with_tls(proxy, tls, admin, &db, &config, &cert, &key);
    wait_until_ready(admin, proxy);

    let response = tls_get(tls, "example.test", "/secure");
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("\r\n\r\nA"));
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
    let response = raw_http(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n"),
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
    assert!(tool_names.contains(&"apply_approved_change"));
    assert!(!tool_names.contains(&"approve_change"));
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

fn read_headers(stream: &mut TcpStream) -> String {
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
