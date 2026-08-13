use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use senix_acme::{AccountConfig, AccountSecret, Http01Issuer, IssueRequest};
use senix_core::{
    CertificateController, CertificateMaterial, CertificateSummary, Http01ChallengeRegistry,
    SecretBytes,
};
use senix_pingora::{TlsCertificateError, TlsCertificateRegistry};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::Mutex;

const ACME_ACCOUNT_SECRET: &str = "acme-account-v1";

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Acme(#[from] senix_acme::Error),
    #[error(transparent)]
    Core(#[from] senix_core::Error),
    #[error(transparent)]
    Tls(#[from] TlsCertificateError),
    #[error("issued certificate names do not match the requested names")]
    DomainMismatch,
}

#[derive(Clone, Debug, Serialize)]
pub struct IssueResult {
    pub certificate: CertificateSummary,
    pub tls_generation: u64,
}

#[derive(Debug)]
pub struct AcmeManager {
    issuer: Mutex<Option<Http01Issuer>>,
    account_config: AccountConfig,
    challenges: Http01ChallengeRegistry,
    certificates: Arc<CertificateController>,
    tls: TlsCertificateRegistry,
}

#[derive(Debug)]
pub struct McpCertificateManager {
    certificates: Arc<CertificateController>,
    acme: Option<Arc<AcmeManager>>,
}

impl McpCertificateManager {
    #[must_use]
    pub fn new(certificates: Arc<CertificateController>, acme: Option<Arc<AcmeManager>>) -> Self {
        Self { certificates, acme }
    }
}

#[async_trait]
impl senix_mcp::CertificateManagement for McpCertificateManager {
    fn list(&self) -> senix_core::Result<Vec<CertificateSummary>> {
        self.certificates.list()
    }

    async fn issue(
        &self,
        domains: Vec<String>,
        timeout: std::time::Duration,
    ) -> Result<senix_mcp::CertificateIssueResult, senix_mcp::CertificateToolError> {
        let acme = self.acme.as_ref().ok_or_else(|| {
            senix_mcp::CertificateToolError::unavailable("ACME issuance is not configured")
        })?;
        let result = AcmeManager::issue(acme, IssueRequest { domains, timeout })
            .await
            .map_err(|_| senix_mcp::CertificateToolError::issuance_failed())?;
        Ok(senix_mcp::CertificateIssueResult {
            certificate: result.certificate,
            tls_generation: result.tls_generation,
        })
    }
}

impl AcmeManager {
    #[must_use]
    pub fn new(
        account_config: AccountConfig,
        challenges: Http01ChallengeRegistry,
        certificates: Arc<CertificateController>,
        tls: TlsCertificateRegistry,
    ) -> Self {
        Self {
            issuer: Mutex::new(None),
            account_config,
            challenges,
            certificates,
            tls,
        }
    }

    pub async fn issue(&self, request: IssueRequest) -> Result<IssueResult, Error> {
        let mut issuer_guard = self.issuer.lock().await;
        if issuer_guard.is_none() {
            *issuer_guard = Some(self.load_or_create_issuer().await?);
        }
        let issued_certificate = issuer_guard
            .as_ref()
            .expect("issuer is initialized")
            .issue(request)
            .await?;
        let prepared = TlsCertificateRegistry::prepare_pem(
            issued_certificate.certificate_chain_pem.as_bytes(),
            issued_certificate.private_key_pem.as_bytes(),
        )?;
        let requested = issued_certificate
            .domains
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let certificate_names = prepared.domains().iter().cloned().collect::<BTreeSet<_>>();
        if requested != certificate_names {
            return Err(Error::DomainMismatch);
        }
        let summary = self.certificates.replace(CertificateMaterial {
            domains: issued_certificate.domains.to_vec(),
            certificate_chain_pem: Arc::from(issued_certificate.certificate_chain_pem.as_bytes()),
            private_key_pem: SecretBytes::new(issued_certificate.private_key_pem.into_bytes()),
            not_before_ms: prepared.not_before_ms(),
            not_after_ms: prepared.not_after_ms(),
        })?;
        let installed = self.tls.install_prepared(&prepared, false);
        Ok(IssueResult {
            certificate: summary,
            tls_generation: installed.generation,
        })
    }

    async fn load_or_create_issuer(&self) -> Result<Http01Issuer, Error> {
        if let Some(secret) = self.certificates.load_secret(ACME_ACCOUNT_SECRET)? {
            return Ok(Http01Issuer::restore(
                &AccountSecret::from_bytes(Arc::<[u8]>::from(secret.expose())),
                self.challenges.clone(),
            )
            .await?);
        }
        let (issuer, secret) =
            Http01Issuer::create(self.account_config.clone(), self.challenges.clone()).await?;
        self.certificates
            .save_secret(ACME_ACCOUNT_SECRET, secret.expose())?;
        Ok(issuer)
    }
}
