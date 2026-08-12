use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};

use crate::{
    AuditEvent, AuditOutcome, CredentialKind, CredentialSummary, DrainOperation, Error,
    GatewayConfig, HealthState, InstanceState, PersistedInstanceState, Result, RiskLevel,
    TrafficState,
    security::{StoredCredential, StoredOwnerAccount},
};

pub trait InstanceStateStore: Send + Sync {
    /// Loads all durable instance states.
    ///
    /// # Errors
    ///
    /// Returns an error when storage cannot be read or contains invalid serialized data.
    fn load_instance_states(&self) -> Result<Vec<PersistedInstanceState>>;
    /// Returns the response previously stored for an idempotent operation.
    ///
    /// # Errors
    ///
    /// Returns an error when storage cannot be read, the response is invalid, or the key was used
    /// for a different resource or operation.
    fn load_idempotent_result(
        &self,
        key: &str,
        operation: &str,
        instance_id: &str,
    ) -> Result<Option<InstanceState>>;
    /// Atomically stores the instance state and response for an idempotent operation.
    ///
    /// # Errors
    ///
    /// Returns an error when the response cannot be serialized or storage rejects the transaction.
    fn commit_instance_operation(
        &self,
        key: &str,
        operation: &str,
        instance_id: &str,
        state: &InstanceState,
    ) -> Result<()>;
    /// Returns the drain operation originally created for an idempotency key.
    ///
    /// # Errors
    ///
    /// Returns an error when the key conflicts or the Adapter cannot read drain operations.
    fn load_drain_operation_by_key(
        &self,
        _key: &str,
        _instance_id: &str,
    ) -> Result<Option<DrainOperation>> {
        Err(Error::InvalidState(
            "state store does not support drain operations".to_owned(),
        ))
    }
    /// Atomically stores the traffic state, drain operation, and idempotent response.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or persistence fails.
    fn commit_drain_operation(
        &self,
        _key: &str,
        _instance_id: &str,
        _state: &InstanceState,
        _operation: &DrainOperation,
    ) -> Result<()> {
        Err(Error::InvalidState(
            "state store does not support drain operations".to_owned(),
        ))
    }
    /// Loads a drain operation by its public identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence or deserialization fails.
    fn load_drain_operation(&self, _operation_id: &str) -> Result<Option<DrainOperation>> {
        Err(Error::InvalidState(
            "state store does not support drain operations".to_owned(),
        ))
    }
    /// Saves the latest observed drain status.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or persistence fails.
    fn update_drain_operation(&self, _operation: &DrainOperation) -> Result<()> {
        Err(Error::InvalidState(
            "state store does not support drain operations".to_owned(),
        ))
    }
}

pub trait ConfigStateStore: InstanceStateStore {
    /// Loads the newest configuration snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when storage cannot be read or the snapshot is invalid.
    fn latest_config(&self) -> Result<Option<(u64, GatewayConfig)>>;
    /// Loads one configuration snapshot by version.
    ///
    /// # Errors
    ///
    /// Returns an error when the version is out of range, storage cannot be read, or the snapshot
    /// is invalid.
    fn config_at(&self, version: u64) -> Result<Option<GatewayConfig>>;
    /// Saves one immutable configuration snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization fails, the version is out of range, or storage rejects
    /// the write.
    fn save_config(&self, version: u64, config: &GatewayConfig) -> Result<()>;
}

pub struct SqliteStateStore {
    connection: Mutex<Connection>,
}

impl std::fmt::Debug for SqliteStateStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqliteStateStore")
            .finish_non_exhaustive()
    }
}

