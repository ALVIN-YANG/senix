use std::sync::Arc;

use senix_core::{
    BackendConfig, DiagnosticEngine, DiagnosticOutcome, GatewayConfig, GatewayRuntime, RouteConfig,
};

#[test]
fn diagnosis_identifies_the_route_matching_failure() {
    let runtime = Arc::new(GatewayRuntime::new());
    runtime.publish(
        GatewayConfig {
            routes: vec![RouteConfig {
                id: "route-main".into(),
                host: "example.test".into(),
                path_prefix: "/api".into(),
                backends: vec![BackendConfig {
                    id: "instance-a".into(),
                    address: "127.0.0.1:4101".parse().unwrap(),
                    generation: 1,
                    weight: 100,
                    tls: None,
                    health_check: None,
                }],
            }],
        },
        vec![],
    );
    let diagnostics = DiagnosticEngine::new(runtime);

    let report = diagnostics.diagnose("example.test", "/missing");

    assert_eq!(report.outcome, DiagnosticOutcome::RouteNotFound);
    assert_eq!(report.steps[0].stage, "route_match");
    assert_eq!(report.steps[0].status, "failed");
    assert!(report.steps[0].detail.contains("/missing"));
}
