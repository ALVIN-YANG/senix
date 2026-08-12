use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    net::{SocketAddr, TcpListener as StdTcpListener},
    path::PathBuf,
    sync::Arc,
    thread,
};

use anyhow::{Context, Result};
use axum::{
    Extension, Json, Router,
    extract::{Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, patch, post},
};
use clap::{Parser, Subcommand};
use pingora_core::server::Server;
use senix_core::{
    AccessPolicy, AuditOutcome, ChangePlan, ConfigEngine, DiagnosticEngine, DrainOptions,
    GatewayConfig, GatewayRuntime, ManagementAction, Principal, ResourceRef, RiskLevel,
    SecurityController, SqliteStateStore, TrafficController,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

mod health;

#[derive(Debug, Parser)]
#[command(version, about = "Senix Rust gateway")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,

    #[arg(long, requires_all = ["tls_cert", "tls_key"])]
    tls_listen: Option<SocketAddr>,

    #[arg(long, requires_all = ["tls_listen", "tls_key"])]
    tls_cert: Option<PathBuf>,

    #[arg(long, requires_all = ["tls_listen", "tls_cert"])]
    tls_key: Option<PathBuf>,

    #[arg(long, default_value = "127.0.0.1:9080")]
    admin_listen: SocketAddr,

    #[arg(long, default_value = "senix.db")]
    db: PathBuf,

    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long = "mcp-allowed-host", value_name = "HOST")]
    mcp_allowed_hosts: Vec<String>,

    #[arg(long = "mcp-allowed-origin", value_name = "ORIGIN")]
    mcp_allowed_origins: Vec<String>,

    #[arg(long, default_value_t = false)]
    admin_secure_cookie: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    Credential {
        #[command(subcommand)]
        command: CredentialCommand,
    },
    Owner {
        #[command(subcommand)]
        command: OwnerCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CredentialCommand {
    Bootstrap {
        #[arg(long, default_value = "senix.db")]
        db: PathBuf,

        #[arg(long, default_value = "owner")]
        label: String,
    },
}

#[derive(Debug, Subcommand)]
enum OwnerCommand {
    Bootstrap {
        #[arg(long, default_value = "senix.db")]
        db: PathBuf,

        #[arg(long, default_value = "admin")]
        username: String,

        #[arg(long, default_value_t = false)]
        password_stdin: bool,
    },
    ResetPassword {
        #[arg(long, default_value = "senix.db")]
        db: PathBuf,

        #[arg(long, default_value_t = false)]
        password_stdin: bool,
    },
}

#[derive(Clone)]
struct AppState {
    traffic: Arc<TrafficController>,
    config: Arc<ConfigEngine>,
    diagnostics: DiagnosticEngine,
    runtime: Arc<GatewayRuntime>,
    security: Arc<SecurityController>,
    mcp_allowed_hosts: Vec<String>,
    mcp_allowed_origins: Vec<String>,
    admin_secure_cookie: bool,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    evidence: serde_json::Value,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "code": self.code,
                "message": self.message,
                "evidence": self.evidence,
            })),
        )
            .into_response()
    }
}

