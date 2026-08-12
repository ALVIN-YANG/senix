use std::{collections::BTreeSet, sync::Arc};

use senix_core::{
    AccessPolicy, BackendConfig, ChangeStatus, ConfigEngine, CredentialKind, Error, GatewayConfig,
    GatewayRuntime, HealthCheckConfig, HealthCheckProtocol, Principal, RouteConfig,
    SqliteStateStore,
};
use uuid::Uuid;

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

fn principal(kind: CredentialKind, label: &str) -> Principal {
    Principal {
        credential_id: Uuid::new_v4(),
        label: label.to_owned(),
        kind,
        policy: AccessPolicy {
            all_resources: true,
            actions: BTreeSet::default(),
            instance_ids: BTreeSet::default(),
        },
    }
}

#[test]
fn a_change_cannot_publish_until_the_owner_approves_its_exact_plan() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), Arc::clone(&store));
    let owner = principal(CredentialKind::Owner, "admin");
    let automation = principal(CredentialKind::ApiKey, "config-agent");

    engine.initialize(valid_config("old.test")).unwrap();
    let planned = engine.plan(valid_config("new.test"), &automation).unwrap();

    assert_eq!(planned.status, ChangeStatus::Planned);
    assert_eq!(planned.created_by.label, "config-agent");
    assert_eq!(
        engine
            .apply(planned.change_id, &automation)
            .unwrap_err()
            .code(),
        Error::ChangeApprovalRequired(planned.change_id.to_string()).code()
    );
    assert!(runtime.acquire("old.test", "/").is_ok());
    assert!(runtime.acquire("new.test", "/").is_err());

    let approved = engine.approve(planned.change_id, &owner).unwrap();
    assert_eq!(approved.status, ChangeStatus::Approved);
    assert_eq!(approved.approved_by.unwrap().label, "admin");
    assert!(approved.approval_expires_at_ms.unwrap() > approved.approved_at_ms.unwrap());

    let applied = engine.apply(planned.change_id, &automation).unwrap();
    assert_eq!(applied.version, 2);
    assert_eq!(applied.change_id, planned.change_id);
    assert!(runtime.acquire("old.test", "/").is_err());
    assert!(runtime.acquire("new.test", "/").is_ok());

    drop(engine);
    drop(runtime);
    drop(store);

    let reopened_store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let reopened_runtime = Arc::new(GatewayRuntime::new());
    let reopened = ConfigEngine::new(Arc::clone(&reopened_runtime), reopened_store);
    let stored = reopened.change(planned.change_id).unwrap().unwrap();
    assert_eq!(stored.status, ChangeStatus::Applied);
    assert_eq!(stored.applied_version, Some(2));
    assert_eq!(reopened.restore_latest().unwrap(), Some(2));
    assert!(reopened_runtime.acquire("new.test", "/").is_ok());
}

#[test]
fn invalid_health_check_never_reaches_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), store);
    let owner = principal(CredentialKind::Owner, "admin");
    let mut invalid = valid_config("example.test");
    invalid.routes[0].backends[0].health_check = Some(HealthCheckConfig {
        protocol: HealthCheckProtocol::Http,
        path: "/health".into(),
        interval_ms: 50,
        timeout_ms: 0,
        healthy_threshold: 2,
        unhealthy_threshold: 2,
    });

    let plan = engine.plan(invalid, &owner).unwrap();
    assert_eq!(plan.issues[0].code, "INVALID_HEALTH_TIMEOUT");
    assert!(engine.approve(plan.change_id, &owner).is_err());
    assert!(runtime.acquire("example.test", "/").is_err());
}

#[test]
fn invalid_config_never_reaches_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), store);
    let owner = principal(CredentialKind::Owner, "admin");

    let mut invalid = valid_config("example.test");
    invalid.routes.push(invalid.routes[0].clone());
    let plan = engine.plan(invalid, &owner).unwrap();

    assert_eq!(plan.issues[0].code, "DUPLICATE_ROUTE_ID");
    assert!(engine.approve(plan.change_id, &owner).is_err());
    assert!(runtime.acquire("example.test", "/").is_err());
}

#[test]
fn applying_and_rolling_back_switches_complete_route_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    let engine = ConfigEngine::new(Arc::clone(&runtime), Arc::clone(&store));
    let owner = principal(CredentialKind::Owner, "admin");

    assert_eq!(engine.initialize(valid_config("old.test")).unwrap(), 1);
    assert!(runtime.acquire("old.test", "/").is_ok());

    let second_plan = engine.plan(valid_config("new.test"), &owner).unwrap();
    engine.approve(second_plan.change_id, &owner).unwrap();
    let second = engine.apply(second_plan.change_id, &owner).unwrap();
    assert_eq!(second.version, 2);
    assert!(runtime.acquire("old.test", "/").is_err());
    assert!(runtime.acquire("new.test", "/").is_ok());

    let rollback_plan = engine.plan_rollback(1, &owner).unwrap();
    engine.approve(rollback_plan.change_id, &owner).unwrap();
    let rollback = engine.apply(rollback_plan.change_id, &owner).unwrap();
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
    let owner = principal(CredentialKind::Owner, "admin");

    engine.initialize(valid_config("example.test")).unwrap();
    let existing = runtime.acquire("example.test", "/slow").unwrap();
    assert_eq!(existing.address(), "127.0.0.1:4101".parse().unwrap());

    let mut changed = valid_config("example.test");
    changed.routes[0].backends[0].address = "127.0.0.1:4201".parse().unwrap();
    let changed_plan = engine.plan(changed, &owner).unwrap();
    engine.approve(changed_plan.change_id, &owner).unwrap();
    engine.apply(changed_plan.change_id, &owner).unwrap();

    assert_eq!(existing.address(), "127.0.0.1:4101".parse().unwrap());
    assert_eq!(
        runtime.acquire("example.test", "/new").unwrap().address(),
        "127.0.0.1:4201".parse().unwrap()
    );
}
