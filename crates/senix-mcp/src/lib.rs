//! MCP Adapter for the Senix control-plane Modules.

use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use http::request::Parts;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Extension, wrapper::Parameters},
    model::{
        CacheScope, CallToolResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ResultType,
    },
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use senix_core::{
    AuditOutcome, CertificateSummary, ConfigEngine, DiagnosticEngine, DrainOptions, Error,
    GatewayConfig, ManagementAction, Principal, ResourceRef, Result as SenixResult, RiskLevel,
    SecurityController, TrafficController,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone)]
pub struct McpModules {
    pub traffic: Arc<TrafficController>,
    pub config: Arc<ConfigEngine>,
    pub diagnostics: DiagnosticEngine,
    pub security: Arc<SecurityController>,
    pub certificates: Option<Arc<dyn CertificateManagement>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CertificateIssueResult {
    pub certificate: CertificateSummary,
    pub tls_generation: u64,
}

#[derive(Clone, Debug)]
pub struct CertificateToolError {
    code: &'static str,
    message: String,
}

impl CertificateToolError {
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: "ACME_DISABLED",
            message: message.into(),
        }
    }

    #[must_use]
    pub fn issuance_failed() -> Self {
        Self {
            code: "CERTIFICATE_ISSUANCE_FAILED",
            message: "certificate issuance failed; inspect the audit log and server diagnostics"
                .to_owned(),
        }
    }
}

#[async_trait]
pub trait CertificateManagement: Send + Sync + fmt::Debug {
    /// Lists secret-free certificate lifecycle metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the durable certificate store cannot be read.
    fn list(&self) -> SenixResult<Vec<CertificateSummary>>;

    /// Runs one bounded HTTP-01 issuance and activates the resulting certificate.
    ///
    /// # Errors
    ///
    /// Returns a public-safe error when ACME is disabled or issuance fails.
    async fn issue(
        &self,
        domains: Vec<String>,
        timeout: Duration,
    ) -> std::result::Result<CertificateIssueResult, CertificateToolError>;
}

#[derive(Clone, Debug, Default)]
pub struct McpHttpOptions {
    /// Extra Host header values accepted in addition to localhost and loopback addresses.
    pub allowed_hosts: Vec<String>,
    /// Browser Origin values accepted by the MCP endpoint. Non-browser clients usually omit it.
    pub allowed_origins: Vec<String>,
}

#[derive(Clone)]
pub struct SenixMcp {
    modules: McpModules,
    tool_router: ToolRouter<Self>,
}

impl SenixMcp {
    #[must_use]
    pub fn new(modules: McpModules) -> Self {
        Self {
            modules,
            tool_router: Self::tool_router(),
        }
    }

