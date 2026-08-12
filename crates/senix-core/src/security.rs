use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::{Error, Result, SqliteStateStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialKind {
    Owner,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ManagementAction {
    #[serde(rename = "instance.read")]
    InstanceRead,
    #[serde(rename = "instance.drain")]
    InstanceDrain,
    #[serde(rename = "instance.rejoin")]
    InstanceRejoin,
    #[serde(rename = "instance.set_weight")]
    InstanceSetWeight,
    #[serde(rename = "instance.disable")]
    InstanceDisable,
    #[serde(rename = "diagnostics.read")]
    DiagnosticsRead,
    #[serde(rename = "change.plan")]
    ChangePlan,
    #[serde(rename = "credential.manage")]
    CredentialManage,
    #[serde(rename = "audit.read")]
    AuditRead,
}

impl ManagementAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InstanceRead => "instance.read",
            Self::InstanceDrain => "instance.drain",
            Self::InstanceRejoin => "instance.rejoin",
            Self::InstanceSetWeight => "instance.set_weight",
            Self::InstanceDisable => "instance.disable",
            Self::DiagnosticsRead => "diagnostics.read",
            Self::ChangePlan => "change.plan",
            Self::CredentialManage => "credential.manage",
            Self::AuditRead => "audit.read",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessPolicy {
    #[serde(default)]
    pub all_resources: bool,
    #[serde(default)]
    pub actions: BTreeSet<ManagementAction>,
    #[serde(default)]
    pub instance_ids: BTreeSet<String>,
}

impl AccessPolicy {
    fn owner() -> Self {
        Self {
            all_resources: true,
            actions: BTreeSet::new(),
            instance_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRef {
    Global,
    Instance(String),
}

impl std::fmt::Display for ResourceRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => formatter.write_str("global"),
            Self::Instance(id) => write!(formatter, "instance/{id}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub credential_id: Uuid,
    pub label: String,
    pub kind: CredentialKind,
    pub policy: AccessPolicy,
}

#[derive(Debug, Clone, Serialize)]
pub struct IssuedApiKey {
    pub credential_id: Uuid,
    pub label: String,
    pub api_key: String,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerAccountSummary {
    pub username: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct IssuedOwnerSession {
    pub username: String,
    pub token: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialSummary {
    pub credential_id: Uuid,
    pub label: String,
    pub kind: CredentialKind,
    pub policy: AccessPolicy,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuditOutcome {
    Succeeded,
    Failed,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub occurred_at_ms: i64,
    pub credential_id: Uuid,
    pub credential_label: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub outcome: AuditOutcome,
    pub risk: RiskLevel,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredCredential {
    pub id: Uuid,
    pub label: String,
    pub kind: CredentialKind,
    pub salt: Vec<u8>,
    pub digest: Vec<u8>,
    pub policy: AccessPolicy,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredOwnerAccount {
    pub username: String,
    pub password_hash: String,
    pub owner_credential_id: Uuid,
    pub session_secret: Vec<u8>,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct OwnerSessionClaims {
    username: String,
    owner_credential_id: Uuid,
    expires_at_ms: i64,
}

#[derive(Debug)]
pub struct SecurityController {
    store: Arc<SqliteStateStore>,
}

impl SecurityController {
    #[must_use]
    pub fn new(store: Arc<SqliteStateStore>) -> Self {
        Self { store }
    }

    /// Creates the only owner credential when no credential exists.
    ///
    /// The returned API key is not recoverable from storage and must be shown only once.
    ///
    /// # Errors
    ///
    /// Returns an error if bootstrap has already completed or storage cannot commit the credential.
    pub fn bootstrap_owner_key(&self, label: &str) -> Result<IssuedApiKey> {
        let label = normalized_label(label)?;
        let (stored, api_key) = new_credential(label, CredentialKind::Owner, AccessPolicy::owner());
        let principal = Principal {
            credential_id: stored.id,
            label: stored.label.clone(),
            kind: stored.kind,
            policy: stored.policy.clone(),
        };
        let audit = audit_event(
            &principal,
            "credential.bootstrap",
            &ResourceRef::Global,
            AuditOutcome::Succeeded,
            RiskLevel::High,
            serde_json::json!({"credential_id": stored.id}),
        );
        self.store.insert_bootstrap_credential(&stored, &audit)?;
        Ok(IssuedApiKey {
            credential_id: stored.id,
            label: stored.label,
            api_key,
            expires_at_ms: stored.expires_at_ms,
        })
    }

    /// Creates the single human owner account linked to the bootstrapped owner credential.
    ///
    /// This operation is intended for the local CLI. Passwords are stored as Argon2id hashes and
    /// are never added to audit details.
    ///
    /// # Errors
    ///
    /// Returns an error when the owner credential is missing, the account already exists, the
    /// username or password is invalid, hashing fails, or storage cannot commit the account.
    pub fn bootstrap_owner_account(
        &self,
        username: &str,
        password: &str,
    ) -> Result<OwnerAccountSummary> {
        let username = normalized_username(username)?;
        validate_password(password)?;
        let owner = self
            .store
            .owner_credential()?
            .ok_or(Error::OwnerCredentialNotInitialized)?;
        let account = StoredOwnerAccount {
            username: username.clone(),
            password_hash: hash_password(password)?,
            owner_credential_id: owner.id,
            session_secret: new_session_secret(),
            created_at_ms: now_ms(),
        };
        let principal = Principal {
            credential_id: owner.id,
            label: username.clone(),
            kind: CredentialKind::Owner,
            policy: owner.policy,
        };
        let audit = audit_event(
            &principal,
            "owner.bootstrap",
            &ResourceRef::Global,
            AuditOutcome::Succeeded,
            RiskLevel::High,
            serde_json::json!({"username": username}),
        );
        self.store
            .insert_owner_account_with_audit(&account, &audit)?;
        Ok(OwnerAccountSummary {
            username: account.username,
            created_at_ms: account.created_at_ms,
        })
    }

    /// Replaces the owner password from the local recovery CLI and invalidates browser sessions.
    ///
    /// # Errors
    ///
    /// Returns an error when the account is missing, the new password is invalid, hashing fails,
    /// or storage cannot atomically update the password and audit record.
    pub fn reset_owner_password(&self, password: &str) -> Result<OwnerAccountSummary> {
        validate_password(password)?;
        let account = self
            .store
            .owner_account()?
            .ok_or(Error::OwnerAccountNotInitialized)?;
        let principal = Principal {
            credential_id: account.owner_credential_id,
            label: account.username.clone(),
            kind: CredentialKind::Owner,
            policy: AccessPolicy::owner(),
        };
        let audit = audit_event(
            &principal,
            "owner.password_reset",
            &ResourceRef::Global,
            AuditOutcome::Succeeded,
            RiskLevel::High,
            serde_json::json!({"scope": "local_recovery"}),
        );
        self.store.reset_owner_password_with_audit(
            &hash_password(password)?,
            &new_session_secret(),
            &audit,
        )?;
        Ok(OwnerAccountSummary {
            username: account.username,
            created_at_ms: account.created_at_ms,
        })
    }

    /// Verifies the owner password and issues a short-lived signed browser session.
    ///
    /// # Errors
    ///
    /// Returns a generic login error for an incorrect username or password. Storage, corrupted
    /// password hashes, invalid TTLs, and audit failures are reported separately.
    pub fn login_owner(
        &self,
        username: &str,
        password: &str,
        ttl_ms: i64,
    ) -> Result<IssuedOwnerSession> {
        if ttl_ms <= 0 {
            return Err(Error::InvalidState(
                "owner session TTL must be positive".to_owned(),
            ));
        }
        let account = self
            .store
            .owner_account()?
            .ok_or(Error::OwnerAccountNotInitialized)?;
        let parsed_hash = PasswordHash::new(&account.password_hash)
            .map_err(|error| Error::Crypto(error.to_string()))?;
        let password_matches = Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok();
        let principal = Principal {
            credential_id: account.owner_credential_id,
            label: account.username.clone(),
            kind: CredentialKind::Owner,
            policy: AccessPolicy::owner(),
        };
        if username != account.username || !password_matches {
            self.record_action(
                &principal,
                "owner.login",
                &ResourceRef::Global,
                AuditOutcome::Denied,
                RiskLevel::High,
                serde_json::json!({}),
            )?;
            return Err(Error::InvalidOwnerLogin);
        }
        let expires_at_ms = now_ms().saturating_add(ttl_ms);
        let claims = OwnerSessionClaims {
            username: account.username.clone(),
            owner_credential_id: account.owner_credential_id,
            expires_at_ms,
        };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
        let signature = sign_session(&account.session_secret, payload.as_bytes())?;
        self.record_action(
            &principal,
            "owner.login",
            &ResourceRef::Global,
            AuditOutcome::Succeeded,
            RiskLevel::Low,
            serde_json::json!({"expires_at_ms": expires_at_ms}),
        )?;
        Ok(IssuedOwnerSession {
            username: account.username,
            token: format!("snxs_{payload}.{}", URL_SAFE_NO_PAD.encode(signature)),
            expires_at_ms,
        })
    }

    /// Authenticates a signed owner browser session without creating server-side session state.
    ///
    /// # Errors
    ///
    /// Returns an authentication error when the token is malformed, has an invalid signature, no
    /// longer matches the owner account, or has expired.
    pub fn authenticate_owner_session(&self, token: &str) -> Result<Principal> {
        let account = self
            .store
            .owner_account()?
            .ok_or(Error::OwnerAccountNotInitialized)?;
        let encoded = token
            .strip_prefix("snxs_")
            .ok_or(Error::InvalidOwnerSession)?;
        let (payload, signature) = encoded.split_once('.').ok_or(Error::InvalidOwnerSession)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| Error::InvalidOwnerSession)?;
        verify_session_signature(&account.session_secret, payload.as_bytes(), &signature)?;
        let claims: OwnerSessionClaims = URL_SAFE_NO_PAD
            .decode(payload)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or(Error::InvalidOwnerSession)?;
        if claims.username != account.username
            || claims.owner_credential_id != account.owner_credential_id
        {
            return Err(Error::InvalidOwnerSession);
        }
        if claims.expires_at_ms <= now_ms() {
            return Err(Error::OwnerSessionExpired);
        }
        Ok(Principal {
            credential_id: account.owner_credential_id,
            label: account.username,
            kind: CredentialKind::Owner,
            policy: AccessPolicy::owner(),
        })
    }

    /// Invalidates every signed owner browser session without storing a session list.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not the owner or storage cannot atomically rotate the
    /// signing secret and append the logout audit event.
    pub fn logout_owner(&self, actor: &Principal) -> Result<()> {
        self.require_owner(actor, "owner.logout")?;
        let audit = audit_event(
            actor,
            "owner.logout",
            &ResourceRef::Global,
            AuditOutcome::Succeeded,
            RiskLevel::Low,
            serde_json::json!({"scope": "all_browser_sessions"}),
        );
        self.store
            .rotate_owner_session_secret_with_audit(&new_session_secret(), &audit)
    }

    /// Issues a restricted API key. Only the owner may manage credentials.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not the owner, the policy is empty or unsafe, the expiry
    /// is not in the future, or persistence fails.
    pub fn issue_key(
        &self,
        actor: &Principal,
        label: &str,
        policy: AccessPolicy,
        expires_at_ms: Option<i64>,
    ) -> Result<IssuedApiKey> {
        self.require_owner(actor, "credential.issue")?;
        validate_policy(&policy)?;
        if expires_at_ms.is_some_and(|expires_at| expires_at <= now_ms()) {
            return Err(Error::InvalidState(
                "credential expiry must be in the future".to_owned(),
            ));
        }
        let (mut stored, api_key) =
            new_credential(normalized_label(label)?, CredentialKind::ApiKey, policy);
        stored.expires_at_ms = expires_at_ms;
        let audit = audit_event(
            actor,
            "credential.issue",
            &ResourceRef::Global,
            AuditOutcome::Succeeded,
            RiskLevel::High,
            serde_json::json!({"credential_id": stored.id, "label": stored.label.clone()}),
        );
        self.store.insert_credential_with_audit(&stored, &audit)?;
        Ok(IssuedApiKey {
            credential_id: stored.id,
            label: stored.label,
            api_key,
            expires_at_ms: stored.expires_at_ms,
        })
    }

    /// Lists credential metadata without returning any bearer secret or digest.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not the owner or persistence cannot be read.
    pub fn list_credentials(&self, actor: &Principal) -> Result<Vec<CredentialSummary>> {
        self.require_owner(actor, "credential.list")?;
        self.store.list_credentials()
    }

    /// Revokes a non-owner credential immediately.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not the owner, the target is unknown or is the owner, or
    /// persistence fails.
    pub fn revoke(&self, actor: &Principal, credential_id: Uuid) -> Result<()> {
        self.require_owner(actor, "credential.revoke")?;
        let target = self
            .store
            .credential(credential_id)?
            .ok_or_else(|| Error::CredentialNotFound(credential_id.to_string()))?;
        if target.kind == CredentialKind::Owner {
            return Err(Error::InvalidState(
                "the owner credential cannot be revoked".to_owned(),
            ));
        }
        let audit = audit_event(
            actor,
            "credential.revoke",
            &ResourceRef::Global,
            AuditOutcome::Succeeded,
            RiskLevel::High,
            serde_json::json!({"credential_id": credential_id}),
        );
        self.store
            .revoke_credential_with_audit(credential_id, now_ms(), &audit)
    }

    /// Lists immutable audit events in newest-first order.
    ///
    /// # Errors
    ///
    /// Returns an error when authorization is denied or storage cannot be read.
    pub fn list_audit(&self, actor: &Principal) -> Result<Vec<AuditEvent>> {
        self.authorize(actor, ManagementAction::AuditRead, &ResourceRef::Global)?;
        self.store.list_audit_events()
    }

    /// Authenticates a raw bearer API key and returns its management principal.
    ///
    /// # Errors
    ///
    /// Returns an authentication error when the key is malformed, unknown, expired, revoked, or
    /// does not match the stored digest.
    pub fn authenticate(&self, api_key: &str) -> Result<Principal> {
        let (id, secret) = parse_api_key(api_key)?;
        let stored = self.store.credential(id)?.ok_or(Error::InvalidCredential)?;
        if stored.revoked_at_ms.is_some() {
            return Err(Error::CredentialRevoked);
        }
        if stored
            .expires_at_ms
            .is_some_and(|expires_at| expires_at <= now_ms())
        {
            return Err(Error::CredentialExpired);
        }
        let candidate = credential_digest(&stored.salt, secret.as_bytes());
        if candidate
            .as_slice()
            .ct_eq(stored.digest.as_slice())
            .unwrap_u8()
            != 1
        {
            return Err(Error::InvalidCredential);
        }
        Ok(Principal {
            credential_id: stored.id,
            label: stored.label,
            kind: stored.kind,
            policy: stored.policy,
        })
    }

    /// Checks one task-level management permission.
    ///
    /// # Errors
    ///
    /// Returns `Forbidden` when the action or resource is outside the principal's grants.
    pub fn authorize(
        &self,
        principal: &Principal,
        action: ManagementAction,
        resource: &ResourceRef,
    ) -> Result<()> {
        if self.allows(principal, action, resource) {
            return Ok(());
        }
        self.record_action(
            principal,
            action.as_str(),
            resource,
            AuditOutcome::Denied,
            action_risk(action),
            serde_json::json!({}),
        )?;
        Err(Error::Forbidden {
            action: action.as_str().to_owned(),
            resource: resource.to_string(),
        })
    }

    /// Returns whether a principal may perform an action without emitting an audit event.
    ///
    /// Adapters may use this only to filter discovery/list results. Executing an action must call
    /// [`Self::authorize`] so denials remain auditable.
    #[must_use]
    pub fn allows(
        &self,
        principal: &Principal,
        action: ManagementAction,
        resource: &ResourceRef,
    ) -> bool {
        if principal.kind == CredentialKind::Owner {
            return true;
        }
        let action_allowed = principal.policy.actions.contains(&action);
        let resource_allowed = match resource {
            ResourceRef::Global => principal.policy.all_resources,
            ResourceRef::Instance(id) => {
                principal.policy.all_resources || principal.policy.instance_ids.contains(id)
            }
        };
        action_allowed && resource_allowed
    }

    /// Appends an audit event containing only explicitly supplied, non-secret details.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be persisted.
    pub fn record_action(
        &self,
        actor: &Principal,
        action: &str,
        resource: &ResourceRef,
        outcome: AuditOutcome,
        risk: RiskLevel,
        details: serde_json::Value,
    ) -> Result<()> {
        self.store.insert_audit_event(&audit_event(
            actor, action, resource, outcome, risk, details,
        ))
    }

    fn require_owner(&self, actor: &Principal, action: &str) -> Result<()> {
        if actor.kind == CredentialKind::Owner {
            return Ok(());
        }
        self.record_action(
            actor,
            action,
            &ResourceRef::Global,
            AuditOutcome::Denied,
            RiskLevel::High,
            serde_json::json!({}),
        )?;
        Err(Error::Forbidden {
            action: action.to_owned(),
            resource: ResourceRef::Global.to_string(),
        })
    }
}

fn audit_event(
    actor: &Principal,
    action: &str,
    resource: &ResourceRef,
    outcome: AuditOutcome,
    risk: RiskLevel,
    details: serde_json::Value,
) -> AuditEvent {
    let (resource_type, resource_id) = match resource {
        ResourceRef::Global => ("GLOBAL".to_owned(), None),
        ResourceRef::Instance(id) => ("INSTANCE".to_owned(), Some(id.clone())),
    };
    AuditEvent {
        event_id: Uuid::new_v4(),
        occurred_at_ms: now_ms(),
        credential_id: actor.credential_id,
        credential_label: actor.label.clone(),
        action: action.to_owned(),
        resource_type,
        resource_id,
        outcome,
        risk,
        details,
    }
}

fn new_credential(
    label: String,
    kind: CredentialKind,
    policy: AccessPolicy,
) -> (StoredCredential, String) {
    let id = Uuid::new_v4();
    let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let salt = Uuid::new_v4().as_bytes().to_vec();
    let digest = credential_digest(&salt, secret.as_bytes());
    (
        StoredCredential {
            id,
            label,
            kind,
            salt,
            digest,
            policy,
            created_at_ms: now_ms(),
            expires_at_ms: None,
            revoked_at_ms: None,
        },
        format!("snx_{id}_{secret}"),
    )
}

fn parse_api_key(api_key: &str) -> Result<(Uuid, &str)> {
    let mut parts = api_key.splitn(3, '_');
    if parts.next() != Some("snx") {
        return Err(Error::InvalidCredential);
    }
    let id = parts
        .next()
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or(Error::InvalidCredential)?;
    let secret = parts
        .next()
        .filter(|value| value.len() == 64)
        .ok_or(Error::InvalidCredential)?;
    Ok((id, secret))
}

fn credential_digest(salt: &[u8], secret: &[u8]) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(salt);
    digest.update(secret);
    digest.finalize().to_vec()
}

fn sign_session(secret: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|error| Error::Crypto(error.to_string()))?;
    mac.update(payload);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn new_session_secret() -> Vec<u8> {
    let mut secret = Vec::with_capacity(32);
    secret.extend_from_slice(Uuid::new_v4().as_bytes());
    secret.extend_from_slice(Uuid::new_v4().as_bytes());
    secret
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes())
        .map_err(|error| Error::Crypto(error.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| Error::Crypto(error.to_string()))
}

fn verify_session_signature(secret: &[u8], payload: &[u8], signature: &[u8]) -> Result<()> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|error| Error::Crypto(error.to_string()))?;
    mac.update(payload);
    mac.verify_slice(signature)
        .map_err(|_| Error::InvalidOwnerSession)
}

fn normalized_username(username: &str) -> Result<String> {
    let username = username.trim();
    if !(3..=64).contains(&username.len())
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(Error::InvalidState(
            "owner username must contain 3 to 64 letters, numbers, dots, underscores or hyphens"
                .to_owned(),
        ));
    }
    Ok(username.to_owned())
}

fn validate_password(password: &str) -> Result<()> {
    if !(12..=256).contains(&password.chars().count()) {
        return Err(Error::InvalidState(
            "owner password must contain 12 to 256 characters".to_owned(),
        ));
    }
    Ok(())
}

fn normalized_label(label: &str) -> Result<String> {
    let label = label.trim();
    if label.is_empty() || label.len() > 80 {
        return Err(Error::InvalidState(
            "credential label must contain 1 to 80 characters".to_owned(),
        ));
    }
    Ok(label.to_owned())
}

fn validate_policy(policy: &AccessPolicy) -> Result<()> {
    if policy.actions.is_empty() {
        return Err(Error::InvalidState(
            "API key must grant at least one management action".to_owned(),
        ));
    }
    if policy.actions.contains(&ManagementAction::CredentialManage) {
        return Err(Error::InvalidState(
            "API keys cannot manage credentials".to_owned(),
        ));
    }
    if !policy.all_resources && policy.instance_ids.is_empty() {
        return Err(Error::InvalidState(
            "API key must target all resources or at least one instance".to_owned(),
        ));
    }
    Ok(())
}

const fn action_risk(action: ManagementAction) -> RiskLevel {
    match action {
        ManagementAction::InstanceDrain
        | ManagementAction::InstanceRejoin
        | ManagementAction::InstanceDisable
        | ManagementAction::ChangePlan
        | ManagementAction::CredentialManage => RiskLevel::High,
        ManagementAction::InstanceSetWeight => RiskLevel::Medium,
        ManagementAction::InstanceRead
        | ManagementAction::DiagnosticsRead
        | ManagementAction::AuditRead => RiskLevel::Low,
    }
}

fn now_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch");
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}
