use std::sync::Arc;

use senix_core::{
    BackendConfig, DrainOperation, DrainOperationStatus, DrainOptions, Error, GatewayConfig,
    GatewayRuntime, HealthState, InstanceState, InstanceStateStore, PersistedInstanceState, Result,
    RouteConfig, SqliteStateStore, TrafficController, TrafficState,
};

struct RejectingStore;

impl InstanceStateStore for RejectingStore {
    fn load_instance_states(&self) -> Result<Vec<PersistedInstanceState>> {
        Ok(vec![])
    }

    fn load_idempotent_result(
        &self,
        _key: &str,
        _operation: &str,
        _instance_id: &str,
    ) -> Result<Option<InstanceState>> {
        Ok(None)
    }

    fn commit_instance_operation(
        &self,
        _key: &str,
        _operation: &str,
        _instance_id: &str,
        _state: &InstanceState,
    ) -> Result<()> {
        Err(Error::InvalidState("write rejected".into()))
    }

    fn load_drain_operation_by_key(
        &self,
        _key: &str,
        _instance_id: &str,
    ) -> Result<Option<DrainOperation>> {
        Ok(None)
    }

    fn commit_drain_operation(
        &self,
        _key: &str,
        _instance_id: &str,
        _state: &InstanceState,
        _operation: &DrainOperation,
    ) -> Result<()> {
        Err(Error::InvalidState("write rejected".into()))
    }
}

struct HealthChangingRejectingStore {
    runtime: Arc<GatewayRuntime>,
}

impl InstanceStateStore for HealthChangingRejectingStore {
    fn load_instance_states(&self) -> Result<Vec<PersistedInstanceState>> {
        Ok(vec![])
    }

    fn load_idempotent_result(
        &self,
        _key: &str,
        _operation: &str,
        _instance_id: &str,
    ) -> Result<Option<InstanceState>> {
        Ok(None)
    }

    fn commit_instance_operation(
        &self,
        _key: &str,
        _operation: &str,
        instance_id: &str,
        _state: &InstanceState,
    ) -> Result<()> {
        self.runtime
            .report_health(instance_id, HealthState::Unhealthy)?;
        Err(Error::InvalidState(
            "write rejected after health changed".into(),
        ))
    }

    fn load_drain_operation_by_key(
        &self,
        _key: &str,
        _instance_id: &str,
    ) -> Result<Option<DrainOperation>> {
        Ok(None)
    }

    fn commit_drain_operation(
        &self,
        _key: &str,
        instance_id: &str,
        _state: &InstanceState,
        _operation: &DrainOperation,
    ) -> Result<()> {
        self.runtime
            .report_health(instance_id, HealthState::Unhealthy)?;
        Err(Error::InvalidState(
            "write rejected after health changed".into(),
        ))
    }
}

fn test_config() -> GatewayConfig {
    GatewayConfig {
        routes: vec![RouteConfig {
            id: "route-main".into(),
            host: "example.test".into(),
            path_prefix: "/".into(),
            backends: vec![
                BackendConfig {
                    id: "instance-a".into(),
                    address: "127.0.0.1:4101".parse().unwrap(),
                    generation: 1,
                    weight: 100,
                    health_check: None,
                },
                BackendConfig {
                    id: "instance-b".into(),
                    address: "127.0.0.1:4102".parse().unwrap(),
                    generation: 1,
                    weight: 100,
                    health_check: None,
                },
            ],
        }],
    }
}

fn single_backend_config() -> GatewayConfig {
    let mut config = test_config();
    config.routes[0].backends.truncate(1);
    config
}

fn begin_drain(traffic: &TrafficController, id: &str, key: &str) -> Result<DrainOperation> {
    traffic.begin_drain(id, DrainOptions::default(), key)
}

