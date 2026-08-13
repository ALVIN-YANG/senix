//! Core domain modules for the Senix gateway.

mod certificate;
mod config;
mod diagnostic;
mod error;
mod runtime;
mod security;
mod state;
mod store;
mod traffic;

pub use certificate::{
    CertificateController, CertificateMaterial, CertificateSummary, Http01ChallengeGuard,
    Http01ChallengeRegistry, ManagedCertificate, SecretBytes, SecretVault,
};
pub use config::{
    AppliedChange, ChangeActor, ChangeKind, ChangePlan, ChangeStatus, ConfigDiff, ConfigEngine,
    ConfigIssue, ConfigSnapshot,
};
pub use diagnostic::{DiagnosticEngine, DiagnosticOutcome, DiagnosticReport, DiagnosticStep};
pub use error::{Error, Result};
pub use runtime::{GatewayRuntime, HealthTarget, RequestLease};
pub use security::{
    AccessPolicy, AuditEvent, AuditOutcome, CredentialKind, CredentialSummary, IssuedApiKey,
    IssuedOwnerSession, ManagementAction, OwnerAccountSummary, Principal, ResourceRef, RiskLevel,
    SecurityController,
};
pub use state::{
    BackendConfig, GatewayConfig, HealthCheckConfig, HealthCheckProtocol, HealthState,
    InstanceState, PersistedInstanceState, RouteConfig, TrafficState,
};
pub use store::{ConfigStateStore, InstanceStateStore, SqliteStateStore};
pub use traffic::{DrainOperation, DrainOperationStatus, DrainOptions, TrafficController};