impl From<senix_core::Error> for ApiError {
    fn from(error: senix_core::Error) -> Self {
        let status = match &error {
            senix_core::Error::AuthenticationRequired
            | senix_core::Error::InvalidCredential
            | senix_core::Error::CredentialExpired
            | senix_core::Error::CredentialRevoked
            | senix_core::Error::InvalidOwnerLogin
            | senix_core::Error::InvalidOwnerSession
            | senix_core::Error::OwnerSessionExpired => StatusCode::UNAUTHORIZED,
            senix_core::Error::Forbidden { .. } => StatusCode::FORBIDDEN,
            senix_core::Error::InstanceNotFound(_)
            | senix_core::Error::DrainOperationNotFound(_)
            | senix_core::Error::SnapshotNotFound(_)
            | senix_core::Error::ChangeNotFound(_)
            | senix_core::Error::CredentialNotFound(_)
            | senix_core::Error::RouteNotFound { .. } => StatusCode::NOT_FOUND,
            senix_core::Error::CredentialAlreadyInitialized
            | senix_core::Error::OwnerAccountAlreadyInitialized
            | senix_core::Error::OwnerAccountNotInitialized
            | senix_core::Error::OwnerCredentialNotInitialized
            | senix_core::Error::LastAvailableBackend { .. }
            | senix_core::Error::StalePlan
            | senix_core::Error::ChangeApprovalRequired(_)
            | senix_core::Error::ChangeApprovalExpired(_) => StatusCode::CONFLICT,
            senix_core::Error::InvalidConfig | senix_core::Error::InvalidState(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            senix_core::Error::NoAvailableBackend(_) => StatusCode::SERVICE_UNAVAILABLE,
            senix_core::Error::Store(_)
            | senix_core::Error::Serialization(_)
            | senix_core::Error::Crypto(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if error.is_internal() {
            error!(error = %error, "management request failed");
        }
        Self {
            status,
            code: error.code(),
            message: error.public_message(),
            evidence: error.evidence(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RejoinRequest {
    generation: u64,
    weight: u32,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct DrainRequest {
    force: bool,
    timeout_ms: u64,
}

impl Default for DrainRequest {
    fn default() -> Self {
        Self {
            force: false,
            timeout_ms: 60_000,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WeightRequest {
    weight: u32,
}

#[derive(Debug, Deserialize)]
struct DiagnosticRequest {
    host: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct IssueCredentialRequest {
    label: String,
    actions: BTreeSet<ManagementAction>,
    #[serde(default)]
    instance_ids: BTreeSet<String>,
    #[serde(default)]
    all_resources: bool,
    expires_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OwnerLoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct OwnerSessionResponse {
    username: String,
    expires_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "senix=info".into()),
        )
        .init();
    let args = Args::parse();

    if let Some(command) = args.command {
        return run_command(command);
    }

    let store = Arc::new(SqliteStateStore::open(&args.db).context("open state database")?);
    let security = Arc::new(SecurityController::new(Arc::clone(&store)));
    let runtime = Arc::new(GatewayRuntime::new());
    let config = Arc::new(ConfigEngine::new(Arc::clone(&runtime), Arc::clone(&store)));
    if config.restore_latest()?.is_none() {
        let path = args
            .config
            .as_ref()
            .context("--config is required for an empty database")?;
        let candidate: GatewayConfig =
            serde_json::from_slice(&fs::read(path).context("read bootstrap config")?)
                .context("parse bootstrap config")?;
        config.initialize(candidate)?;
    }

    let traffic = Arc::new(TrafficController::new(
        Arc::clone(&runtime),
        Arc::clone(&store),
    ));
    let state = AppState {
        traffic,
        config,
        diagnostics: DiagnosticEngine::new(Arc::clone(&runtime)),
        runtime: Arc::clone(&runtime),
        security,
        mcp_allowed_hosts: args.mcp_allowed_hosts,
        mcp_allowed_origins: args.mcp_allowed_origins,
        admin_secure_cookie: args.admin_secure_cookie,
    };

    let admin_listener = StdTcpListener::bind(args.admin_listen)
        .with_context(|| format!("bind admin listener {}", args.admin_listen))?;
    admin_listener.set_nonblocking(true)?;
    spawn_admin(admin_listener, state);

    let mut server = Server::new(None).context("create Pingora server")?;
    server.bootstrap();
    let tls_listen = args.tls_listen.map(|listen| listen.to_string());
    let tls_cert = args
        .tls_cert
        .as_ref()
        .map(|path| path.to_str().context("--tls-cert path is not valid UTF-8"))
        .transpose()?;
    let tls_key = args
        .tls_key
        .as_ref()
        .map(|path| path.to_str().context("--tls-key path is not valid UTF-8"))
        .transpose()?;
    let tls = tls_listen
        .as_deref()
        .zip(tls_cert)
        .zip(tls_key)
        .map(|((listen, cert), key)| (listen, cert, key));
    senix_pingora::add_http_proxy(&mut server, &args.listen.to_string(), tls, runtime)
        .context("configure proxy listeners")?;
    info!(proxy = %args.listen, tls = ?args.tls_listen, admin = %args.admin_listen, "senixd started");
    server.run_forever();
}

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Credential {
            command: CredentialCommand::Bootstrap { db, label },
        } => {
            let store = Arc::new(SqliteStateStore::open(&db).context("open state database")?);
            let issued = SecurityController::new(store)
                .bootstrap_owner_key(&label)
                .context("bootstrap owner credential")?;
            println!("{}", serde_json::to_string_pretty(&issued)?);
            Ok(())
        }
        Command::Owner {
            command:
                OwnerCommand::Bootstrap {
                    db,
                    username,
                    password_stdin,
                },
        } => {
            let password = read_owner_password(password_stdin)?;
            let store = Arc::new(SqliteStateStore::open(&db).context("open state database")?);
            let account = SecurityController::new(store)
                .bootstrap_owner_account(&username, &password)
                .context("bootstrap owner account")?;
            println!("{}", serde_json::to_string_pretty(&account)?);
            Ok(())
        }
        Command::Owner {
            command: OwnerCommand::ResetPassword { db, password_stdin },
        } => {
            let password = read_owner_password(password_stdin)?;
            let store = Arc::new(SqliteStateStore::open(&db).context("open state database")?);
            let account = SecurityController::new(store)
                .reset_owner_password(&password)
                .context("reset owner password")?;
            println!("{}", serde_json::to_string_pretty(&account)?);
            Ok(())
        }
    }
}

fn read_owner_password(password_stdin: bool) -> Result<String> {
    anyhow::ensure!(
        password_stdin,
        "--password-stdin is required so the password is not exposed in process arguments"
    );
    let mut password = String::new();
    io::stdin()
        .read_to_string(&mut password)
        .context("read owner password from stdin")?;
    Ok(password.trim_end_matches(['\r', '\n']).to_owned())
}

fn spawn_admin(listener: StdTcpListener, state: AppState) {
    thread::Builder::new()
        .name("senix-admin".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("senix-admin-worker")
                .build()
                .expect("build admin runtime");
            runtime.block_on(async move {
                tokio::spawn(health::run(Arc::clone(&state.runtime)));
                let listener =
                    tokio::net::TcpListener::from_std(listener).expect("adopt admin listener");
                axum::serve(listener, admin_router(state))
                    .await
                    .expect("serve admin interface");
            });
        })
        .expect("spawn admin thread");
}

fn admin_router(state: AppState) -> Router {
    let mcp = senix_mcp::streamable_http_service_with_options(
        senix_mcp::McpModules {
            traffic: Arc::clone(&state.traffic),
            config: Arc::clone(&state.config),
            diagnostics: state.diagnostics.clone(),
            security: Arc::clone(&state.security),
        },
        &senix_mcp::McpHttpOptions {
            allowed_hosts: state.mcp_allowed_hosts.clone(),
            allowed_origins: state.mcp_allowed_origins.clone(),
        },
    );
    let api = Router::new()
        .route("/api/v1/auth/session", get(owner_session))
        .route("/api/v1/auth/session", delete(owner_logout))
        .route("/api/v1/instances", get(list_instances))
        .route("/api/v1/instances/{id}", get(instance_status))
        .route("/api/v1/instances/{id}/drain", post(drain))
        .route("/api/v1/operations/{id}", get(drain_status))
        .route("/api/v1/instances/{id}/rejoin", post(rejoin))
        .route("/api/v1/instances/{id}/weight", patch(set_weight))
        .route("/api/v1/instances/{id}/disable", post(disable))
        .route("/api/v1/diagnostics/requests", post(diagnose))
        .route("/api/v1/config", get(current_config))
        .route("/api/v1/changes", get(list_changes))
        .route("/api/v1/changes/plan", post(plan_change))
        .route("/api/v1/changes/{id}", get(get_change))
        .route("/api/v1/changes/{id}/approve", post(approve_change))
        .route("/api/v1/changes/{id}/apply", post(apply_change))
        .route(
            "/api/v1/snapshots/{version}/rollback-plan",
            post(plan_rollback),
        )
        .route(
            "/api/v1/credentials",
            get(list_credentials).post(issue_credential),
        )
        .route("/api/v1/credentials/{id}", delete(revoke_credential))
        .route("/api/v1/audit-events", get(list_audit_events))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state.security),
            authenticate_api,
        ));
    let mcp = Router::new()
        .nest_service("/mcp", mcp)
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state.security),
            authenticate_bearer,
        ));
    Router::new()
        .route("/", get(admin_redirect))
        .route("/admin", get(admin_redirect))
        .route("/admin/", get(admin_index))
        .route("/admin/admin.css", get(admin_css))
        .route("/admin/admin.js", get(admin_js))
        .route("/healthz", get(health))
        .route("/api/v1/auth/login", post(owner_login))
        .merge(api)
        .merge(mcp)
        .with_state(state)
}

async fn authenticate_bearer(
    State(security): State<Arc<SecurityController>>,
    request: Request,
    next: Next,
) -> Response {
    let result = bearer_from_headers(request.headers())
        .ok_or(senix_core::Error::AuthenticationRequired)
        .and_then(|api_key| security.authenticate(api_key));
    finish_authentication(result, request, next).await
}

async fn authenticate_api(
    State(security): State<Arc<SecurityController>>,
    request: Request,
    next: Next,
) -> Response {
    let bearer = bearer_from_headers(request.headers());
    let session = session_cookie(request.headers());
    let (result, uses_cookie) = if let Some(api_key) = bearer {
        (security.authenticate(api_key), false)
    } else if let Some(session) = session {
        (security.authenticate_owner_session(session), true)
    } else {
        (Err(senix_core::Error::AuthenticationRequired), false)
    };
    if uses_cookie
        && !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        )
        && request
            .headers()
            .get("x-senix-csrf")
            .and_then(|value| value.to_str().ok())
            != Some("1")
    {
        return ApiError {
            status: StatusCode::FORBIDDEN,
            code: "CSRF_REQUIRED",
            message: "cookie-authenticated writes require X-Senix-CSRF: 1".to_owned(),
            evidence: json!({}),
        }
        .into_response();
    }
    finish_authentication(result, request, next).await
}

async fn finish_authentication(
    result: senix_core::Result<Principal>,
    mut request: Request,
    next: Next,
) -> Response {
    match result {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            let mut response = next.run(request).await;
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(error) => {
            let mut response = ApiError::from(error).into_response();
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Bearer"),
            );
            response
        }
    }
}