#[test]
fn draining_stops_new_requests_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");

    let store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), store.load_instance_states().unwrap());
    let traffic = TrafficController::new(Arc::clone(&runtime), Arc::clone(&store));

    let held_request = runtime.acquire("example.test", "/slow").unwrap();
    assert_eq!(held_request.instance_id(), "instance-a");

    let operation = begin_drain(&traffic, "instance-a", "deploy-42").unwrap();
    assert_eq!(operation.status, DrainOperationStatus::Draining);
    assert_eq!(operation.ordinary_in_flight, 1);

    for _ in 0..8 {
        let request = runtime.acquire("example.test", "/new").unwrap();
        assert_eq!(request.instance_id(), "instance-b");
    }

    drop(held_request);
    assert_eq!(
        traffic.status("instance-a").unwrap().traffic,
        TrafficState::Drained
    );

    drop(traffic);
    drop(runtime);
    drop(store);

    let reopened_store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let reopened_runtime = Arc::new(GatewayRuntime::new());
    reopened_runtime.publish(
        test_config(),
        reopened_store.load_instance_states().unwrap(),
    );
    let reopened = TrafficController::new(Arc::clone(&reopened_runtime), reopened_store);

    assert_eq!(
        reopened.status("instance-a").unwrap(),
        InstanceState {
            id: "instance-a".into(),
            generation: 1,
            weight: 100,
            traffic: TrafficState::Drained,
            health: HealthState::Healthy,
            health_override: false,
            in_flight: 0,
        }
    );

    for _ in 0..4 {
        let request = reopened_runtime
            .acquire("example.test", "/after-restart")
            .unwrap();
        assert_eq!(request.instance_id(), "instance-b");
    }
}

#[test]
fn replaying_a_drain_key_does_not_drain_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), store.load_instance_states().unwrap());
    let traffic = TrafficController::new(Arc::clone(&runtime), store);

    let first = begin_drain(&traffic, "instance-a", "deploy-42").unwrap();
    assert_eq!(first.status, DrainOperationStatus::Drained);

    traffic
        .rejoin("instance-a", 2, 25, false, "rejoin-42")
        .unwrap();
    let replay = begin_drain(&traffic, "instance-a", "deploy-42").unwrap();

    assert_eq!(replay, first);
    assert_eq!(
        traffic.status("instance-a").unwrap().traffic,
        TrafficState::Serving
    );
}

#[test]
fn replaying_a_rejoin_key_does_not_rejoin_after_a_later_drain() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), store.load_instance_states().unwrap());
    let traffic = TrafficController::new(runtime, store);

    begin_drain(&traffic, "instance-a", "drain-before-rejoin").unwrap();
    let first = traffic
        .rejoin("instance-a", 2, 25, false, "rejoin-42")
        .unwrap();
    begin_drain(&traffic, "instance-a", "drain-after-rejoin").unwrap();

    let replay = traffic
        .rejoin("instance-a", 2, 25, false, "rejoin-42")
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        traffic.status("instance-a").unwrap().traffic,
        TrafficState::Drained
    );
}

#[test]
fn weights_shape_new_requests_and_disabling_removes_a_backend() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), store.load_instance_states().unwrap());
    let traffic = TrafficController::new(Arc::clone(&runtime), store);

    traffic
        .set_weight("instance-a", 25, "weight-instance-a")
        .unwrap();
    traffic
        .set_weight("instance-b", 75, "weight-instance-b")
        .unwrap();

    let mut a_requests = 0;
    let mut b_requests = 0;
    for _ in 0..40 {
        let request = runtime.acquire("example.test", "/weighted").unwrap();
        match request.instance_id() {
            "instance-a" => a_requests += 1,
            "instance-b" => b_requests += 1,
            other => panic!("unexpected backend: {other}"),
        }
    }
    assert_eq!((a_requests, b_requests), (10, 30));

    traffic.disable("instance-b", "disable-instance-b").unwrap();
    for _ in 0..8 {
        let request = runtime.acquire("example.test", "/disabled").unwrap();
        assert_eq!(request.instance_id(), "instance-a");
    }
}

#[test]
fn failed_persistence_cannot_rejoin_an_instance_in_memory() {
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), vec![]);
    let dir = tempfile::tempdir().unwrap();
    let durable = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let traffic = TrafficController::new(Arc::clone(&runtime), durable);
    begin_drain(&traffic, "instance-a", "drain-before-failure").unwrap();

    let rejecting = TrafficController::new(Arc::clone(&runtime), Arc::new(RejectingStore));
    assert!(
        rejecting
            .rejoin("instance-a", 2, 100, false, "failed-rejoin")
            .is_err()
    );
    assert_eq!(
        rejecting.status("instance-a").unwrap().traffic,
        TrafficState::Drained
    );
    for _ in 0..4 {
        assert_eq!(
            runtime
                .acquire("example.test", "/after-failure")
                .unwrap()
                .instance_id(),
            "instance-b"
        );
    }
}