impl SqliteStateStore {
    /// Opens a `SQLite` state database and applies the vertical-slice schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened or migrated.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS instance_states (
                 id TEXT PRIMARY KEY,
                 generation INTEGER NOT NULL,
                 weight INTEGER NOT NULL,
                 traffic TEXT NOT NULL,
                 health TEXT NOT NULL,
                 health_override INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS idempotent_results (
                 key TEXT PRIMARY KEY,
                 operation TEXT NOT NULL,
                 instance_id TEXT NOT NULL,
                 result_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS drain_operations (
                 operation_id TEXT PRIMARY KEY,
                 instance_id TEXT NOT NULL,
                 operation_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS config_snapshots (
                 version INTEGER PRIMARY KEY,
                 config_json TEXT NOT NULL,
                 created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS credentials (
                 id TEXT PRIMARY KEY,
                 label TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 salt BLOB NOT NULL,
                 digest BLOB NOT NULL,
                 policy_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 expires_at_ms INTEGER,
                 revoked_at_ms INTEGER
             );
             CREATE TABLE IF NOT EXISTS audit_events (
                 event_id TEXT PRIMARY KEY,
                 occurred_at_ms INTEGER NOT NULL,
                 credential_id TEXT NOT NULL,
                 credential_label TEXT NOT NULL,
                 action TEXT NOT NULL,
                 resource_type TEXT NOT NULL,
                 resource_id TEXT,
                 outcome TEXT NOT NULL,
                 risk TEXT NOT NULL,
                 details_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS owner_accounts (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 username TEXT NOT NULL UNIQUE,
                 password_hash TEXT NOT NULL,
                 owner_credential_id TEXT NOT NULL REFERENCES credentials(id),
                 session_secret BLOB NOT NULL,
                 created_at_ms INTEGER NOT NULL
             );",
        )?;
        let has_health_override = {
            let mut statement = connection.prepare("PRAGMA table_info(instance_states)")?;
            let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
            let mut found = false;
            for column in columns {
                if column? == "health_override" {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_health_override {
            connection.execute(
                "ALTER TABLE instance_states ADD COLUMN health_override INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Loads durable instance states through the `SQLite` Adapter.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be read or contains invalid data.
    pub fn load_instance_states(&self) -> Result<Vec<PersistedInstanceState>> {
        <Self as InstanceStateStore>::load_instance_states(self)
    }

    pub(crate) fn insert_bootstrap_credential(
        &self,
        credential: &StoredCredential,
        audit: &AuditEvent,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM credentials", [], |row| row.get(0))?;
        if count != 0 {
            return Err(Error::CredentialAlreadyInitialized);
        }
        insert_credential(&transaction, credential)?;
        insert_audit_event(&transaction, audit)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn insert_credential_with_audit(
        &self,
        credential: &StoredCredential,
        audit: &AuditEvent,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        insert_credential(&transaction, credential)?;
        insert_audit_event(&transaction, audit)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn owner_credential(&self) -> Result<Option<StoredCredential>> {
        let connection = self.connection.lock();
        let stored = connection
            .query_row(
                "SELECT id, label, kind, salt, digest, policy_json, created_at_ms,
                        expires_at_ms, revoked_at_ms
                 FROM credentials WHERE kind = 'OWNER' LIMIT 1",
                [],
                stored_credential_row,
            )
            .optional()?;
        decode_stored_credential(stored)
    }

    pub(crate) fn owner_account(&self) -> Result<Option<StoredOwnerAccount>> {
        let connection = self.connection.lock();
        connection
            .query_row(
                "SELECT username, password_hash, owner_credential_id, session_secret,
                        created_at_ms
                 FROM owner_accounts WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?
            .map(
                |(username, password_hash, owner_credential_id, session_secret, created_at_ms)| {
                    Ok(StoredOwnerAccount {
                        username,
                        password_hash,
                        owner_credential_id: uuid::Uuid::parse_str(&owner_credential_id).map_err(
                            |error| {
                                Error::InvalidState(format!(
                                    "stored owner credential id is invalid: {error}"
                                ))
                            },
                        )?,
                        session_secret,
                        created_at_ms,
                    })
                },
            )
            .transpose()
    }

    pub(crate) fn insert_owner_account_with_audit(
        &self,
        account: &StoredOwnerAccount,
        audit: &AuditEvent,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let exists: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM owner_accounts WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        if exists != 0 {
            return Err(Error::OwnerAccountAlreadyInitialized);
        }
        let revoked = transaction.execute(
            "UPDATE credentials
             SET revoked_at_ms = COALESCE(revoked_at_ms, ?2)
             WHERE id = ?1 AND kind = 'OWNER'",
            params![
                account.owner_credential_id.to_string(),
                account.created_at_ms
            ],
        )?;
        if revoked == 0 {
            return Err(Error::OwnerCredentialNotInitialized);
        }
        transaction.execute(
            "INSERT INTO owner_accounts
               (singleton, username, password_hash, owner_credential_id, session_secret,
                created_at_ms)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![
                account.username,
                account.password_hash,
                account.owner_credential_id.to_string(),
                account.session_secret,
                account.created_at_ms,
            ],
        )?;
        insert_audit_event(&transaction, audit)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn rotate_owner_session_secret_with_audit(
        &self,
        session_secret: &[u8],
        audit: &AuditEvent,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE owner_accounts SET session_secret = ?1 WHERE singleton = 1",
            [session_secret],
        )?;
        if changed == 0 {
            return Err(Error::OwnerAccountNotInitialized);
        }
        insert_audit_event(&transaction, audit)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn reset_owner_password_with_audit(
        &self,
        password_hash: &str,
        session_secret: &[u8],
        audit: &AuditEvent,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE owner_accounts
             SET password_hash = ?1, session_secret = ?2
             WHERE singleton = 1",
            params![password_hash, session_secret],
        )?;
        if changed == 0 {
            return Err(Error::OwnerAccountNotInitialized);
        }
        insert_audit_event(&transaction, audit)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn credential(&self, id: uuid::Uuid) -> Result<Option<StoredCredential>> {
        let connection = self.connection.lock();
        let stored = connection
            .query_row(
                "SELECT id, label, kind, salt, digest, policy_json, created_at_ms,
                        expires_at_ms, revoked_at_ms
                 FROM credentials WHERE id = ?1 LIMIT 1",
                [id.to_string()],
                stored_credential_row,
            )
            .optional()?;
        decode_stored_credential(stored)
    }

    pub(crate) fn list_credentials(&self) -> Result<Vec<CredentialSummary>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, label, kind, policy_json, created_at_ms, expires_at_ms, revoked_at_ms
             FROM credentials ORDER BY created_at_ms, id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })?;
        let mut credentials = Vec::new();
        for row in rows {
            let (id, label, kind, policy_json, created_at_ms, expires_at_ms, revoked_at_ms) = row?;
            credentials.push(CredentialSummary {
                credential_id: uuid::Uuid::parse_str(&id).map_err(|error| {
                    Error::InvalidState(format!("stored credential id is invalid: {error}"))
                })?,
                label,
                kind: parse_credential_kind(&kind)?,
                policy: serde_json::from_str(&policy_json)?,
                created_at_ms,
                expires_at_ms,
                revoked_at_ms,
            });
        }
        Ok(credentials)
    }

    pub(crate) fn revoke_credential_with_audit(
        &self,
        credential_id: uuid::Uuid,
        revoked_at_ms: i64,
        audit: &AuditEvent,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE credentials SET revoked_at_ms = COALESCE(revoked_at_ms, ?2) WHERE id = ?1",
            params![credential_id.to_string(), revoked_at_ms],
        )?;
        if changed == 0 {
            return Err(Error::CredentialNotFound(credential_id.to_string()));
        }
        insert_audit_event(&transaction, audit)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn insert_audit_event(&self, event: &AuditEvent) -> Result<()> {
        insert_audit_event(&self.connection.lock(), event)
    }

    pub(crate) fn list_audit_events(&self) -> Result<Vec<AuditEvent>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT event_id, occurred_at_ms, credential_id, credential_label, action,
                    resource_type, resource_id, outcome, risk, details_json
             FROM audit_events ORDER BY occurred_at_ms DESC, event_id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (
                event_id,
                occurred_at_ms,
                credential_id,
                credential_label,
                action,
                resource_type,
                resource_id,
                outcome,
                risk,
                details_json,
            ) = row?;
            events.push(AuditEvent {
                event_id: uuid::Uuid::parse_str(&event_id).map_err(|error| {
                    Error::InvalidState(format!("stored audit event id is invalid: {error}"))
                })?,
                occurred_at_ms,
                credential_id: uuid::Uuid::parse_str(&credential_id).map_err(|error| {
                    Error::InvalidState(format!("stored audit credential id is invalid: {error}"))
                })?,
                credential_label,
                action,
                resource_type,
                resource_id,
                outcome: parse_audit_outcome(&outcome)?,
                risk: parse_risk(&risk)?,
                details: serde_json::from_str(&details_json)?,
            });
        }
        Ok(events)
    }
}

type StoredCredentialRow = (
    String,
    String,
    String,
    Vec<u8>,
    Vec<u8>,
    String,
    i64,
    Option<i64>,
    Option<i64>,
);

fn stored_credential_row(row: &Row<'_>) -> rusqlite::Result<StoredCredentialRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn decode_stored_credential(
    stored: Option<StoredCredentialRow>,
) -> Result<Option<StoredCredential>> {
    stored
        .map(
            |(
                id,
                label,
                kind,
                salt,
                digest,
                policy_json,
                created_at_ms,
                expires_at_ms,
                revoked_at_ms,
            )| {
                Ok(StoredCredential {
                    id: uuid::Uuid::parse_str(&id).map_err(|error| {
                        Error::InvalidState(format!("stored credential id is invalid: {error}"))
                    })?,
                    label,
                    kind: parse_credential_kind(&kind)?,
                    salt,
                    digest,
                    policy: serde_json::from_str(&policy_json)?,
                    created_at_ms,
                    expires_at_ms,
                    revoked_at_ms,
                })
            },
        )
        .transpose()
}

fn insert_credential(connection: &Connection, credential: &StoredCredential) -> Result<()> {
    connection.execute(
        "INSERT INTO credentials
           (id, label, kind, salt, digest, policy_json, created_at_ms, expires_at_ms, revoked_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            credential.id.to_string(),
            credential.label,
            credential_kind_name(credential.kind),
            credential.salt,
            credential.digest,
            serde_json::to_string(&credential.policy)?,
            credential.created_at_ms,
            credential.expires_at_ms,
            credential.revoked_at_ms,
        ],
    )?;
    Ok(())
}

fn insert_audit_event(connection: &Connection, event: &AuditEvent) -> Result<()> {
    connection.execute(
        "INSERT INTO audit_events
           (event_id, occurred_at_ms, credential_id, credential_label, action, resource_type,
            resource_id, outcome, risk, details_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            event.event_id.to_string(),
            event.occurred_at_ms,
            event.credential_id.to_string(),
            event.credential_label,
            event.action,
            event.resource_type,
            event.resource_id,
            audit_outcome_name(event.outcome),
            risk_name(event.risk),
            serde_json::to_string(&event.details)?,
        ],
    )?;
    Ok(())
}

fn audit_outcome_name(outcome: AuditOutcome) -> &'static str {
    match outcome {
        AuditOutcome::Succeeded => "SUCCEEDED",
        AuditOutcome::Failed => "FAILED",
        AuditOutcome::Denied => "DENIED",
    }
}

fn parse_audit_outcome(value: &str) -> Result<AuditOutcome> {
    match value {
        "SUCCEEDED" => Ok(AuditOutcome::Succeeded),
        "FAILED" => Ok(AuditOutcome::Failed),
        "DENIED" => Ok(AuditOutcome::Denied),
        _ => Err(Error::InvalidState(format!(
            "stored audit outcome is invalid: {value}"
        ))),
    }
}

fn risk_name(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Low => "LOW",
        RiskLevel::Medium => "MEDIUM",
        RiskLevel::High => "HIGH",
    }
}

fn parse_risk(value: &str) -> Result<RiskLevel> {
    match value {
        "LOW" => Ok(RiskLevel::Low),
        "MEDIUM" => Ok(RiskLevel::Medium),
        "HIGH" => Ok(RiskLevel::High),
        _ => Err(Error::InvalidState(format!(
            "stored audit risk is invalid: {value}"
        ))),
    }
}

fn credential_kind_name(kind: CredentialKind) -> &'static str {
    match kind {
        CredentialKind::Owner => "OWNER",
        CredentialKind::ApiKey => "API_KEY",
    }
}

fn parse_credential_kind(value: &str) -> Result<CredentialKind> {
    match value {
        "OWNER" => Ok(CredentialKind::Owner),
        "API_KEY" => Ok(CredentialKind::ApiKey),
        _ => Err(Error::InvalidState(format!(
            "stored credential kind is invalid: {value}"
        ))),
    }
}

impl InstanceStateStore for SqliteStateStore {
    fn load_instance_states(&self) -> Result<Vec<PersistedInstanceState>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, generation, weight, traffic, health, health_override
             FROM instance_states ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            let traffic: String = row.get(3)?;
            let health: String = row.get(4)?;
            let generation = row.get::<_, i64>(1)?;
            let generation = u64::try_from(generation).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(1, Type::Integer, Box::new(error))
            })?;
            Ok(PersistedInstanceState {
                id: row.get(0)?,
                generation,
                weight: row.get(2)?,
                traffic: parse_traffic(&traffic),
                health: parse_health(&health),
                health_override: row.get(5)?,
            })
        })?;
        let mut states = Vec::new();
        for row in rows {
            let mut state = row?;
            if state.traffic == TrafficState::Draining {
                state.traffic = TrafficState::Drained;
            }
            states.push(state);
        }
        Ok(states)
    }

    fn load_idempotent_result(
        &self,
        key: &str,
        operation: &str,
        instance_id: &str,
    ) -> Result<Option<InstanceState>> {
        let connection = self.connection.lock();
        let stored = connection
            .query_row(
                "SELECT operation, instance_id, result_json
                 FROM idempotent_results WHERE key = ?1",
                [key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored_operation, stored_instance, result_json)) = stored else {
            return Ok(None);
        };
        if stored_operation != operation || stored_instance != instance_id {
            return Err(Error::InvalidState(format!(
                "idempotency key {key} was already used for {stored_operation} on {stored_instance}"
            )));
        }
        Ok(Some(serde_json::from_str(&result_json)?))
    }

    fn commit_instance_operation(
        &self,
        key: &str,
        operation: &str,
        instance_id: &str,
        state: &InstanceState,
    ) -> Result<()> {
        let generation = i64::try_from(state.generation).map_err(|_| {
            Error::InvalidState(format!(
                "generation {} is too large for SQLite",
                state.generation
            ))
        })?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO instance_states
               (id, generation, weight, traffic, health, health_override)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               generation = excluded.generation,
               weight = excluded.weight,
               traffic = excluded.traffic,
               health = excluded.health,
               health_override = excluded.health_override",
            params![
                state.id,
                generation,
                state.weight,
                traffic_name(state.traffic),
                health_name(state.health),
                state.health_override,
            ],
        )?;
        transaction.execute(
            "INSERT INTO idempotent_results (key, operation, instance_id, result_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![key, operation, instance_id, serde_json::to_string(state)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn load_drain_operation_by_key(
        &self,
        key: &str,
        instance_id: &str,
    ) -> Result<Option<DrainOperation>> {
        let connection = self.connection.lock();
        let stored = connection
            .query_row(
                "SELECT operation, instance_id, result_json
                 FROM idempotent_results WHERE key = ?1",
                [key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored_operation, stored_instance, result_json)) = stored else {
            return Ok(None);
        };
        if stored_operation != "begin_drain" || stored_instance != instance_id {
            return Err(Error::InvalidState(format!(
                "idempotency key {key} was already used for {stored_operation} on {stored_instance}"
            )));
        }
        Ok(Some(serde_json::from_str(&result_json)?))
    }

    fn commit_drain_operation(
        &self,
        key: &str,
        instance_id: &str,
        state: &InstanceState,
        operation: &DrainOperation,
    ) -> Result<()> {
        let generation = i64::try_from(state.generation).map_err(|_| {
            Error::InvalidState(format!(
                "generation {} is too large for SQLite",
                state.generation
            ))
        })?;
        let operation_json = serde_json::to_string(operation)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO instance_states
               (id, generation, weight, traffic, health, health_override)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               generation = excluded.generation,
               weight = excluded.weight,
               traffic = excluded.traffic,
               health = excluded.health,
               health_override = excluded.health_override",
            params![
                state.id,
                generation,
                state.weight,
                traffic_name(state.traffic),
                health_name(state.health),
                state.health_override,
            ],
        )?;
        transaction.execute(
            "INSERT INTO drain_operations (operation_id, instance_id, operation_json)
             VALUES (?1, ?2, ?3)",
            params![operation.operation_id, instance_id, operation_json],
        )?;
        transaction.execute(
            "INSERT INTO idempotent_results (key, operation, instance_id, result_json)
             VALUES (?1, 'begin_drain', ?2, ?3)",
            params![key, instance_id, operation_json],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn load_drain_operation(&self, operation_id: &str) -> Result<Option<DrainOperation>> {
        let connection = self.connection.lock();
        let json = connection
            .query_row(
                "SELECT operation_json FROM drain_operations WHERE operation_id = ?1",
                [operation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(Error::from))
            .transpose()
    }

    fn update_drain_operation(&self, operation: &DrainOperation) -> Result<()> {
        let changed = self.connection.lock().execute(
            "UPDATE drain_operations SET operation_json = ?2 WHERE operation_id = ?1",
            params![operation.operation_id, serde_json::to_string(operation)?],
        )?;
        if changed == 0 {
            return Err(Error::DrainOperationNotFound(
                operation.operation_id.clone(),
            ));
        }
        Ok(())
    }
}

impl ConfigStateStore for SqliteStateStore {
    fn latest_config(&self) -> Result<Option<(u64, GatewayConfig)>> {
        let connection = self.connection.lock();
        let stored = connection
            .query_row(
                "SELECT version, config_json
                 FROM config_snapshots ORDER BY version DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        stored
            .map(|(version, json)| {
                let version = u64::try_from(version)
                    .map_err(|_| Error::InvalidState("snapshot version is negative".to_owned()))?;
                Ok((version, serde_json::from_str(&json)?))
            })
            .transpose()
    }

    fn config_at(&self, version: u64) -> Result<Option<GatewayConfig>> {
        let version = sqlite_version(version)?;
        let connection = self.connection.lock();
        let json = connection
            .query_row(
                "SELECT config_json FROM config_snapshots WHERE version = ?1",
                [version],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(Error::from))
            .transpose()
    }

    fn save_config(&self, version: u64, config: &GatewayConfig) -> Result<()> {
        self.connection.lock().execute(
            "INSERT INTO config_snapshots (version, config_json) VALUES (?1, ?2)",
            params![sqlite_version(version)?, serde_json::to_string(config)?],
        )?;
        Ok(())
    }
}

fn sqlite_version(version: u64) -> Result<i64> {
    i64::try_from(version)
        .map_err(|_| Error::InvalidState(format!("snapshot version {version} is too large")))
}

fn traffic_name(state: TrafficState) -> &'static str {
    match state {
        TrafficState::Serving => "SERVING",
        TrafficState::Draining => "DRAINING",
        TrafficState::Drained => "DRAINED",
        TrafficState::Disabled => "DISABLED",
    }
}

fn parse_traffic(value: &str) -> TrafficState {
    match value {
        "DRAINING" => TrafficState::Draining,
        "DRAINED" => TrafficState::Drained,
        "DISABLED" => TrafficState::Disabled,
        _ => TrafficState::Serving,
    }
}

fn health_name(state: HealthState) -> &'static str {
    match state {
        HealthState::Unknown => "UNKNOWN",
        HealthState::Healthy => "HEALTHY",
        HealthState::Unhealthy => "UNHEALTHY",
    }
}

fn parse_health(value: &str) -> HealthState {
    match value {
        "UNKNOWN" => HealthState::Unknown,
        "UNHEALTHY" => HealthState::Unhealthy,
        _ => HealthState::Healthy,
    }
}