fn bearer_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == "senix_session").then_some(value)
            })
        })
}

fn bearer_token(value: &str) -> Option<&str> {
    let mut parts = value.split_ascii_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    (scheme.eq_ignore_ascii_case("Bearer") && parts.next().is_none()).then_some(token)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn admin_redirect() -> Redirect {
    Redirect::temporary("/admin/")
}

async fn admin_index() -> Response {
    static_response(
        "text/html; charset=utf-8",
        include_str!("../assets/admin.html"),
        true,
    )
}

async fn admin_css() -> Response {
    static_response(
        "text/css; charset=utf-8",
        include_str!("../assets/admin.css"),
        false,
    )
}

async fn admin_js() -> Response {
    static_response(
        "text/javascript; charset=utf-8",
        include_str!("../assets/admin.js"),
        false,
    )
}

fn static_response(content_type: &'static str, body: &'static str, html: bool) -> Response {
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if html {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
        );
    }
    response
}

async fn owner_login(
    State(state): State<AppState>,
    Json(body): Json<OwnerLoginRequest>,
) -> Result<Response, ApiError> {
    const SESSION_TTL_MS: i64 = 8 * 60 * 60 * 1_000;
    let issued = state
        .security
        .login_owner(&body.username, &body.password, SESSION_TTL_MS)?;
    let mut cookie = format!(
        "senix_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        issued.token,
        SESSION_TTL_MS / 1_000
    );
    if state.admin_secure_cookie {
        cookie.push_str("; Secure");
    }
    let mut response = Json(OwnerSessionResponse {
        username: issued.username,
        expires_at_ms: Some(issued.expires_at_ms),
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: "could not create owner session".to_owned(),
            evidence: json!({}),
        })?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn owner_session(Extension(principal): Extension<Principal>) -> Json<OwnerSessionResponse> {
    Json(OwnerSessionResponse {
        username: principal.label,
        expires_at_ms: None,
    })
}

async fn owner_logout(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Response, ApiError> {
    state.security.logout_owner(&principal)?;
    let mut cookie = "senix_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0".to_owned();
    if state.admin_secure_cookie {
        cookie.push_str("; Secure");
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("static cookie attributes are valid"),
    );
    Ok(response)
}

async fn list_instances(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<senix_core::InstanceState>>, ApiError> {
    let instances = state
        .traffic
        .list_instances()
        .into_iter()
        .filter(|instance| {
            state.security.allows(
                &principal,
                ManagementAction::InstanceRead,
                &ResourceRef::Instance(instance.id.clone()),
            )
        })
        .collect();
    Ok(Json(instances))
}

async fn instance_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<senix_core::InstanceState>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::InstanceRead,
        &ResourceRef::Instance(id.clone()),
    )?;
    Ok(Json(state.traffic.status(&id)?))
}

async fn drain(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Option<Json<DrainRequest>>,
) -> Result<(StatusCode, Json<senix_core::DrainOperation>), ApiError> {
    let resource = ResourceRef::Instance(id.clone());
    state
        .security
        .authorize(&principal, ManagementAction::InstanceDrain, &resource)?;
    let body = body.map_or_else(DrainRequest::default, |Json(body)| body);
    let operation = state.traffic.begin_drain(
        &id,
        DrainOptions {
            force: body.force,
            timeout_ms: body.timeout_ms,
        },
        idempotency_key(&headers)?,
    )?;
    state.security.record_action(
        &principal,
        ManagementAction::InstanceDrain.as_str(),
        &resource,
        AuditOutcome::Succeeded,
        if body.force {
            RiskLevel::High
        } else {
            RiskLevel::Medium
        },
        json!({"operation_id": operation.operation_id, "force": body.force}),
    )?;
    Ok((StatusCode::ACCEPTED, Json(operation)))
}

async fn drain_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<senix_core::DrainOperation>, ApiError> {
    let operation = state.traffic.drain_status(&id)?;
    state.security.authorize(
        &principal,
        ManagementAction::InstanceRead,
        &ResourceRef::Instance(operation.instance_id.clone()),
    )?;
    Ok(Json(operation))
}

async fn rejoin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Json(body): Json<RejoinRequest>,
) -> Result<Json<senix_core::InstanceState>, ApiError> {
    let resource = ResourceRef::Instance(id.clone());
    state
        .security
        .authorize(&principal, ManagementAction::InstanceRejoin, &resource)?;
    let instance = state.traffic.rejoin(
        &id,
        body.generation,
        body.weight,
        body.force,
        idempotency_key(&headers)?,
    )?;
    state.security.record_action(
        &principal,
        ManagementAction::InstanceRejoin.as_str(),
        &resource,
        AuditOutcome::Succeeded,
        if body.force {
            RiskLevel::High
        } else {
            RiskLevel::Medium
        },
        json!({"generation": body.generation, "force": body.force}),
    )?;
    Ok(Json(instance))
}