#[test]
fn health_changes_selection_without_overwriting_traffic_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), vec![]);
    let traffic = TrafficController::new(Arc::clone(&runtime), store);

    let unhealthy = runtime
        .report_health("instance-a", HealthState::Unhealthy)
        .unwrap();
    assert_eq!(unhealthy.health, HealthState::Unhealthy);
    assert_eq!(unhealthy.traffic, TrafficState::Serving);
    for _ in 0..4 {
        assert_eq!(
            runtime
                .acquire("example.test", "/while-unhealthy")
                .unwrap()
                .instance_id(),
            "instance-b"
        );
    }

    traffic
        .disable("instance-a", "disable-after-failed-health")
        .unwrap();
    let recovered = runtime
        .report_health("instance-a", HealthState::Healthy)
        .unwrap();
    assert_eq!(recovered.health, HealthState::Healthy);
    assert_eq!(recovered.traffic, TrafficState::Disabled);
    for _ in 0..4 {
        assert_eq!(
            runtime
                .acquire("example.test", "/after-recovery")
                .unwrap()
                .instance_id(),
            "instance-b"
        );
    }
}

#[test]
fn persistence_rollback_does_not_overwrite_a_newer_health_result() {
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), vec![]);
    let store = Arc::new(HealthChangingRejectingStore {
        runtime: Arc::clone(&runtime),
    });
    let traffic = TrafficController::new(Arc::clone(&runtime), store);

    assert!(begin_drain(&traffic, "instance-a", "failing-drain").is_err());
    let state = traffic.status("instance-a").unwrap();
    assert_eq!(state.traffic, TrafficState::Serving);
    assert_eq!(state.health, HealthState::Unhealthy);
    for _ in 0..4 {
        assert_eq!(
            runtime
                .acquire("example.test", "/after-health-race")
                .unwrap()
                .instance_id(),
            "instance-b"
        );
    }
}

#[test]
fn last_available_backend_requires_an_explicit_forced_drain() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(single_backend_config(), vec![]);
    let traffic = TrafficController::new(Arc::clone(&runtime), store);

    let blocked = traffic.begin_drain(
        "instance-a",
        DrainOptions {
            force: false,
            timeout_ms: 1_000,
        },
        "single-instance-drain",
    );
    assert!(matches!(
        blocked,
        Err(Error::LastAvailableBackend { instance_id, route_id })
            if instance_id == "instance-a" && route_id == "route-main"
    ));
    assert_eq!(
        traffic.status("instance-a").unwrap().traffic,
        TrafficState::Serving
    );

    let forced = traffic
        .begin_drain(
            "instance-a",
            DrainOptions {
                force: true,
                timeout_ms: 1_000,
            },
            "single-instance-forced-drain",
        )
        .unwrap();
    assert_eq!(forced.status, DrainOperationStatus::Drained);
    assert_eq!(forced.ordinary_in_flight, 0);
    assert_eq!(forced.long_lived_in_flight, 0);
}

#[test]
fn drain_operation_is_idempotent_queryable_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("senix.db");
    let store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), vec![]);
    let traffic = TrafficController::new(Arc::clone(&runtime), Arc::clone(&store));
    let held_request = runtime.acquire("example.test", "/slow").unwrap();
    assert_eq!(held_request.instance_id(), "instance-a");

    let options = DrainOptions {
        force: false,
        timeout_ms: 1_000,
    };
    let started = traffic
        .begin_drain("instance-a", options, "durable-drain")
        .unwrap();
    assert_eq!(started.status, DrainOperationStatus::Draining);
    assert_eq!(started.ordinary_in_flight, 1);
    let replay = traffic
        .begin_drain("instance-a", options, "durable-drain")
        .unwrap();
    assert_eq!(replay, started);
    assert_eq!(
        traffic.drain_status(&started.operation_id).unwrap(),
        started
    );

    drop(held_request);
    let completed = traffic.drain_status(&started.operation_id).unwrap();
    assert_eq!(completed.status, DrainOperationStatus::Drained);
    assert_eq!(completed.ordinary_in_flight, 0);

    traffic
        .rejoin("instance-a", 2, 100, false, "rejoin-after-operation")
        .unwrap();
    let mut new_generation_request = None;
    for _ in 0..2 {
        let request = runtime.acquire("example.test", "/generation-two").unwrap();
        if request.instance_id() == "instance-a" {
            new_generation_request = Some(request);
            break;
        }
    }
    let new_generation_request = new_generation_request.unwrap();
    assert_eq!(
        traffic.drain_status(&started.operation_id).unwrap(),
        completed,
        "a completed drain must not count requests from a newer generation"
    );
    drop(new_generation_request);

    drop(traffic);
    drop(runtime);
    drop(store);
    let reopened_store = Arc::new(SqliteStateStore::open(&db).unwrap());
    let reopened_runtime = Arc::new(GatewayRuntime::new());
    reopened_runtime.publish(
        test_config(),
        reopened_store.load_instance_states().unwrap(),
    );
    let reopened = TrafficController::new(reopened_runtime, reopened_store);
    assert_eq!(
        reopened.drain_status(&started.operation_id).unwrap(),
        completed
    );
}

