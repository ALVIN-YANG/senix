use std::{
    collections::HashSet,
    fmt::Write as _,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    ConfigStateStore, CredentialKind, Error, GatewayConfig, GatewayRuntime, HealthCheckProtocol,
    Principal, Result,
};

const APPROVAL_TTL_MS: i64 = 15 * 60 * 1_000;

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
pub struct ChangeActor {
    pub credential_id: Uuid,
    pub label: String,
}

impl From<&Principal> for ChangeActor {
    fn from(principal: &Principal) -> Self {
        Self {
            credential_id: principal.credential_id,
            label: principal.label.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChangeKind {
    Configuration,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChangeStatus {
    Planned,
    Approved,
    Applied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePlan {
    pub change_id: Uuid,
    pub kind: ChangeKind,
    pub rollback_target_version: Option<u64>,
    pub base_version: u64,
    pub candidate_digest: String,
    pub candidate: GatewayConfig,
    pub diff: ConfigDiff,
    pub issues: Vec<ConfigIssue>,
    pub status: ChangeStatus,
    pub created_at_ms: i64,
    pub created_by: ChangeActor,
    pub approved_at_ms: Option<i64>,
    #[serde(default)]
    pub approval_expires_at_ms: Option<i64>,
    pub approved_by: Option<ChangeActor>,
    pub applied_at_ms: Option<i64>,
    pub applied_by: Option<ChangeActor>,
    pub applied_version: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedChange {
    pub change_id: Uuid,
    pub version: u64,
    pub diff: ConfigDiff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub version: u64,
    pub config: GatewayConfig,
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

    /// Creates the first durable snapshot without creating a management change.
    ///
    /// This is reserved for local process bootstrap before the management plane starts.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration is invalid, a snapshot already exists, persistence
    /// fails, or persisted instance states cannot be read.
    pub fn initialize(&self, candidate: GatewayConfig) -> Result<u64> {
        let issues = validate(&candidate);
        if !issues.is_empty() {
            return Err(Error::InvalidConfig);
        }
        let _guard = self.publish_lock.lock();
        if self.store.latest_config()?.is_some() {
            return Err(Error::InvalidState(
                "configuration has already been initialized".to_owned(),
            ));
        }
        let states = self.store.load_instance_states()?;
        self.store.save_config(1, &candidate)?;
        self.runtime.publish(candidate, states);
        Ok(1)
    }

    /// Creates and persists an immutable, version-bound Change Plan.
    ///
    /// # Errors
    ///
    /// Returns an error when the current snapshot cannot be read or the plan cannot be persisted.
    pub fn plan(&self, candidate: GatewayConfig, actor: &Principal) -> Result<ChangePlan> {
        self.plan_with_kind(candidate, actor, ChangeKind::Configuration, None)
    }

    /// Creates a Change Plan whose candidate is a prior immutable Snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the target snapshot does not exist or the plan cannot be persisted.
    pub fn plan_rollback(&self, target_version: u64, actor: &Principal) -> Result<ChangePlan> {
        let candidate = self
            .store
            .config_at(target_version)?
            .ok_or(Error::SnapshotNotFound(target_version))?;
        self.plan_with_kind(candidate, actor, ChangeKind::Rollback, Some(target_version))
    }

    fn plan_with_kind(
        &self,
        candidate: GatewayConfig,
        actor: &Principal,
        kind: ChangeKind,
        rollback_target_version: Option<u64>,
    ) -> Result<ChangePlan> {
        let _guard = self.publish_lock.lock();
        let current = self.store.latest_config()?;
        let base_version = current.as_ref().map_or(0, |(version, _)| *version);
        let diff = diff(current.as_ref().map(|(_, config)| config), &candidate);
        let issues = validate(&candidate);
        let plan = ChangePlan {
            change_id: Uuid::new_v4(),
            kind,
            rollback_target_version,
            base_version,
            candidate_digest: candidate_digest(&candidate)?,
            candidate,
            diff,
            issues,
            status: ChangeStatus::Planned,
            created_at_ms: now_ms(),
            created_by: actor.into(),
            approved_at_ms: None,
            approval_expires_at_ms: None,
            approved_by: None,
            applied_at_ms: None,
            applied_by: None,
            applied_version: None,
        };
        self.store.save_change(&plan)?;
        Ok(plan)
    }

    /// Approves the exact persisted Change Plan as the Owner.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not the Owner, the plan is unknown or invalid, its
    /// persisted content has changed, or persistence fails.
    pub fn approve(&self, change_id: Uuid, actor: &Principal) -> Result<ChangePlan> {
        if actor.kind != CredentialKind::Owner {
            return Err(Error::Forbidden {
                action: "change.approve".to_owned(),
                resource: format!("change/{change_id}"),
            });
        }
        let _guard = self.publish_lock.lock();
        let mut plan = self
            .store
            .change(change_id)?
            .ok_or_else(|| Error::ChangeNotFound(change_id.to_string()))?;
        verify_digest(&plan)?;
        if !plan.issues.is_empty() {
            return Err(Error::InvalidConfig);
        }
        if plan.status == ChangeStatus::Applied {
            return Ok(plan);
        }
        let current_version = self
            .store
            .latest_config()?
            .map_or(0, |(version, _)| version);
        if current_version != plan.base_version {
            return Err(Error::StalePlan);
        }
        let approved_at_ms = now_ms();
        if plan.status == ChangeStatus::Approved
            && plan
                .approval_expires_at_ms
                .is_some_and(|expires_at| expires_at > approved_at_ms)
        {
            return Ok(plan);
        }
        plan.status = ChangeStatus::Approved;
        plan.approved_at_ms = Some(approved_at_ms);
        plan.approval_expires_at_ms = Some(approved_at_ms.saturating_add(APPROVAL_TTL_MS));
        plan.approved_by = Some(actor.into());
        self.store.update_change(&plan)?;
        Ok(plan)
    }

    /// Atomically persists and publishes an approved Change Plan.
    ///
    /// Repeating an already applied change id returns the original result without publishing a new
    /// Snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when approval is missing, the base Snapshot is stale, persisted content
    /// changed, validation fails, or persistence/publish preparation fails.
    pub fn apply(&self, change_id: Uuid, actor: &Principal) -> Result<AppliedChange> {
        let _guard = self.publish_lock.lock();
        let mut plan = self
            .store
            .change(change_id)?
            .ok_or_else(|| Error::ChangeNotFound(change_id.to_string()))?;
        verify_digest(&plan)?;
        if plan.status == ChangeStatus::Applied {
            return Ok(AppliedChange {
                change_id,
                version: plan.applied_version.ok_or_else(|| {
                    Error::InvalidState("applied change has no snapshot version".to_owned())
                })?,
                diff: plan.diff,
            });
        }
        if plan.status != ChangeStatus::Approved {
            return Err(Error::ChangeApprovalRequired(change_id.to_string()));
        }
        if plan
            .approval_expires_at_ms
            .is_none_or(|expires_at| expires_at <= now_ms())
        {
            return Err(Error::ChangeApprovalExpired(change_id.to_string()));
        }
        if !validate(&plan.candidate).is_empty() {
            return Err(Error::InvalidConfig);
        }
        let current_version = self
            .store
            .latest_config()?
            .map_or(0, |(version, _)| version);
        if current_version != plan.base_version {
            return Err(Error::StalePlan);
        }
        let version = current_version + 1;
        let states = self.store.load_instance_states()?;
        plan.status = ChangeStatus::Applied;
        plan.applied_at_ms = Some(now_ms());
        plan.applied_by = Some(actor.into());
        plan.applied_version = Some(version);
        self.store.commit_config_change(version, &plan)?;
        self.runtime.publish(plan.candidate, states);
        Ok(AppliedChange {
            change_id,
            version,
            diff: plan.diff,
        })
    }

    /// Returns one durable Change Plan.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence cannot be read or stored data is invalid.
    pub fn change(&self, change_id: Uuid) -> Result<Option<ChangePlan>> {
        self.store.change(change_id)
    }

    /// Lists durable Change Plans newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence cannot be read or stored data is invalid.
    pub fn list_changes(&self) -> Result<Vec<ChangePlan>> {
        self.store.list_changes()
    }

    /// Returns the current durable Snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence cannot be read or stored configuration is invalid.
    pub fn current(&self) -> Result<Option<ConfigSnapshot>> {
        Ok(self
            .store
            .latest_config()?
            .map(|(version, config)| ConfigSnapshot { version, config }))
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

fn candidate_digest(candidate: &GatewayConfig) -> Result<String> {
    let digest = Sha256::digest(serde_json::to_vec(candidate)?);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(encoded)
}

fn verify_digest(plan: &ChangePlan) -> Result<()> {
    if candidate_digest(&plan.candidate)? != plan.candidate_digest {
        return Err(Error::InvalidState(
            "persisted change content does not match its digest".to_owned(),
        ));
    }
    Ok(())
}

fn now_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch");
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
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