async fn set_weight(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Json(body): Json<WeightRequest>,
) -> Result<Json<senix_core::InstanceState>, ApiError> {
    let resource = ResourceRef::Instance(id.clone());
    state
        .security
        .authorize(&principal, ManagementAction::InstanceSetWeight, &resource)?;
    let instance = state
        .traffic
        .set_weight(&id, body.weight, idempotency_key(&headers)?)?;
    state.security.record_action(
        &principal,
        ManagementAction::InstanceSetWeight.as_str(),
        &resource,
        AuditOutcome::Succeeded,
        RiskLevel::Medium,
        json!({"weight": body.weight}),
    )?;
    Ok(Json(instance))
}

async fn disable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> Result<Json<senix_core::InstanceState>, ApiError> {
    let resource = ResourceRef::Instance(id.clone());
    state
        .security
        .authorize(&principal, ManagementAction::InstanceDisable, &resource)?;
    let instance = state.traffic.disable(&id, idempotency_key(&headers)?)?;
    state.security.record_action(
        &principal,
        ManagementAction::InstanceDisable.as_str(),
        &resource,
        AuditOutcome::Succeeded,
        RiskLevel::High,
        json!({}),
    )?;
    Ok(Json(instance))
}

async fn diagnose(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<DiagnosticRequest>,
) -> Result<Json<senix_core::DiagnosticReport>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::DiagnosticsRead,
        &ResourceRef::Global,
    )?;
    Ok(Json(state.diagnostics.diagnose(&body.host, &body.path)))
}