#[test]
fn drain_timeout_pauses_without_terminating_the_held_request() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), vec![]);
    let traffic = TrafficController::new(Arc::clone(&runtime), store);
    let held_request = runtime.acquire("example.test", "/long-running").unwrap();
    assert_eq!(held_request.instance_id(), "instance-a");
    let started = traffic
        .begin_drain(
            "instance-a",
            DrainOptions {
                force: false,
                timeout_ms: 20,
            },
            "timeout-drain",
        )
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(30));
    let timed_out = traffic.drain_status(&started.operation_id).unwrap();
    assert_eq!(timed_out.status, DrainOperationStatus::DrainTimeout);
    assert_eq!(timed_out.ordinary_in_flight, 1);
    assert_eq!(
        traffic.status("instance-a").unwrap().traffic,
        TrafficState::Draining
    );

    drop(held_request);
    let after_completion = traffic.drain_status(&started.operation_id).unwrap();
    assert_eq!(after_completion.status, DrainOperationStatus::DrainTimeout);
    assert_eq!(after_completion.ordinary_in_flight, 0);
    assert_eq!(
        traffic.status("instance-a").unwrap().traffic,
        TrafficState::Drained
    );
}

#[test]
fn rejoin_waits_until_the_draining_generation_has_no_in_flight_requests() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), vec![]);
    let traffic = TrafficController::new(Arc::clone(&runtime), store);
    let held_request = runtime.acquire("example.test", "/slow").unwrap();
    assert_eq!(held_request.instance_id(), "instance-a");
    begin_drain(&traffic, "instance-a", "drain-before-early-rejoin").unwrap();

    assert!(
        traffic
            .rejoin("instance-a", 2, 100, false, "early-rejoin")
            .is_err()
    );
    assert_eq!(
        traffic.status("instance-a").unwrap().traffic,
        TrafficState::Draining
    );

    drop(held_request);
    let rejoined = traffic
        .rejoin("instance-a", 2, 100, false, "completed-rejoin")
        .unwrap();
    assert_eq!(rejoined.traffic, TrafficState::Serving);
    assert_eq!(rejoined.generation, 2);
}

#[test]
fn forced_rejoin_routes_without_falsifying_health() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStateStore::open(dir.path().join("senix.db")).unwrap());
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(test_config(), vec![]);
    let traffic = TrafficController::new(Arc::clone(&runtime), store);
    begin_drain(&traffic, "instance-a", "drain-before-health-override").unwrap();
    runtime
        .report_health("instance-a", HealthState::Unhealthy)
        .unwrap();

    assert!(
        traffic
            .rejoin("instance-a", 2, 100, false, "unsafe-normal-rejoin")
            .is_err()
    );
    let forced = traffic
        .rejoin("instance-a", 2, 100, true, "forced-health-rejoin")
        .unwrap();
    assert_eq!(forced.traffic, TrafficState::Serving);
    assert_eq!(forced.health, HealthState::Unhealthy);
    assert!(forced.health_override);

    let mut selected = Vec::new();
    for _ in 0..4 {
        selected.push(
            runtime
                .acquire("example.test", "/forced-health")
                .unwrap()
                .instance_id()
                .to_owned(),
        );
    }
    selected.sort();
    assert_eq!(
        selected,
        ["instance-a", "instance-a", "instance-b", "instance-b"]
    );
}

#[test]
fn opening_an_existing_database_adds_the_health_override_column() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("legacy-slice.db");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE instance_states (
                 id TEXT PRIMARY KEY,
                 generation INTEGER NOT NULL,
                 weight INTEGER NOT NULL,
                 traffic TEXT NOT NULL,
                 health TEXT NOT NULL
             );
             INSERT INTO instance_states (id, generation, weight, traffic, health)
             VALUES ('instance-a', 1, 100, 'DRAINED', 'HEALTHY');",
        )
        .unwrap();
    drop(connection);

    let store = SqliteStateStore::open(&db).unwrap();
    let states = store.load_instance_states().unwrap();
    assert_eq!(states.len(), 1);
    assert!(!states[0].health_override);
    assert_eq!(states[0].traffic, TrafficState::Drained);
}
