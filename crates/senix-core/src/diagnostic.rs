use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{GatewayRuntime, HealthState, TrafficState};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticOutcome {
    Ready,
    RouteNotFound,
    NoAvailableBackend,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticStep {
    pub stage: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub host: String,
    pub path: String,
    pub outcome: DiagnosticOutcome,
    pub steps: Vec<DiagnosticStep>,
}

/// Produces deterministic routing evidence from the same snapshot used by requests.
#[derive(Clone, Debug)]
pub struct DiagnosticEngine {
    runtime: Arc<GatewayRuntime>,
}

impl DiagnosticEngine {
    #[must_use]
    pub fn new(runtime: Arc<GatewayRuntime>) -> Self {
        Self { runtime }
    }

    #[must_use]
    pub fn diagnose(&self, host: &str, path: &str) -> DiagnosticReport {
        let Some((route_id, instances)) = self.runtime.describe_route(host, path) else {
            return DiagnosticReport {
                host: host.to_owned(),
                path: path.to_owned(),
                outcome: DiagnosticOutcome::RouteNotFound,
                steps: vec![DiagnosticStep {
                    stage: "route_match".to_owned(),
                    status: "failed".to_owned(),
                    detail: format!("no route matched host={host} path={path}"),
                }],
            };
        };

        let mut steps = vec![DiagnosticStep {
            stage: "route_match".to_owned(),
            status: "passed".to_owned(),
            detail: format!("matched route {route_id}"),
        }];
        let mut available = false;
        for instance in instances {
            let serving = instance.traffic == TrafficState::Serving
                && instance.health == HealthState::Healthy
                && instance.weight > 0;
            available |= serving;
            steps.push(DiagnosticStep {
                stage: "backend_state".to_owned(),
                status: if serving { "passed" } else { "blocked" }.to_owned(),
                detail: format!(
                    "instance={} traffic={:?} health={:?} weight={} in_flight={}",
                    instance.id,
                    instance.traffic,
                    instance.health,
                    instance.weight,
                    instance.in_flight
                ),
            });
        }
        steps.push(DiagnosticStep {
            stage: "backend_selection".to_owned(),
            status: if available { "passed" } else { "failed" }.to_owned(),
            detail: if available {
                "at least one backend can accept a new request"
            } else {
                "no backend can accept a new request"
            }
            .to_owned(),
        });

        DiagnosticReport {
            host: host.to_owned(),
            path: path.to_owned(),
            outcome: if available {
                DiagnosticOutcome::Ready
            } else {
                DiagnosticOutcome::NoAvailableBackend
            },
            steps,
        }
    }
}