async fn plan_change(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(candidate): Json<GatewayConfig>,
) -> Result<(StatusCode, Json<ChangePlan>), ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::ChangePlan,
        &ResourceRef::Global,
    )?;
    let plan = state.config.plan(candidate, &principal)?;
    state.security.record_action(
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
    Ok((StatusCode::CREATED, Json(plan)))
}

async fn list_changes(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<ChangePlan>>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::ChangeRead,
        &ResourceRef::Global,
    )?;
    Ok(Json(state.config.list_changes()?))
}

async fn current_config(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<senix_core::ConfigSnapshot>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::ChangeRead,
        &ResourceRef::Global,
    )?;
    let snapshot = state.config.current()?.ok_or_else(|| {
        senix_core::Error::InvalidState("configuration is not initialized".into())
    })?;
    Ok(Json(snapshot))
}

async fn get_change(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<ChangePlan>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::ChangeRead,
        &ResourceRef::Global,
    )?;
    let change = state
        .config
        .change(id)?
        .ok_or_else(|| senix_core::Error::ChangeNotFound(id.to_string()))?;
    Ok(Json(change))
}

async fn approve_change(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<ChangePlan>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::ChangeApprove,
        &ResourceRef::Global,
    )?;
    let change = state.config.approve(id, &principal)?;
    state.security.record_action(
        &principal,
        ManagementAction::ChangeApprove.as_str(),
        &ResourceRef::Global,
        AuditOutcome::Succeeded,
        RiskLevel::High,
        json!({"change_id": id, "candidate_digest": change.candidate_digest}),
    )?;
    Ok(Json(change))
}

