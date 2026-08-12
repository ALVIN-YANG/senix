use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub routes: Vec<RouteConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub host: String,
    pub path_prefix: String,
    pub backends: Vec<BackendConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendConfig {
    pub id: String,
    pub address: SocketAddr,
    pub generation: u64,
    pub weight: u32,
    #[serde(default)]
    pub health_check: Option<HealthCheckConfig>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthCheckProtocol {
    Tcp,
    #[default]
    Http,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthCheckConfig {
    pub protocol: HealthCheckProtocol,
    pub path: String,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub healthy_threshold: u32,
    pub unhealthy_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            protocol: HealthCheckProtocol::Http,
            path: "/health".to_owned(),
            interval_ms: 5_000,
            timeout_ms: 1_000,
            healthy_threshold: 2,
            unhealthy_threshold: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrafficState {
    #[default]
    Serving,
    Draining,
    Drained,
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HealthState {
    Unknown,
    #[default]
    Healthy,
    Unhealthy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceState {
    pub id: String,
    pub generation: u64,
    pub weight: u32,
    pub traffic: TrafficState,
    pub health: HealthState,
    #[serde(default)]
    pub health_override: bool,
    pub in_flight: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedInstanceState {
    pub id: String,
    pub generation: u64,
    pub weight: u32,
    pub traffic: TrafficState,
    pub health: HealthState,
    #[serde(default)]
    pub health_override: bool,
}

impl From<&InstanceState> for PersistedInstanceState {
    fn from(value: &InstanceState) -> Self {
        Self {
            id: value.id.clone(),
            generation: value.generation,
            weight: value.weight,
            traffic: value.traffic,
            health: value.health,
            health_override: value.health_override,
        }
    }
}