    fn tool_visible(&self, principal: &Principal, name: &str) -> bool {
        let action = match name {
            "list_instances" | "get_instance_health" | "get_drain_status" => {
                ManagementAction::InstanceRead
            }
            "drain_instance" => ManagementAction::InstanceDrain,
            "rejoin_instance" => ManagementAction::InstanceRejoin,
            "set_instance_weight" => ManagementAction::InstanceSetWeight,
            "disable_instance" => ManagementAction::InstanceDisable,
            "diagnose_request" => {
                return self.modules.security.allows(
                    principal,
                    ManagementAction::DiagnosticsRead,
                    &ResourceRef::Global,
                );
            }
            "plan_change" | "plan_rollback" => {
                return self.modules.security.allows(
                    principal,
                    ManagementAction::ChangePlan,
                    &ResourceRef::Global,
                );
            }
            "list_changes" | "get_change" => {
                return self.modules.security.allows(
                    principal,
                    ManagementAction::ChangeRead,
                    &ResourceRef::Global,
                );
            }
            "apply_approved_change" => {
                return self.modules.security.allows(
                    principal,
                    ManagementAction::ChangeApply,
                    &ResourceRef::Global,
                );
            }
            "list_audit_events" => {
                return self.modules.security.allows(
                    principal,
                    ManagementAction::AuditRead,
                    &ResourceRef::Global,
                );
            }
            "list_certificates" => {
                return self.modules.certificates.is_some()
                    && self.modules.security.allows(
                        principal,
                        ManagementAction::CertificateRead,
                        &ResourceRef::Global,
                    );
            }
            "issue_certificate" => {
                return self.modules.certificates.is_some()
                    && self.modules.security.allows(
                        principal,
                        ManagementAction::CertificateIssue,
                        &ResourceRef::Global,
                    );
            }
            _ => return false,
        };
        self.modules
            .traffic
            .list_instances()
            .iter()
            .any(|instance| {
                self.modules.security.allows(
                    principal,
                    action,
                    &ResourceRef::Instance(instance.id.clone()),
                )
            })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InstanceParams {
    /// Stable Senix instance identifier, not an IP address.
    instance_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OperationParams {
    /// Operation identifier returned by `drain_instance`.
    operation_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ChangeParams {
    /// Stable Change Plan identifier returned by `plan_change`.
    change_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SnapshotParams {
    /// Immutable Snapshot version to restore through the normal approval chain.
    target_version: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DrainParams {
    /// Stable Senix instance identifier.
    instance_id: String,
    /// Maximum time to wait for ordinary in-flight requests.
    #[serde(default = "default_drain_timeout_ms")]
    timeout_ms: u64,
    /// Explicitly bypass last-backend protection. Existing connections are never terminated.
    #[serde(default)]
    force: bool,
    /// Caller-generated key used to make retries return the same operation.
    idempotency_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RejoinParams {
    instance_id: String,
    /// New deployment generation; must be greater than the previous generation.
    generation: u64,
    weight: u32,
    #[serde(default)]
    force: bool,
    idempotency_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WeightParams {
    instance_id: String,
    weight: u32,
    idempotency_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DisableParams {
    instance_id: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DiagnoseParams {
    host: String,
    path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PlanChangeParams {
    /// Complete candidate `GatewayConfig` JSON object.
    candidate: serde_json::Value,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IssueCertificateParams {
    /// Non-wildcard DNS names that already resolve to this gateway's HTTP listener.
    domains: Vec<String>,
    /// Maximum ACME validation time, between 10 and 300 seconds.
    #[serde(default = "default_acme_timeout_seconds")]
    timeout_seconds: u64,
}

#[tool_router(router = tool_router)]
impl SenixMcp {
    #[tool(description = "List only the gateway instances visible to this Credential.")]
    async fn list_instances(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            Ok(self
                .modules
                .traffic
                .list_instances()
                .into_iter()
                .filter(|instance| {
                    self.modules.security.allows(
                        &principal,
                        ManagementAction::InstanceRead,
                        &ResourceRef::Instance(instance.id.clone()),
                    )
                })
                .collect::<Vec<_>>())
        })();
        domain_result(result)
    }

    #[tool(
        description = "Get traffic, health, generation, weight and in-flight state for one instance."
    )]
    async fn get_instance_health(
        &self,
        Parameters(input): Parameters<InstanceParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::InstanceRead,
                &ResourceRef::Instance(input.instance_id.clone()),
            )?;
            self.modules.traffic.status(&input.instance_id)
        })();
        domain_result(result)
    }

    #[tool(
        description = "Stop assigning new requests to an instance and start a bounded drain operation."
    )]
    async fn drain_instance(
        &self,
        Parameters(input): Parameters<DrainParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            let resource = ResourceRef::Instance(input.instance_id.clone());
            self.modules.security.authorize(
                &principal,
                ManagementAction::InstanceDrain,
                &resource,
            )?;
            let operation = self.modules.traffic.begin_drain(
                &input.instance_id,
                DrainOptions {
                    force: input.force,
                    timeout_ms: input.timeout_ms,
                },
                &input.idempotency_key,
            )?;
            self.modules.security.record_action(
                &principal,
                ManagementAction::InstanceDrain.as_str(),
                &resource,
                AuditOutcome::Succeeded,
                if input.force {
                    RiskLevel::High
                } else {
                    RiskLevel::Medium
                },
                json!({"operation_id": operation.operation_id, "force": input.force}),
            )?;
            Ok(operation)
        })();
        domain_result(result)
    }

    #[tool(description = "Read the latest durable state of a drain operation.")]
    async fn get_drain_status(
        &self,
        Parameters(input): Parameters<OperationParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            let operation = self.modules.traffic.drain_status(&input.operation_id)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::InstanceRead,
                &ResourceRef::Instance(operation.instance_id.clone()),
            )?;
            Ok(operation)
        })();
        domain_result(result)
    }

    #[tool(
        description = "Return a fully drained instance as a strictly newer deployment generation."
    )]
    async fn rejoin_instance(
        &self,
        Parameters(input): Parameters<RejoinParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            let resource = ResourceRef::Instance(input.instance_id.clone());
            self.modules.security.authorize(
                &principal,
                ManagementAction::InstanceRejoin,
                &resource,
            )?;
            let instance = self.modules.traffic.rejoin(
                &input.instance_id,
                input.generation,
                input.weight,
                input.force,
                &input.idempotency_key,
            )?;
            self.modules.security.record_action(
                &principal,
                ManagementAction::InstanceRejoin.as_str(),
                &resource,
                AuditOutcome::Succeeded,
                if input.force {
                    RiskLevel::High
                } else {
                    RiskLevel::Medium
                },
                json!({"generation": input.generation, "force": input.force}),
            )?;
            Ok(instance)
        })();
        domain_result(result)
    }

    #[tool(description = "Set the share of new requests assigned to a serving instance.")]
    async fn set_instance_weight(
        &self,
        Parameters(input): Parameters<WeightParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            let resource = ResourceRef::Instance(input.instance_id.clone());
            self.modules.security.authorize(
                &principal,
                ManagementAction::InstanceSetWeight,
                &resource,
            )?;
            let instance = self.modules.traffic.set_weight(
                &input.instance_id,
                input.weight,
                &input.idempotency_key,
            )?;
            self.modules.security.record_action(
                &principal,
                ManagementAction::InstanceSetWeight.as_str(),
                &resource,
                AuditOutcome::Succeeded,
                RiskLevel::Medium,
                json!({"weight": input.weight}),
            )?;
            Ok(instance)
        })();
        domain_result(result)
    }

    #[tool(description = "Keep an instance out of selection until an explicit rejoin.")]
    async fn disable_instance(
        &self,
        Parameters(input): Parameters<DisableParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            let resource = ResourceRef::Instance(input.instance_id.clone());
            self.modules.security.authorize(
                &principal,
                ManagementAction::InstanceDisable,
                &resource,
            )?;
            let instance = self
                .modules
                .traffic
                .disable(&input.instance_id, &input.idempotency_key)?;
            self.modules.security.record_action(
                &principal,
                ManagementAction::InstanceDisable.as_str(),
                &resource,
                AuditOutcome::Succeeded,
                RiskLevel::High,
                json!({}),
            )?;
            Ok(instance)
        })();
        domain_result(result)
    }

    #[tool(
        description = "Explain route matching and backend eligibility using current runtime evidence."
    )]
    async fn diagnose_request(
        &self,
        Parameters(input): Parameters<DiagnoseParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::DiagnosticsRead,
                &ResourceRef::Global,
            )?;
            Ok(self.modules.diagnostics.diagnose(&input.host, &input.path))
        })();
        domain_result(result)
    }

    #[tool(
        description = "Persist a complete candidate as an immutable, version-bound Change Plan without applying it."
    )]
    async fn plan_change(
        &self,
        Parameters(input): Parameters<PlanChangeParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::ChangePlan,
                &ResourceRef::Global,
            )?;
            let candidate: GatewayConfig =
                serde_json::from_value(input.candidate).map_err(|error| {
                    Error::InvalidState(format!("candidate config is invalid: {error}"))
                })?;
            let plan = self.modules.config.plan(candidate, &principal)?;
            self.modules.security.record_action(
                &principal,
                ManagementAction::ChangePlan.as_str(),
                &ResourceRef::Global,
                AuditOutcome::Succeeded,
                RiskLevel::Low,
                json!({
                    "change_id": plan.change_id,
                    "candidate_digest": plan.candidate_digest,
                    "issue_count": plan.issues.len()
                }),
            )?;
            Ok(plan)
        })();
        domain_result(result)
    }

    #[tool(
        description = "Create an immutable rollback Change Plan from a historical Snapshot. It still requires Owner approval before apply."
    )]
    async fn plan_rollback(
        &self,
        Parameters(input): Parameters<SnapshotParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::ChangePlan,
                &ResourceRef::Global,
            )?;
            let plan = self
                .modules
                .config
                .plan_rollback(input.target_version, &principal)?;
            self.modules.security.record_action(
                &principal,
                "change.plan_rollback",
                &ResourceRef::Global,
                AuditOutcome::Succeeded,
                RiskLevel::Low,
                json!({
                    "change_id": plan.change_id,
                    "target_version": input.target_version
                }),
            )?;
            Ok(plan)
        })();
        domain_result(result)
    }

    #[tool(description = "List durable Change Plans newest first.")]
    async fn list_changes(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::ChangeRead,
                &ResourceRef::Global,
            )?;
            self.modules.config.list_changes()
        })();
        domain_result(result)
    }

    #[tool(description = "Read one durable Change Plan including approval and apply status.")]
    async fn get_change(
        &self,
        Parameters(input): Parameters<ChangeParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::ChangeRead,
                &ResourceRef::Global,
            )?;
            let change_id = uuid::Uuid::parse_str(&input.change_id)
                .map_err(|_| Error::ChangeNotFound(input.change_id.clone()))?;
            self.modules
                .config
                .change(change_id)?
                .ok_or(Error::ChangeNotFound(input.change_id))
        })();
        domain_result(result)
    }

    #[tool(
        description = "Apply an exact Change Plan that the Owner already approved. This tool cannot approve or modify the plan."
    )]
    async fn apply_approved_change(
        &self,
        Parameters(input): Parameters<ChangeParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::ChangeApply,
                &ResourceRef::Global,
            )?;
            let change_id = uuid::Uuid::parse_str(&input.change_id)
                .map_err(|_| Error::ChangeNotFound(input.change_id.clone()))?;
            let applied = self.modules.config.apply(change_id, &principal)?;
            self.modules.security.record_action(
                &principal,
                ManagementAction::ChangeApply.as_str(),
                &ResourceRef::Global,
                AuditOutcome::Succeeded,
                RiskLevel::High,
                json!({"change_id": change_id, "snapshot_version": applied.version}),
            )?;
            Ok(applied)
        })();
        domain_result(result)
    }

    #[tool(description = "List immutable, secret-free management audit events.")]
    async fn list_audit_events(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.list_audit(&principal)
        })();
        domain_result(result)
    }

    #[tool(
        description = "List TLS certificate metadata without account credentials or private keys."
    )]
    async fn list_certificates(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        let result = (|| {
            let principal = principal(&parts)?;
            self.modules.security.authorize(
                &principal,
                ManagementAction::CertificateRead,
                &ResourceRef::Global,
            )?;
            self.modules
                .certificates
                .as_ref()
                .ok_or_else(|| Error::InvalidState("certificate storage is disabled".to_owned()))?
                .list()
        })();
        domain_result(result)
    }

    #[tool(
        description = "Issue a non-wildcard TLS certificate with HTTP-01 and atomically activate it. Never returns private keys."
    )]
    async fn issue_certificate(
        &self,
        Parameters(input): Parameters<IssueCertificateParams>,
        Extension(parts): Extension<Parts>,
    ) -> CallToolResult {
        let principal = match principal(&parts) {
            Ok(principal) => principal,
            Err(error) => return domain_result::<serde_json::Value>(Err(error)),
        };
        if let Err(error) = self.modules.security.authorize(
            &principal,
            ManagementAction::CertificateIssue,
            &ResourceRef::Global,
        ) {
            return domain_result::<serde_json::Value>(Err(error));
        }
        if !(10..=300).contains(&input.timeout_seconds) {
            return CallToolResult::structured_error(json!({
                "code": "INVALID_ACME_TIMEOUT",
                "message": "timeout_seconds must be between 10 and 300",
                "evidence": {"timeout_seconds": input.timeout_seconds}
            }));
        }
        let domains = input.domains;
        let Some(certificates) = &self.modules.certificates else {
            return CallToolResult::structured_error(json!({
                "code": "ACME_DISABLED",
                "message": "ACME issuance is not configured",
                "evidence": {}
            }));
        };
        match certificates
            .issue(domains.clone(), Duration::from_secs(input.timeout_seconds))
            .await
        {
            Ok(result) => {
                if let Err(error) = self.modules.security.record_action(
                    &principal,
                    ManagementAction::CertificateIssue.as_str(),
                    &ResourceRef::Global,
                    AuditOutcome::Succeeded,
                    RiskLevel::High,
                    json!({
                        "certificate_id": result.certificate.certificate_id,
                        "domains": &result.certificate.domains,
                        "not_after_ms": result.certificate.not_after_ms,
                        "tls_generation": result.tls_generation
                    }),
                ) {
                    return domain_result::<serde_json::Value>(Err(error));
                }
                match serde_json::to_value(result) {
                    Ok(value) => CallToolResult::structured(value),
                    Err(_) => CallToolResult::structured_error(json!({
                        "code": "INTERNAL_ERROR",
                        "message": "failed to serialize tool result",
                        "evidence": {}
                    })),
                }
            }
            Err(error) => {
                if let Err(audit_error) = self.modules.security.record_action(
                    &principal,
                    ManagementAction::CertificateIssue.as_str(),
                    &ResourceRef::Global,
                    AuditOutcome::Failed,
                    RiskLevel::High,
                    json!({"domains": domains}),
                ) {
                    return domain_result::<serde_json::Value>(Err(audit_error));
                }
                CallToolResult::structured_error(json!({
                    "code": error.code,
                    "message": error.message,
                    "evidence": {}
                }))
            }
        }
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "senix",
    version = "0.1.0",
    instructions = "Manage Senix gateway traffic through scoped, audited tools. Never executes deployment commands."
)]
impl ServerHandler for SenixMcp {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        let principal = context
            .extensions
            .get::<Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>())
            .ok_or_else(|| ErrorData::invalid_request("management credential is required", None))?;
        let tools = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|tool| self.tool_visible(principal, &tool.name))
            .collect();
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(CacheScope::Private),
        })
    }
}

