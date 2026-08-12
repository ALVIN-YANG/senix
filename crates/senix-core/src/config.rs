use std::{collections::HashSet, sync::Arc};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::{ConfigStateStore, Error, GatewayConfig, GatewayRuntime, HealthCheckProtocol, Result};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigIssue {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDiff {
    pub added_routes: Vec<String>,
    pub removed_routes: Vec<String>,
    pub changed_routes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePlan {
    pub base_version: u64,
    pub candidate: GatewayConfig,
    pub diff: ConfigDiff,
    pub issues: Vec<ConfigIssue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedChange {
    pub version: u64,
    pub diff: ConfigDiff,
}

/// Validates, versions, persists and atomically publishes complete gateway snapshots.
pub struct ConfigEngine {
    runtime: Arc<GatewayRuntime>,
    store: Arc<dyn ConfigStateStore>,
    publish_lock: Mutex<()>,
}

impl std::fmt::Debug for ConfigEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigEngine")
            .finish_non_exhaustive()
    }
}

impl ConfigEngine {
    pub fn new<S>(runtime: Arc<GatewayRuntime>, store: Arc<S>) -> Self
    where
        S: ConfigStateStore + 'static,
    {
        Self {
            runtime,
            store,
            publish_lock: Mutex::new(()),
        }
    }

    /// Builds a validated, version-bound plan without changing runtime state.
    ///
    /// # Errors
    ///
    /// Returns an error when the current snapshot cannot be read from the state store.
    pub fn plan(&self, candidate: GatewayConfig) -> Result<ChangePlan> {
        let current = self.store.latest_config()?;
        let base_version = current.as_ref().map_or(0, |(version, _)| *version);
        let diff = diff(current.as_ref().map(|(_, config)| config), &candidate);
        let issues = validate(&candidate);
        Ok(ChangePlan {
            base_version,
            candidate,
            diff,
            issues,
        })
    }

    /// Persists and atomically publishes a previously created plan.
    ///
    /// # Errors
    ///
    /// Returns an error for validation issues, a stale base version, persistence failure, or an
    /// unreadable set of persisted instance states.
    pub fn apply(&self, plan: ChangePlan) -> Result<AppliedChange> {
        if !plan.issues.is_empty() {
            return Err(Error::InvalidConfig);
        }
        let _guard = self.publish_lock.lock();
        let current_version = self
            .store
            .latest_config()?
            .map_or(0, |(version, _)| version);
        if current_version != plan.base_version {
            return Err(Error::StalePlan);
        }
        let version = current_version + 1;
        self.store.save_config(version, &plan.candidate)?;
        let states = self.store.load_instance_states()?;
        self.runtime.publish(plan.candidate, states);
        Ok(AppliedChange {
            version,
            diff: plan.diff,
        })
    }

    /// Publishes a prior snapshot as a new version.
    ///
    /// # Errors
    ///
    /// Returns an error when the target does not exist or when planning, persistence, or publish
    /// preparation fails.
    pub fn rollback(&self, target_version: u64) -> Result<AppliedChange> {
        let config = self
            .store
            .config_at(target_version)?
            .ok_or(Error::SnapshotNotFound(target_version))?;
        let plan = self.plan(config)?;
        self.apply(plan)
    }

    /// Restores the latest durable snapshot into the data plane.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot or persisted instance states cannot be read.
    pub fn restore_latest(&self) -> Result<Option<u64>> {
        let Some((version, config)) = self.store.latest_config()? else {
            return Ok(None);
        };
        let states = self.store.load_instance_states()?;
        self.runtime.publish(config, states);
        Ok(Some(version))
    }
}

fn validate(config: &GatewayConfig) -> Vec<ConfigIssue> {
    let mut issues = Vec::new();
    let mut route_ids = HashSet::new();
    let mut route_matches = HashSet::new();
    for route in &config.routes {
        if !route_ids.insert(route.id.as_str()) {
            issues.push(ConfigIssue {
                code: "DUPLICATE_ROUTE_ID".to_owned(),
                message: format!("route id appears more than once: {}", route.id),
            });
        }
        if !route_matches.insert((route.host.to_ascii_lowercase(), route.path_prefix.as_str())) {
            issues.push(ConfigIssue {
                code: "DUPLICATE_ROUTE_MATCH".to_owned(),
                message: format!(
                    "host and path prefix appear more than once: {}{}",
                    route.host, route.path_prefix
                ),
            });
        }
        if route.host.trim().is_empty() {
            issues.push(issue("EMPTY_HOST", &route.id));
        }
        if !route.path_prefix.starts_with('/') {
            issues.push(issue("INVALID_PATH_PREFIX", &route.id));
        }
        if route.backends.is_empty() {
            issues.push(issue("EMPTY_BACKEND_POOL", &route.id));
        } else if route.backends.iter().all(|backend| backend.weight == 0) {
            issues.push(issue("ZERO_BACKEND_WEIGHT", &route.id));
        }
        for backend in &route.backends {
            let Some(check) = &backend.health_check else {
                continue;
            };
            if check.interval_ms == 0 {
                issues.push(health_issue("INVALID_HEALTH_INTERVAL", &backend.id));
            }
            if check.timeout_ms == 0 {
                issues.push(health_issue("INVALID_HEALTH_TIMEOUT", &backend.id));
            }
            if check.healthy_threshold == 0 {
                issues.push(health_issue("INVALID_HEALTHY_THRESHOLD", &backend.id));
            }
            if check.unhealthy_threshold == 0 {
                issues.push(health_issue("INVALID_UNHEALTHY_THRESHOLD", &backend.id));
            }
            if check.protocol == HealthCheckProtocol::Http && !check.path.starts_with('/') {
                issues.push(health_issue("INVALID_HEALTH_PATH", &backend.id));
            }
        }
    }
    issues
}

fn issue(code: &str, route_id: &str) -> ConfigIssue {
    ConfigIssue {
        code: code.to_owned(),
        message: format!("route {route_id} failed {code}"),
    }
}

fn health_issue(code: &str, backend_id: &str) -> ConfigIssue {
    ConfigIssue {
        code: code.to_owned(),
        message: format!("backend {backend_id} failed {code}"),
    }
}

fn diff(current: Option<&GatewayConfig>, candidate: &GatewayConfig) -> ConfigDiff {
    let current_routes = current.map(|config| &config.routes[..]).unwrap_or_default();
    let mut result = ConfigDiff::default();

    for route in &candidate.routes {
        match current_routes.iter().find(|old| old.id == route.id) {
            None => result.added_routes.push(route.id.clone()),
            Some(old) if old != route => result.changed_routes.push(route.id.clone()),
            Some(_) => {}
        }
    }
    for route in current_routes {
        if !candidate.routes.iter().any(|new| new.id == route.id) {
            result.removed_routes.push(route.id.clone());
        }
    }
    result
}
