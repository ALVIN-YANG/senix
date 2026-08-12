use std::sync::Arc;

use senix_core::{
    BackendConfig, ConfigEngine, GatewayConfig, GatewayRuntime, HealthCheckConfig,
    HealthCheckProtocol, RouteConfig, SqliteStateStore,
};

fn valid_config(host: &str) -> GatewayConfig {
    GatewayConfig {
        routes: vec![RouteConfig {
            id: "route-main".into(),
            host: host.into(),
            path_prefix: "/".into(),
            backends: vec![BackendConfig {
                id: "instance-a".into(),
                address: "127.0.0.1:4101".parse().unwrap(),
                generation: 1,
                weight: 100,
                health_check: None,
            }],
        }],
    }
}

#[test]
fn invalid_health_check_never_reaches_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), store);
    let mut invalid = valid_config("example.test");
    invalid.routes[0].backends[0].health_check = Some(HealthCheckConfig {
        protocol: HealthCheckProtocol::Http,
        path: "/health".into(),
        interval_ms: 50,
        timeout_ms: 0,
        healthy_threshold: 2,
        unhealthy_threshold: 2,
    });

    let plan = engine.plan(invalid).unwrap();
    assert_eq!(plan.issues[0].code, "INVALID_HEALTH_TIMEOUT");
    assert!(engine.apply(plan).is_err());
    assert!(runtime.acquire("example.test", "/").is_err());
}

#[test]
fn invalid_config_never_reaches_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), store);

    let mut invalid = valid_config("example.test");
    invalid.routes.push(invalid.routes[0].clone());
    let plan = engine.plan(invalid).unwrap();

    assert_eq!(plan.issues[0].code, "DUPLICATE_ROUTE_ID");
    assert!(engine.apply(plan).is_err());
    assert!(runtime.acquire("example.test", "/").is_err());
}

#[test]
fn applying_and_rolling_back_switches_complete_route_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), Arc::clone(&store));

    let first = engine
        .apply(engine.plan(valid_config("old.test")).unwrap())
        .unwrap();
    assert_eq!(first.version, 1);
    assert!(runtime.acquire("old.test", "/").is_ok());

    let second = engine
        .apply(engine.plan(valid_config("new.test")).unwrap())
        .unwrap();
    assert_eq!(second.version, 2);
    assert!(runtime.acquire("old.test", "/").is_err());
    assert!(runtime.acquire("new.test", "/").is_ok());

    let rollback = engine.rollback(1).unwrap();
    assert_eq!(rollback.version, 3);
    assert!(runtime.acquire("old.test", "/").is_ok());
    assert!(runtime.acquire("new.test", "/").is_err());

    drop(engine);
    drop(runtime);
    drop(store);

    let reopened_store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let reopened_runtime = Arc::new(GatewayRuntime::new());
    let reopened = ConfigEngine::new(Arc::clone(&reopened_runtime), reopened_store);
    assert_eq!(reopened.restore_latest().unwrap(), Some(3));
    assert!(reopened_runtime.acquire("old.test", "/").is_ok());
}

#[test]
fn an_existing_lease_keeps_the_old_backend_address_after_publish() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), store);

    engine
        .apply(engine.plan(valid_config("example.test")).unwrap())
        .unwrap();
    let existing = runtime.acquire("example.test", "/slow").unwrap();
    assert_eq!(existing.address(), "127.0.0.1:4101".parse().unwrap());

    let mut changed = valid_config("example.test");
    changed.routes[0].backends[0].address = "127.0.0.1:4201".parse().unwrap();
    engine.apply(engine.plan(changed).unwrap()).unwrap();

    assert_eq!(existing.address(), "127.0.0.1:4101".parse().unwrap());
    assert_eq!(
        runtime.acquire("example.test", "/new").unwrap().address(),
        "127.0.0.1:4201".parse().unwrap()
    );
}