pub type SenixMcpService = StreamableHttpService<SenixMcp, LocalSessionManager>;

#[must_use]
pub fn streamable_http_service(modules: McpModules) -> SenixMcpService {
    streamable_http_service_with_options(modules, &McpHttpOptions::default())
}

#[must_use]
pub fn streamable_http_service_with_options(
    modules: McpModules,
    options: &McpHttpOptions,
) -> SenixMcpService {
    let mut config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    if !options.allowed_hosts.is_empty() {
        let mut hosts = config.allowed_hosts.clone();
        hosts.extend(options.allowed_hosts.iter().cloned());
        hosts.sort();
        hosts.dedup();
        config = config.with_allowed_hosts(hosts);
    }
    if !options.allowed_origins.is_empty() {
        config = config.with_allowed_origins(options.allowed_origins.iter().cloned());
    }
    StreamableHttpService::new(
        move || Ok(SenixMcp::new(modules.clone())),
        Arc::<LocalSessionManager>::default(),
        config,
    )
}

fn principal(parts: &Parts) -> SenixResult<Principal> {
    parts
        .extensions
        .get::<Principal>()
        .cloned()
        .ok_or(Error::AuthenticationRequired)
}

fn domain_result<T>(result: SenixResult<T>) -> CallToolResult
where
    T: Serialize,
{
    match result {
        Ok(value) => match serde_json::to_value(value) {
            Ok(value) => CallToolResult::structured(value),
            Err(_) => CallToolResult::structured_error(json!({
                "code": "INTERNAL_ERROR",
                "message": "failed to serialize tool result",
                "evidence": {}
            })),
        },
        Err(error) => CallToolResult::structured_error(json!({
            "code": error.code(),
            "message": error.public_message(),
            "evidence": error.evidence(),
        })),
    }
}

const fn default_drain_timeout_ms() -> u64 {
    60_000
}

const fn default_acme_timeout_seconds() -> u64 {
    90
}
