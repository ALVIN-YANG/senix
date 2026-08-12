use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("management credential is required")]
    AuthenticationRequired,

    #[error("management credential is invalid")]
    InvalidCredential,

    #[error("management credential has expired")]
    CredentialExpired,

    #[error("management credential has been revoked")]
    CredentialRevoked,

    #[error("management credential bootstrap has already completed")]
    CredentialAlreadyInitialized,

    #[error("owner account bootstrap has already completed")]
    OwnerAccountAlreadyInitialized,

    #[error("owner credential has not been initialized")]
    OwnerCredentialNotInitialized,

    #[error("owner account has not been initialized")]
    OwnerAccountNotInitialized,

    #[error("owner username or password is invalid")]
    InvalidOwnerLogin,

    #[error("owner management session is invalid")]
    InvalidOwnerSession,

    #[error("owner management session has expired")]
    OwnerSessionExpired,

    #[error("credential not found: {0}")]
    CredentialNotFound(String),

    #[error("credential cannot perform {action} on {resource}")]
    Forbidden { action: String, resource: String },

    #[error("instance not found: {0}")]
    InstanceNotFound(String),

    #[error("no route matched host={host} path={path}")]
    RouteNotFound { host: String, path: String },

    #[error("route matched but has no serving healthy backend: {0}")]
    NoAvailableBackend(String),

    #[error(
        "draining instance {instance_id} would remove the last available backend from route {route_id}"
    )]
    LastAvailableBackend {
        instance_id: String,
        route_id: String,
    },

    #[error("drain operation not found: {0}")]
    DrainOperationNotFound(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("configuration has validation issues")]
    InvalidConfig,

    #[error("configuration changed since this plan was created")]
    StalePlan,

    #[error("configuration snapshot not found: {0}")]
    SnapshotNotFound(u64),

    #[error("state store error: {0}")]
    Store(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("cryptographic operation failed: {0}")]
    Crypto(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AuthenticationRequired => "AUTHENTICATION_REQUIRED",
            Self::InvalidCredential => "INVALID_CREDENTIAL",
            Self::CredentialExpired => "CREDENTIAL_EXPIRED",
            Self::CredentialRevoked => "CREDENTIAL_REVOKED",
            Self::CredentialAlreadyInitialized => "CREDENTIAL_ALREADY_INITIALIZED",
            Self::OwnerAccountAlreadyInitialized => "OWNER_ACCOUNT_ALREADY_INITIALIZED",
            Self::OwnerCredentialNotInitialized => "OWNER_CREDENTIAL_NOT_INITIALIZED",
            Self::OwnerAccountNotInitialized => "OWNER_ACCOUNT_NOT_INITIALIZED",
            Self::InvalidOwnerLogin => "INVALID_OWNER_LOGIN",
            Self::InvalidOwnerSession => "INVALID_OWNER_SESSION",
            Self::OwnerSessionExpired => "OWNER_SESSION_EXPIRED",
            Self::CredentialNotFound(_) => "CREDENTIAL_NOT_FOUND",
            Self::Forbidden { .. } => "FORBIDDEN",
            Self::InstanceNotFound(_) => "INSTANCE_NOT_FOUND",
            Self::RouteNotFound { .. } => "ROUTE_NOT_FOUND",
            Self::NoAvailableBackend(_) => "NO_AVAILABLE_BACKEND",
            Self::LastAvailableBackend { .. } => "LAST_AVAILABLE_BACKEND",
            Self::DrainOperationNotFound(_) => "DRAIN_OPERATION_NOT_FOUND",
            Self::InvalidState(_) => "INVALID_STATE",
            Self::InvalidConfig => "INVALID_CONFIG",
            Self::StalePlan => "STALE_PLAN",
            Self::SnapshotNotFound(_) => "SNAPSHOT_NOT_FOUND",
            Self::Store(_) | Self::Serialization(_) | Self::Crypto(_) => "INTERNAL_ERROR",
        }
    }

    #[must_use]
    pub fn evidence(&self) -> serde_json::Value {
        match self {
            Self::CredentialNotFound(credential_id) => {
                serde_json::json!({"credential_id": credential_id})
            }
            Self::Forbidden { action, resource } => {
                serde_json::json!({"action": action, "resource": resource})
            }
            Self::InstanceNotFound(instance_id) => {
                serde_json::json!({"instance_id": instance_id})
            }
            Self::RouteNotFound { host, path } => {
                serde_json::json!({"host": host, "path": path})
            }
            Self::NoAvailableBackend(route_id) => serde_json::json!({"route_id": route_id}),
            Self::LastAvailableBackend {
                instance_id,
                route_id,
            } => serde_json::json!({"instance_id": instance_id, "route_id": route_id}),
            Self::DrainOperationNotFound(operation_id) => {
                serde_json::json!({"operation_id": operation_id})
            }
            Self::SnapshotNotFound(version) => serde_json::json!({"version": version}),
            _ => serde_json::json!({}),
        }
    }

    #[must_use]
    pub const fn is_internal(&self) -> bool {
        matches!(
            self,
            Self::Store(_) | Self::Serialization(_) | Self::Crypto(_)
        )
    }

    #[must_use]
    pub fn public_message(&self) -> String {
        if self.is_internal() {
            "internal state error".to_owned()
        } else {
            self.to_string()
        }
    }
}
