use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write as _,
    fs,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener as StdTcpListener},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, patch, post},
};
use clap::{Parser, Subcommand};
use pingora_core::server::{Server, configuration::ServerConf};
use senix_core::{
    AccessPolicy, AuditOutcome, ChangePlan, ConfigEngine, DiagnosticEngine, DrainOptions,
    GatewayConfig, GatewayRuntime, Http01ChallengeRegistry, ManagementAction, Principal,
    ResourceRef, RiskLevel, SecretVault, SecurityController, SqliteStateStore, TrafficController,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

mod certificate;
mod health;

#[derive(Debug, Parser)]
#[command(version, about = "Senix Rust gateway")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,

    #[arg(long)]
    tls_listen: Option<SocketAddr>,

    #[arg(long, requires_all = ["tls_listen", "tls_key"])]
    tls_cert: Option<PathBuf>,

    #[arg(long, requires_all = ["tls_listen", "tls_cert"])]
    tls_key: Option<PathBuf>,

    #[arg(long)]
    secret_key_file: Option<PathBuf>,

    #[arg(long, requires = "secret_key_file")]
    acme_directory_url: Option<String>,

    #[arg(long = "acme-contact", requires = "acme_directory_url")]
    acme_contacts: Vec<String>,

    #[arg(long, default_value_t = false, requires = "acme_directory_url")]
    acme_accept_terms: bool,

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

    #[arg(long, default_value_t = 30)]
    shutdown_grace_seconds: u64,

    #[arg(long, default_value_t = 5)]
    shutdown_timeout_seconds: u64,
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
    SecretKey {
        #[command(subcommand)]
        command: SecretKeyCommand,
    },
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
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

#[derive(Debug, Subcommand)]
enum SecretKeyCommand {
    Generate {
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum BackupCommand {
    Create {
        #[arg(long, default_value = "senix.db")]
        db: PathBuf,

        #[arg(long)]
        output: PathBuf,

        #[arg(long)]
        secret_key_file: Option<PathBuf>,
    },
    Verify {
        #[arg(long)]
        input: PathBuf,

        #[arg(long)]
        secret_key_file: Option<PathBuf>,
    },
    Restore {
        #[arg(long)]
        input: PathBuf,

        #[arg(long)]
        db: PathBuf,

        #[arg(long)]
        secret_key_file: Option<PathBuf>,
    },
}

#[derive(Clone)]
struct AppState {
    traffic: Arc<TrafficController>,
    config: Arc<ConfigEngine>,
    diagnostics: DiagnosticEngine,
    runtime: Arc<GatewayRuntime>,
    metrics: Arc<senix_pingora::ProxyMetrics>,
    tls: senix_pingora::TlsCertificateRegistry,
    login_limiter: Arc<LoginRateLimiter>,
    security: Arc<SecurityController>,
    certificates: Option<Arc<senix_core::CertificateController>>,
    acme: Option<Arc<certificate::AcmeManager>>,
    mcp_allowed_hosts: Vec<String>,
    mcp_allowed_origins: Vec<String>,
    admin_secure_cookie: bool,
}

const LOGIN_FAILURE_LIMIT: u8 = 5;
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);
const LOGIN_LOCKOUT: Duration = Duration::from_secs(15 * 60);
const MAX_TRACKED_LOGIN_PEERS: usize = 4_096;

#[derive(Debug)]
struct LoginFailureState {
    failures: u8,
    window_started: Instant,
    locked_until: Option<Instant>,
    last_seen: Instant,
}

#[derive(Debug)]
struct LoginRateLimiter {
    peers: Mutex<HashMap<IpAddr, LoginFailureState>>,
    verification_slots: Arc<tokio::sync::Semaphore>,
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self {
            peers: Mutex::default(),
            verification_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}

impl LoginRateLimiter {
    fn try_acquire_verification(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.verification_slots)
            .try_acquire_owned()
            .ok()
    }

    fn retry_after(&self, peer: IpAddr) -> Option<Duration> {
        let now = Instant::now();
        let mut peers = self
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = peers.get_mut(&peer)?;
        state.last_seen = now;
        if let Some(retry_after) = state
            .locked_until
            .and_then(|deadline| deadline.checked_duration_since(now))
        {
            return Some(retry_after);
        }
        if now.duration_since(state.window_started) >= LOGIN_FAILURE_WINDOW {
            peers.remove(&peer);
        }
        None
    }

    fn record_failure(&self, peer: IpAddr) {
        let now = Instant::now();
        let mut peers = self
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !peers.contains_key(&peer) && peers.len() >= MAX_TRACKED_LOGIN_PEERS {
            let oldest = peers
                .iter()
                .min_by_key(|(_, state)| state.last_seen)
                .map(|(peer, _)| *peer);
            if let Some(oldest) = oldest {
                peers.remove(&oldest);
            }
        }
        let state = peers.entry(peer).or_insert(LoginFailureState {
            failures: 0,
            window_started: now,
            locked_until: None,
            last_seen: now,
        });
        if now.duration_since(state.window_started) >= LOGIN_FAILURE_WINDOW {
            state.failures = 0;
            state.window_started = now;
            state.locked_until = None;
        }
        state.failures = state.failures.saturating_add(1);
        state.last_seen = now;
        if state.failures >= LOGIN_FAILURE_LIMIT {
            state.locked_until = Some(now + LOGIN_LOCKOUT);
        }
    }

    fn record_success(&self, peer: IpAddr) {
        self.peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&peer);
    }
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
        let mut response = (
            self.status,
            Json(json!({
                "code": self.code,
                "message": self.message,
                "evidence": self.evidence,
            })),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
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

impl From<certificate::Error> for ApiError {
    fn from(error: certificate::Error) -> Self {
        if let certificate::Error::Acme(senix_acme::Error::InvalidDomains(message)) = &error {
            return Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: "INVALID_CERTIFICATE_DOMAINS",
                message: message.clone(),
                evidence: json!({}),
            };
        }
        error!(error = %error, "certificate issuance failed");
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "CERTIFICATE_ISSUANCE_FAILED",
            message: "certificate issuance failed; inspect the audit log and server diagnostics"
                .to_owned(),
            evidence: json!({}),
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
struct IssueCertificateRequest {
    domains: Vec<String>,
    #[serde(default = "default_acme_timeout_seconds")]
    timeout_seconds: u64,
}

const fn default_acme_timeout_seconds() -> u64 {
    90
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

#[derive(Debug, Serialize)]
struct BackupReport {
    schema_version: i64,
    snapshot_version: Option<u64>,
    managed_secret_count: u64,
    managed_certificate_count: u64,
    master_key_verified: bool,
}

struct CertificateServices {
    tls: senix_pingora::TlsCertificateRegistry,
    challenges: Http01ChallengeRegistry,
    controller: Option<Arc<senix_core::CertificateController>>,
    acme: Option<Arc<certificate::AcmeManager>>,
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
    let certificate_services = initialize_certificate_services(&args, &store)?;

    let mut server_config = ServerConf::new().context("create Pingora server configuration")?;
    server_config.grace_period_seconds = Some(args.shutdown_grace_seconds);
    server_config.graceful_shutdown_timeout_seconds = Some(args.shutdown_timeout_seconds);
    let mut server = Server::new_with_opt_and_conf(None, server_config);
    server.bootstrap();
    let tls_listen = args.tls_listen.map(|listen| listen.to_string());
    let tls = tls_listen
        .as_deref()
        .map(|listen| (listen, certificate_services.tls.clone()));
    let metrics = Arc::new(senix_pingora::ProxyMetrics::default());
    senix_pingora::add_http_proxy(
        &mut server,
        &args.listen.to_string(),
        tls,
        Arc::clone(&runtime),
        certificate_services.challenges.clone(),
        Arc::clone(&metrics),
    )
    .context("configure proxy listeners")?;

    let state = AppState {
        traffic,
        config,
        diagnostics: DiagnosticEngine::new(Arc::clone(&runtime)),
        runtime: Arc::clone(&runtime),
        metrics,
        tls: certificate_services.tls.clone(),
        login_limiter: Arc::default(),
        security,
        certificates: certificate_services.controller,
        acme: certificate_services.acme,
        mcp_allowed_hosts: args.mcp_allowed_hosts,
        mcp_allowed_origins: args.mcp_allowed_origins,
        admin_secure_cookie: args.admin_secure_cookie,
    };

    let admin_listener = StdTcpListener::bind(args.admin_listen)
        .with_context(|| format!("bind admin listener {}", args.admin_listen))?;
    admin_listener.set_nonblocking(true)?;
    spawn_admin(admin_listener, state);

    info!(proxy = %args.listen, tls = ?args.tls_listen, admin = %args.admin_listen, "senixd started");
    server.run_forever();
}

fn initialize_certificate_services(
    args: &Args,
    store: &Arc<SqliteStateStore>,
) -> Result<CertificateServices> {
    let tls = senix_pingora::TlsCertificateRegistry::new();
    if let (Some(certificate_path), Some(private_key_path)) =
        (args.tls_cert.as_ref(), args.tls_key.as_ref())
    {
        let installed = tls
            .install_files(certificate_path, private_key_path, true)
            .context("load TLS certificate")?;
        info!(generation = installed.generation, domains = ?installed.domains, "TLS certificate installed");
    }

    let controller = args
        .secret_key_file
        .as_ref()
        .map(|path| {
            let vault = load_secret_vault(path)?;
            Ok::<_, anyhow::Error>(Arc::new(senix_core::CertificateController::new(
                Arc::clone(store),
                vault,
            )))
        })
        .transpose()?;
    if let Some(controller) = &controller {
        for certificate in controller
            .load_active()
            .context("load managed certificates")?
        {
            let prepared = senix_pingora::TlsCertificateRegistry::prepare_pem(
                &certificate.certificate_chain_pem,
                certificate.private_key_pem.expose(),
            )
            .with_context(|| format!("parse managed certificate {}", certificate.certificate_id))?;
            let installed = tls.install_prepared(&prepared, false);
            info!(
                certificate_id = %certificate.certificate_id,
                generation = installed.generation,
                domains = ?installed.domains,
                "managed TLS certificate restored"
            );
        }
    }

    let challenges = Http01ChallengeRegistry::new();
    let acme = args
        .acme_directory_url
        .as_ref()
        .map(|directory_url| {
            anyhow::ensure!(
                args.acme_accept_terms,
                "--acme-accept-terms is required when ACME is enabled"
            );
            let controller = controller
                .as_ref()
                .context("ACME requires --secret-key-file")?;
            Ok::<_, anyhow::Error>(Arc::new(certificate::AcmeManager::new(
                senix_acme::AccountConfig {
                    directory_url: directory_url.clone(),
                    contacts: args.acme_contacts.clone(),
                    terms_of_service_agreed: true,
                },
                challenges.clone(),
                Arc::clone(controller),
                tls.clone(),
            )))
        })
        .transpose()?;
    anyhow::ensure!(
        args.tls_listen.is_none() || tls.generation() > 0 || acme.is_some(),
        "--tls-listen requires an installed certificate or ACME configuration"
    );
    Ok(CertificateServices {
        tls,
        challenges,
        controller,
        acme,
    })
}

fn load_secret_vault(path: &std::path::Path) -> Result<SecretVault> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("read secret key metadata {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "secret key path must be a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        anyhow::ensure!(
            metadata.permissions().mode().trailing_zeros() >= 6,
            "secret key file must not be readable or writable by group or others"
        );
    }
    let encoded = fs::read_to_string(path)
        .with_context(|| format!("read secret key file {}", path.display()))?;
    SecretVault::from_base64(encoded.trim()).context("parse secret key")
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
        Command::SecretKey {
            command: SecretKeyCommand::Generate { output },
        } => generate_secret_key(&output),
        Command::Backup {
            command:
                BackupCommand::Create {
                    db,
                    output,
                    secret_key_file,
                },
        } => create_backup(&db, &output, secret_key_file.as_deref()),
        Command::Backup {
            command:
                BackupCommand::Verify {
                    input,
                    secret_key_file,
                },
        } => {
            let report = inspect_backup(&input, secret_key_file.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Backup {
            command:
                BackupCommand::Restore {
                    input,
                    db,
                    secret_key_file,
                },
        } => restore_backup(&input, &db, secret_key_file.as_deref()),
    }
}

fn create_backup(
    db: &std::path::Path,
    output: &std::path::Path,
    key: Option<&std::path::Path>,
) -> Result<()> {
    anyhow::ensure!(
        db.is_file(),
        "state database does not exist: {}",
        db.display()
    );
    ensure_new_output(output)?;
    let store = Arc::new(SqliteStateStore::open(db).context("open state database")?);
    store.verify_integrity().context("verify source database")?;
    verify_master_key(Arc::clone(&store), key)?;

    let temporary = temporary_output(output)?;
    store
        .backup_to(&temporary)
        .context("create consistent SQLite backup")?;
    let report = inspect_backup(&temporary, key)?;
    persist_private_file(temporary, output)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn restore_backup(
    input: &std::path::Path,
    db: &std::path::Path,
    key: Option<&std::path::Path>,
) -> Result<()> {
    ensure_new_output(db)?;
    let source = Arc::new(
        SqliteStateStore::open_read_only(input).context("open backup database read-only")?,
    );
    source
        .verify_integrity()
        .context("verify backup database")?;
    let report = backup_report(&source, key)?;

    let temporary = temporary_output(db)?;
    source
        .backup_to(&temporary)
        .context("copy verified backup")?;
    inspect_backup(&temporary, key).context("verify restored database")?;
    persist_private_file(temporary, db)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn inspect_backup(input: &std::path::Path, key: Option<&std::path::Path>) -> Result<BackupReport> {
    anyhow::ensure!(
        input.is_file(),
        "backup does not exist: {}",
        input.display()
    );
    let store = Arc::new(
        SqliteStateStore::open_read_only(input).context("open backup database read-only")?,
    );
    store
        .verify_integrity()
        .context("verify backup integrity")?;
    backup_report(&store, key)
}

fn backup_report(
    store: &Arc<SqliteStateStore>,
    key: Option<&std::path::Path>,
) -> Result<BackupReport> {
    let status = store.database_status().context("read backup inventory")?;
    let master_key_verified = verify_master_key(Arc::clone(store), key)?;
    Ok(BackupReport {
        schema_version: status.schema_version,
        snapshot_version: status.snapshot_version,
        managed_secret_count: status.managed_secret_count,
        managed_certificate_count: status.managed_certificate_count,
        master_key_verified,
    })
}

fn verify_master_key(store: Arc<SqliteStateStore>, key: Option<&std::path::Path>) -> Result<bool> {
    let status = store
        .database_status()
        .context("read protected inventory")?;
    if status.managed_secret_count == 0 && status.managed_certificate_count == 0 {
        return Ok(false);
    }
    let key = key.context(
        "--secret-key-file is required because the database contains encrypted material",
    )?;
    let vault = load_secret_vault(key)?;
    senix_core::CertificateController::new(store, vault)
        .verify_protected_material()
        .context("master key cannot decrypt all protected backup material")?;
    Ok(true)
}

fn ensure_new_output(path: &std::path::Path) -> Result<()> {
    anyhow::ensure!(
        !path.exists(),
        "refusing to overwrite existing file: {}",
        path.display()
    );
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    anyhow::ensure!(
        parent.is_dir(),
        "output directory does not exist: {}",
        parent.display()
    );
    Ok(())
}

fn temporary_output(destination: &std::path::Path) -> Result<tempfile::TempPath> {
    let parent = destination
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    Ok(tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary file in {}", parent.display()))?
        .into_temp_path())
}

fn persist_private_file(
    temporary: tempfile::TempPath,
    destination: &std::path::Path,
) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::OpenOptions::new()
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    temporary
        .persist_noclobber(destination)
        .map_err(|error| error.error)
        .with_context(|| format!("persist {} without overwriting", destination.display()))?;
    #[cfg(unix)]
    {
        let parent = destination
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn generate_secret_key(output: &std::path::Path) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(output)
        .with_context(|| format!("create secret key file {}", output.display()))?;
    file.write_all(SecretVault::generate_base64().as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!("created {}", output.display());
    Ok(())
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
                axum::serve(
                    listener,
                    admin_router(state).into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .expect("serve admin interface");
            });
        })
        .expect("spawn admin thread");
}

fn admin_router(state: AppState) -> Router {
    let mcp_certificates = state.certificates.as_ref().map(|certificates| {
        Arc::new(certificate::McpCertificateManager::new(
            Arc::clone(certificates),
            state.acme.clone(),
        )) as Arc<dyn senix_mcp::CertificateManagement>
    });
    let mcp = senix_mcp::streamable_http_service_with_options(
        senix_mcp::McpModules {
            traffic: Arc::clone(&state.traffic),
            config: Arc::clone(&state.config),
            diagnostics: state.diagnostics.clone(),
            security: Arc::clone(&state.security),
            certificates: mcp_certificates,
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
        .route("/api/v1/certificates", get(list_certificates))
        .route("/api/v1/certificates/issue", post(issue_certificate))
        .route("/api/v1/audit-events", get(list_audit_events))
        .route("/metrics", get(metrics))
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(body): Json<OwnerLoginRequest>,
) -> Response {
    const SESSION_TTL_MS: i64 = 8 * 60 * 60 * 1_000;
    if let Some(retry_after) = state.login_limiter.retry_after(peer.ip()) {
        return login_rate_limited_response(retry_after);
    }
    let Some(_verification_slot) = state.login_limiter.try_acquire_verification() else {
        return login_rate_limited_response(Duration::from_secs(1));
    };
    if let Some(retry_after) = state.login_limiter.retry_after(peer.ip()) {
        return login_rate_limited_response(retry_after);
    }
    let security = Arc::clone(&state.security);
    let login = tokio::task::spawn_blocking(move || {
        security.login_owner(&body.username, &body.password, SESSION_TTL_MS)
    })
    .await;
    let issued = match login {
        Err(error) => {
            error!(error = %error, "owner login verifier failed");
            return ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "owner login could not be completed".to_owned(),
                evidence: json!({}),
            }
            .into_response();
        }
        Ok(Ok(issued)) => issued,
        Ok(Err(error)) => {
            if matches!(error, senix_core::Error::InvalidOwnerLogin) {
                state.login_limiter.record_failure(peer.ip());
            }
            return ApiError::from(error).into_response();
        }
    };
    state.login_limiter.record_success(peer.ip());
    owner_login_response(&state, issued).unwrap_or_else(IntoResponse::into_response)
}

fn owner_login_response(
    state: &AppState,
    issued: senix_core::IssuedOwnerSession,
) -> Result<Response, ApiError> {
    const SESSION_TTL_MS: i64 = 8 * 60 * 60 * 1_000;
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

fn login_rate_limited_response(retry_after: Duration) -> Response {
    let seconds = retry_after
        .as_secs()
        .saturating_add(u64::from(retry_after.subsec_nanos() > 0));
    let mut response = ApiError {
        status: StatusCode::TOO_MANY_REQUESTS,
        code: "LOGIN_RATE_LIMITED",
        message: "too many failed login attempts; retry later".to_owned(),
        evidence: json!({"retry_after_seconds": seconds}),
    }
    .into_response();
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_str(&seconds.to_string())
            .expect("integer retry duration is a valid HTTP header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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

async fn metrics(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Response, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::MetricsRead,
        &ResourceRef::Global,
    )?;
    let proxy = state.metrics.snapshot();
    let config_version = state
        .config
        .current()?
        .map_or(0, |snapshot| snapshot.version);
    let instances = state.traffic.list_instances();
    let mut body = format!(
        "# HELP senix_build_info Senix process build information.\n\
# TYPE senix_build_info gauge\n\
senix_build_info{{version=\"{}\"}} 1\n\
# HELP senix_proxy_requests_total Requests accepted by the proxy.\n\
# TYPE senix_proxy_requests_total counter\n\
senix_proxy_requests_total {}\n\
# HELP senix_proxy_responses_total Proxy responses by HTTP status class.\n\
# TYPE senix_proxy_responses_total counter\n\
senix_proxy_responses_total{{class=\"1xx\"}} {}\n\
senix_proxy_responses_total{{class=\"2xx\"}} {}\n\
senix_proxy_responses_total{{class=\"3xx\"}} {}\n\
senix_proxy_responses_total{{class=\"4xx\"}} {}\n\
senix_proxy_responses_total{{class=\"5xx\"}} {}\n\
# HELP senix_proxy_errors_total Proxy requests that ended with a Pingora error.\n\
# TYPE senix_proxy_errors_total counter\n\
senix_proxy_errors_total {}\n\
# HELP senix_config_snapshot_version Currently published configuration snapshot.\n\
# TYPE senix_config_snapshot_version gauge\n\
senix_config_snapshot_version {}\n",
        prometheus_escape(env!("CARGO_PKG_VERSION")),
        proxy.requests,
        proxy.responses_1xx,
        proxy.responses_2xx,
        proxy.responses_3xx,
        proxy.responses_4xx,
        proxy.responses_5xx,
        proxy.errors,
        config_version,
    );
    append_certificate_metrics(&mut body, &state);
    body.push_str(
        "# HELP senix_instance_in_flight Requests currently assigned to an upstream instance.\n\
# TYPE senix_instance_in_flight gauge\n",
    );
    for instance in &instances {
        let instance_id = prometheus_escape(&instance.id);
        write!(
            body,
            "senix_instance_in_flight{{instance_id=\"{instance_id}\",kind=\"ordinary\"}} {}\n\
senix_instance_in_flight{{instance_id=\"{instance_id}\",kind=\"long_lived\"}} {}\n",
            instance
                .in_flight
                .saturating_sub(instance.long_lived_in_flight),
            instance.long_lived_in_flight,
        )
        .expect("writing metrics to a String cannot fail");
    }
    body.push_str(
        "# HELP senix_instance_info Current traffic and health state for each instance.\n\
# TYPE senix_instance_info gauge\n",
    );
    for instance in &instances {
        writeln!(
            body,
            "senix_instance_info{{instance_id=\"{}\",traffic=\"{}\",health=\"{}\",generation=\"{}\"}} 1",
            prometheus_escape(&instance.id),
            traffic_state_label(instance.traffic),
            health_state_label(instance.health),
            instance.generation,
        )
        .expect("writing metrics to a String cannot fail");
    }

    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn append_certificate_metrics(body: &mut String, state: &AppState) {
    let certificates = state.tls.active_certificates();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX);
    let expired = certificates
        .iter()
        .filter(|certificate| certificate.not_after_ms <= now_ms)
        .count();
    let expiring = certificates
        .iter()
        .filter(|certificate| {
            certificate.not_after_ms > now_ms
                && certificate.not_after_ms <= now_ms.saturating_add(30 * 24 * 60 * 60 * 1_000)
        })
        .count();
    let earliest_expiry = certificates
        .iter()
        .map(|certificate| certificate.not_after_ms)
        .min()
        .map_or(0, |expiry| expiry / 1_000);
    write!(
        body,
        "# HELP senix_certificate_store_enabled Whether encrypted certificate storage is configured.\n\
# TYPE senix_certificate_store_enabled gauge\n\
senix_certificate_store_enabled {}\n\
# HELP senix_certificates_active Active certificates loaded by the TLS data plane.\n\
# TYPE senix_certificates_active gauge\n\
senix_certificates_active {}\n\
# HELP senix_certificates_expired Active certificates already expired.\n\
# TYPE senix_certificates_expired gauge\n\
senix_certificates_expired {}\n\
# HELP senix_certificates_expiring_within_30_days Active certificates expiring within 30 days.\n\
# TYPE senix_certificates_expiring_within_30_days gauge\n\
senix_certificates_expiring_within_30_days {}\n\
# HELP senix_certificate_earliest_expiry_timestamp_seconds Earliest active certificate expiry as a Unix timestamp.\n\
# TYPE senix_certificate_earliest_expiry_timestamp_seconds gauge\n\
senix_certificate_earliest_expiry_timestamp_seconds {}\n",
        u8::from(state.certificates.is_some()),
        certificates.len(),
        expired,
        expiring,
        earliest_expiry,
    )
    .expect("writing metrics to a String cannot fail");
}

fn prometheus_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

const fn traffic_state_label(state: senix_core::TrafficState) -> &'static str {
    match state {
        senix_core::TrafficState::Serving => "SERVING",
        senix_core::TrafficState::Draining => "DRAINING",
        senix_core::TrafficState::Drained => "DRAINED",
        senix_core::TrafficState::Disabled => "DISABLED",
    }
}

const fn health_state_label(state: senix_core::HealthState) -> &'static str {
    match state {
        senix_core::HealthState::Unknown => "UNKNOWN",
        senix_core::HealthState::Healthy => "HEALTHY",
        senix_core::HealthState::Unhealthy => "UNHEALTHY",
    }
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

async fn list_certificates(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<senix_core::CertificateSummary>>, ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::CertificateRead,
        &ResourceRef::Global,
    )?;
    let certificates = state.certificates.as_ref().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "CERTIFICATE_STORE_DISABLED",
        message: "certificate storage requires --secret-key-file".to_owned(),
        evidence: json!({}),
    })?;
    Ok(Json(certificates.list()?))
}

async fn issue_certificate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<IssueCertificateRequest>,
) -> Result<(StatusCode, Json<certificate::IssueResult>), ApiError> {
    state.security.authorize(
        &principal,
        ManagementAction::CertificateIssue,
        &ResourceRef::Global,
    )?;
    if !(10..=300).contains(&body.timeout_seconds) {
        return Err(ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "INVALID_ACME_TIMEOUT",
            message: "timeout_seconds must be between 10 and 300".to_owned(),
            evidence: json!({"timeout_seconds": body.timeout_seconds}),
        });
    }
    let acme = state.acme.as_ref().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "ACME_DISABLED",
        message: "ACME issuance is not configured".to_owned(),
        evidence: json!({}),
    })?;
    let domains = body.domains;
    match acme
        .issue(senix_acme::IssueRequest {
            domains: domains.clone(),
            timeout: std::time::Duration::from_secs(body.timeout_seconds),
        })
        .await
    {
        Ok(result) => {
            state.security.record_action(
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
            )?;
            Ok((StatusCode::CREATED, Json(result)))
        }
        Err(error) => {
            state.security.record_action(
                &principal,
                ManagementAction::CertificateIssue.as_str(),
                &ResourceRef::Global,
                AuditOutcome::Failed,
                RiskLevel::High,
                json!({"domains": domains}),
            )?;
            Err(error.into())
        }
    }
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