async fn apply_change(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<senix_core::AppliedChange>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::ChangeApply,
        &ResourceRef::Global,
    )?;
    let applied = state.config.apply(id, &principal)?;
    state.security.record_action(
        &principal,
        ManagementAction::ChangeApply.as_str(),
        &ResourceRef::Global,
        AuditOutcome::Succeeded,
        RiskLevel::High,
        json!({"change_id": id, "snapshot_version": applied.version}),
    )?;
    Ok(Json(applied))
}

async fn plan_rollback(
    State(state): State<AppState>,
    Path(version): Path<u64>,
    Extension(principal): Extension<Principal>,
) -> Result<(StatusCode, Json<ChangePlan>), ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::ChangePlan,
        &ResourceRef::Global,
    )?;
    let plan = state.config.plan_rollback(version, &principal)?;
    state.security.record_action(
        &principal,
        "change.plan_rollback",
        &ResourceRef::Global,
        AuditOutcome::Succeeded,
        RiskLevel::Low,
        json!({"change_id": plan.change_id, "target_version": version}),
    )?;
    Ok((StatusCode::CREATED, Json(plan)))
}

async fn issue_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<IssueCredentialRequest>,
) -> Result<(StatusCode, Json<senix_core::IssuedApiKey>), ApiError> {
    let issued = state.security.issue_key(
        &principal,
        &body.label,
        AccessPolicy {
            all_resources: body.all_resources,
            actions: body.actions,
            instance_ids: body.instance_ids,
        },
        body.expires_at_ms,
    )?;
    Ok((StatusCode::CREATED, Json(issued)))
}

async fn list_credentials(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<senix_core::CredentialSummary>>, ApiError> {
    Ok(Json(state.security.list_credentials(&principal)?))
}

async fn revoke_credential(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
) -> Result<StatusCode, ApiError> {
    let id = uuid::Uuid::parse_str(&id).map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "INVALID_CREDENTIAL_ID",
        message: "credential id must be a UUID".to_owned(),
        evidence: json!({"credential_id": id}),
    })?;
    state.security.revoke(&principal, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_audit_events(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<senix_core::AuditEvent>>, ApiError> {
    Ok(Json(state.security.list_audit(&principal)?))
}

fn idempotency_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "IDEMPOTENCY_KEY_REQUIRED",
            message: "Idempotency-Key header is required".to_owned(),
            evidence: json!({}),
        })
}
