use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{GatewayRuntime, InstanceState, InstanceStateStore, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainOptions {
    pub force: bool,
    pub timeout_ms: u64,
}

impl Default for DrainOptions {
    fn default() -> Self {
        Self {
            force: false,
            timeout_ms: 60_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DrainOperationStatus {
    Draining,
    Drained,
    DrainTimeout,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainOperation {
    pub operation_id: String,
    pub instance_id: String,
    #[serde(default)]
    pub generation: u64,
    pub status: DrainOperationStatus,
    pub ordinary_in_flight: u64,
    pub long_lived_in_flight: u64,
    pub started_at_ms: u64,
    pub deadline_at_ms: u64,
    pub force: bool,
}

/// Durable instance traffic operations shared by HTTP, CLI and MCP adapters.
pub struct TrafficController {
    runtime: Arc<GatewayRuntime>,
    store: Arc<dyn InstanceStateStore>,
    idempotency_lock: Mutex<()>,
}

impl std::fmt::Debug for TrafficController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrafficController")
            .finish_non_exhaustive()
    }
}

impl TrafficController {
    pub fn new<S>(runtime: Arc<GatewayRuntime>, store: Arc<S>) -> Self
    where
        S: InstanceStateStore + 'static,
    {
        Self {
            runtime,
            store,
            idempotency_lock: Mutex::new(()),
        }
    }

    /// Starts a bounded drain operation and immediately closes the instance to new requests.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero timeout, a missing instance, removal of the last available
    /// backend without `force`, an idempotency conflict, or persistence failure.
    pub fn begin_drain(
        &self,
        id: &str,
        options: DrainOptions,
        idempotency_key: &str,
    ) -> Result<DrainOperation> {
        if options.timeout_ms == 0 {
            return Err(crate::Error::InvalidState(
                "drain timeout must be greater than zero".to_owned(),
            ));
        }
        let _guard = self.idempotency_lock.lock();
        if let Some(operation) = self
            .store
            .load_drain_operation_by_key(idempotency_key, id)?
        {
            return Ok(operation);
        }
        let started_at_ms = unix_time_ms()?;
        let before = self.runtime.instance_state(id)?;
        let state = self.runtime.drain(id, options.force)?;
        let status = if state.in_flight == 0 {
            DrainOperationStatus::Drained
        } else {
            DrainOperationStatus::Draining
        };
        let operation = DrainOperation {
            operation_id: Uuid::new_v4().to_string(),
            instance_id: id.to_owned(),
            generation: state.generation,
            status,
            ordinary_in_flight: state.in_flight.saturating_sub(state.long_lived_in_flight),
            long_lived_in_flight: state.long_lived_in_flight,
            started_at_ms,
            deadline_at_ms: started_at_ms.saturating_add(options.timeout_ms),
            force: options.force,
        };
        if let Err(error) =
            self.store
                .commit_drain_operation(idempotency_key, id, &state, &operation)
        {
            self.runtime.restore_control_state(&before)?;
            return Err(error);
        }
        Ok(operation)
    }

    /// Returns the latest durable drain state and current in-flight request count.
    ///
    /// # Errors
    ///
    /// Returns an error when the operation or instance no longer exists, the clock is invalid, or
    /// persistence fails.
    pub fn drain_status(&self, operation_id: &str) -> Result<DrainOperation> {
        let _guard = self.idempotency_lock.lock();
        let mut operation = self
            .store
            .load_drain_operation(operation_id)?
            .ok_or_else(|| crate::Error::DrainOperationNotFound(operation_id.to_owned()))?;
        let before = operation.clone();
        let state = self.runtime.instance_state(&operation.instance_id)?;
        if state.generation == operation.generation {
            operation.ordinary_in_flight =
                state.in_flight.saturating_sub(state.long_lived_in_flight);
            operation.long_lived_in_flight = state.long_lived_in_flight;
            if operation.status == DrainOperationStatus::Draining && state.in_flight == 0 {
                operation.status = DrainOperationStatus::Drained;
            } else if operation.status == DrainOperationStatus::Draining
                && unix_time_ms()? >= operation.deadline_at_ms
            {
                operation.status = DrainOperationStatus::DrainTimeout;
            }
        } else {
            operation.ordinary_in_flight = 0;
            operation.long_lived_in_flight = 0;
            if operation.status == DrainOperationStatus::Draining {
                operation.status = DrainOperationStatus::Drained;
            }
        }
        if operation != before {
            self.store.update_drain_operation(&operation)?;
        }
        Ok(operation)
    }

    /// Returns the current traffic, health, generation, weight, and in-flight count.
    ///
    /// # Errors
    ///
    /// Returns an error when the instance does not exist.
    pub fn status(&self, id: &str) -> Result<InstanceState> {
        self.runtime.instance_state(id)
    }

    /// Lists all current instance states in stable identifier order.
    #[must_use]
    pub fn list_instances(&self) -> Vec<InstanceState> {
        self.runtime.instance_states()
    }

    /// Reopens an instance with a strictly newer deployment generation.
    ///
    /// # Errors
    ///
    /// Returns an error when the instance is missing or unhealthy, the generation does not
    /// increase, the key conflicts, or persistence fails.
    pub fn rejoin(
        &self,
        id: &str,
        generation: u64,
        weight: u32,
        force: bool,
        idempotency_key: &str,
    ) -> Result<InstanceState> {
        let _guard = self.idempotency_lock.lock();
        if let Some(result) = self
            .store
            .load_idempotent_result(idempotency_key, "rejoin", id)?
        {
            return Ok(result);
        }
        let before = self.runtime.instance_state(id)?;
        let state = self.runtime.rejoin(id, generation, weight, force)?;
        self.commit_or_restore(idempotency_key, "rejoin", id, &before, &state)?;
        Ok(state)
    }

    /// Changes the share of new requests assigned to a serving instance.
    ///
    /// # Errors
    ///
    /// Returns an error when the instance does not exist, the key conflicts, or persistence fails.
    pub fn set_weight(
        &self,
        id: &str,
        weight: u32,
        idempotency_key: &str,
    ) -> Result<InstanceState> {
        let _guard = self.idempotency_lock.lock();
        if let Some(result) =
            self.store
                .load_idempotent_result(idempotency_key, "set_weight", id)?
        {
            return Ok(result);
        }
        let before = self.runtime.instance_state(id)?;
        let state = self.runtime.set_weight(id, weight)?;
        self.commit_or_restore(idempotency_key, "set_weight", id, &before, &state)?;
        Ok(state)
    }

    /// Keeps an instance out of selection until an explicit rejoin.
    ///
    /// # Errors
    ///
    /// Returns an error when the instance does not exist, the key conflicts, or persistence fails.
    pub fn disable(&self, id: &str, idempotency_key: &str) -> Result<InstanceState> {
        let _guard = self.idempotency_lock.lock();
        if let Some(result) = self
            .store
            .load_idempotent_result(idempotency_key, "disable", id)?
        {
            return Ok(result);
        }
        let before = self.runtime.instance_state(id)?;
        let state = self.runtime.disable(id)?;
        self.commit_or_restore(idempotency_key, "disable", id, &before, &state)?;
        Ok(state)
    }

    fn commit_or_restore(
        &self,
        key: &str,
        operation: &str,
        id: &str,
        before: &InstanceState,
        after: &InstanceState,
    ) -> Result<()> {
        if let Err(error) = self
            .store
            .commit_instance_operation(key, operation, id, after)
        {
            self.runtime.restore_control_state(before)?;
            return Err(error);
        }
        Ok(())
    }
}

fn unix_time_ms() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            crate::Error::InvalidState(format!("system clock is before epoch: {error}"))
        })?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| crate::Error::InvalidState("system time does not fit in u64".to_owned()))
}
