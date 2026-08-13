//! ACME protocol adapter for Senix certificate issuance.

use std::{fmt, sync::Arc, time::Duration};

use instant_acme::{
    Account, AccountBuilder, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier,
    NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use senix_core::{Http01ChallengeGuard, Http01ChallengeRegistry};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("ACME account request failed: {0}")]
    Account(#[source] instant_acme::Error),
    #[error("ACME account credentials are invalid: {0}")]
    InvalidCredentials(#[source] serde_json::Error),
    #[error("ACME account configuration is invalid: {0}")]
    InvalidAccountConfig(String),
    #[error("ACME domain list is invalid: {0}")]
    InvalidDomains(String),
    #[error("ACME authorization for {domain} has unsupported status {status:?}")]
    AuthorizationStatus {
        domain: String,
        status: AuthorizationStatus,
    },
    #[error("ACME server did not offer HTTP-01 for {0}")]
    Http01Unavailable(String),
    #[error("publish HTTP-01 response for {domain}: {source}")]
    PublishChallenge {
        domain: String,
        #[source]
        source: senix_core::Error,
    },
    #[error("ACME order entered unexpected status {0:?}")]
    UnexpectedOrderStatus(OrderStatus),
}

impl From<instant_acme::Error> for Error {
    fn from(source: instant_acme::Error) -> Self {
        Self::Account(source)
    }
}

/// Serialized ACME account identity and private key. Debug output is always redacted.
#[derive(Clone)]
pub struct AccountSecret(Arc<[u8]>);

impl AccountSecret {
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for AccountSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AccountSecret([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountConfig {
    pub directory_url: String,
    pub contacts: Vec<String>,
    pub terms_of_service_agreed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueRequest {
    pub domains: Vec<String>,
    pub timeout: Duration,
}

impl IssueRequest {
    #[must_use]
    pub fn new(domains: Vec<String>) -> Self {
        Self {
            domains,
            timeout: Duration::from_secs(90),
        }
    }
}

pub struct IssuedCertificate {
    pub domains: Arc<[String]>,
    pub certificate_chain_pem: String,
    pub private_key_pem: String,
}

impl fmt::Debug for IssuedCertificate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedCertificate")
            .field("domains", &self.domains)
            .field("certificate_chain_pem", &"[REDACTED]")
            .field("private_key_pem", &"[REDACTED]")
            .finish()
    }
}

/// ACME account with a narrow, HTTP-01-only issuance interface.
#[derive(Clone)]
pub struct Http01Issuer {
    account: Account,
    challenges: Http01ChallengeRegistry,
}

impl fmt::Debug for Http01Issuer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Http01Issuer")
            .finish_non_exhaustive()
    }
}

impl Http01Issuer {
    /// Creates a new ACME account and returns the credentials that must be stored securely.
    ///
    /// # Errors
    ///
    /// Returns an error when contacts are malformed, terms are not accepted, or the ACME server
    /// cannot create the account.
    pub async fn create(
        config: AccountConfig,
        challenges: Http01ChallengeRegistry,
    ) -> Result<(Self, AccountSecret), Error> {
        Self::create_with_builder(config, challenges, Account::builder()?).await
    }

    async fn create_with_builder(
        config: AccountConfig,
        challenges: Http01ChallengeRegistry,
        builder: AccountBuilder,
    ) -> Result<(Self, AccountSecret), Error> {
        validate_account_config(&config)?;
        let contacts = config
            .contacts
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let (account, credentials) = builder
            .create(
                &NewAccount {
                    contact: &contacts,
                    terms_of_service_agreed: config.terms_of_service_agreed,
                    only_return_existing: false,
                },
                config.directory_url,
                None,
            )
            .await?;
        let secret = AccountSecret::from_bytes(
            serde_json::to_vec(&credentials).map_err(Error::InvalidCredentials)?,
        );
        Ok((
            Self {
                account,
                challenges,
            },
            secret,
        ))
    }

    /// Restores an ACME account from opaque serialized credentials.
    ///
    /// # Errors
    ///
    /// Returns an error when the credentials are malformed or their directory cannot be loaded.
    pub async fn restore(
        secret: &AccountSecret,
        challenges: Http01ChallengeRegistry,
    ) -> Result<Self, Error> {
        let credentials: AccountCredentials =
            serde_json::from_slice(secret.expose()).map_err(Error::InvalidCredentials)?;
        let account = Account::builder()?.from_credentials(credentials).await?;
        Ok(Self {
            account,
            challenges,
        })
    }

    /// Issues a certificate after publishing domain-bound HTTP-01 responses in the data plane.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid names, failed authorizations, timeouts, or CA failures.
    pub async fn issue(&self, request: IssueRequest) -> Result<IssuedCertificate, Error> {
        let domains = normalize_domains(request.domains)?;
        let identifiers = domains
            .iter()
            .cloned()
            .map(Identifier::Dns)
            .collect::<Vec<_>>();
        let mut order = self.account.new_order(&NewOrder::new(&identifiers)).await?;
        let mut challenge_guards: Vec<Http01ChallengeGuard> = Vec::new();

        {
            let mut authorizations = order.authorizations();
            while let Some(result) = authorizations.next().await {
                let mut authorization = result?;
                let domain = authorization.identifier().to_string();
                match authorization.status {
                    AuthorizationStatus::Valid => continue,
                    AuthorizationStatus::Pending => {}
                    status => {
                        return Err(Error::AuthorizationStatus { domain, status });
                    }
                }
                let mut challenge = authorization
                    .challenge(ChallengeType::Http01)
                    .ok_or_else(|| Error::Http01Unavailable(domain.clone()))?;
                let token = challenge.token.clone();
                let key_authorization = challenge.key_authorization();
                let guard = self
                    .challenges
                    .publish(&domain, &token, key_authorization.as_str())
                    .map_err(|source| Error::PublishChallenge {
                        domain: domain.clone(),
                        source,
                    })?;
                challenge.set_ready().await?;
                challenge_guards.push(guard);
            }
        }

        let retries = RetryPolicy::default().timeout(request.timeout);
        let status = order.poll_ready(&retries).await?;
        if status != OrderStatus::Ready {
            return Err(Error::UnexpectedOrderStatus(status));
        }
        drop(challenge_guards);

        let private_key_pem = order.finalize().await?;
        let certificate_chain_pem = order.poll_certificate(&retries).await?;
        Ok(IssuedCertificate {
            domains: Arc::from(domains),
            certificate_chain_pem,
            private_key_pem,
        })
    }
}

fn validate_account_config(config: &AccountConfig) -> Result<(), Error> {
    if !config.terms_of_service_agreed {
        return Err(Error::InvalidAccountConfig(
            "terms of service must be accepted explicitly".to_owned(),
        ));
    }
    if !config.directory_url.starts_with("https://") || config.directory_url.len() > 2_048 {
        return Err(Error::InvalidAccountConfig(
            "directory URL must be an HTTPS URL".to_owned(),
        ));
    }
    if config.contacts.len() > 5
        || config.contacts.iter().any(|contact| {
            !contact.starts_with("mailto:")
                || contact.len() > 320
                || contact.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
        })
    {
        return Err(Error::InvalidAccountConfig(
            "contacts must contain at most five mailto URIs".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_domains(domains: Vec<String>) -> Result<Vec<String>, Error> {
    if domains.is_empty() || domains.len() > 100 {
        return Err(Error::InvalidDomains(
            "an order must contain 1-100 DNS names".to_owned(),
        ));
    }
    let mut normalized = domains
        .into_iter()
        .map(|domain| normalize_domain(&domain))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort_unstable();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_domain(domain: &str) -> Result<String, Error> {
    let normalized = domain.trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.len() > 253
        || normalized.starts_with("*.")
        || normalized.split('.').count() < 2
        || normalized.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(Error::InvalidDomains(format!(
            "HTTP-01 requires a non-wildcard DNS name, got {domain}"
        )));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        time::Duration,
    };

    use bytes::Bytes;
    use http::{Method, Response, StatusCode, header};
    use instant_acme::{BodyWrapper, BytesResponse, Error as AcmeError, HttpClient};
    use senix_core::Http01ChallengeRegistry;

    use super::{AccountConfig, AccountSecret, Http01Issuer, IssueRequest, normalize_domains};

    #[test]
    fn normalizes_and_deduplicates_http01_names() {
        let domains = normalize_domains(vec![
            "WWW.Example.TEST.".to_owned(),
            "example.test".to_owned(),
            "www.example.test".to_owned(),
        ])
        .unwrap();
        assert_eq!(domains, ["example.test", "www.example.test"]);
    }

    #[test]
    fn rejects_wildcard_and_single_label_names() {
        assert!(normalize_domains(vec!["*.example.test".to_owned()]).is_err());
        assert!(normalize_domains(vec!["localhost".to_owned()]).is_err());
    }

    #[test]
    fn secrets_and_issued_keys_are_not_in_debug_output() {
        let secret = AccountSecret::from_bytes(Vec::from("private-account-key"));
        assert_eq!(format!("{secret:?}"), "AccountSecret([REDACTED])");
        let request = IssueRequest::new(vec!["example.test".to_owned()]);
        assert_eq!(request.timeout.as_secs(), 90);
    }

    #[tokio::test]
    async fn completes_the_http01_order_and_limits_challenge_lifetime() {
        let challenges = Http01ChallengeRegistry::new();
        let mock = Arc::new(MockAcme::new(challenges.clone()));
        let builder =
            instant_acme::Account::builder_with_http(Box::new(ArcClient(Arc::clone(&mock))));
        let (issuer, secret) = Http01Issuer::create_with_builder(
            AccountConfig {
                directory_url: "https://acme.test/directory".to_owned(),
                contacts: vec!["mailto:ops@example.test".to_owned()],
                terms_of_service_agreed: true,
            },
            challenges.clone(),
            builder,
        )
        .await
        .unwrap();
        assert!(!secret.expose().is_empty());

        let certificate = issuer
            .issue(IssueRequest {
                domains: vec!["example.test".to_owned()],
                timeout: Duration::from_secs(3),
            })
            .await
            .unwrap();
        assert!(mock.challenge_was_visible.load(Ordering::SeqCst));
        assert!(
            certificate
                .certificate_chain_pem
                .contains("BEGIN CERTIFICATE")
        );
        assert!(certificate.private_key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(
            challenges
                .resolve("example.test", "/.well-known/acme-challenge/http01-token")
                .is_none()
        );
    }

    struct MockAcme {
        challenges: Http01ChallengeRegistry,
        challenge_was_visible: AtomicBool,
        finalized: AtomicBool,
        nonce: AtomicU64,
    }

    impl MockAcme {
        fn new(challenges: Http01ChallengeRegistry) -> Self {
            Self {
                challenges,
                challenge_was_visible: AtomicBool::new(false),
                finalized: AtomicBool::new(false),
                nonce: AtomicU64::new(0),
            }
        }

        fn response(&self, method: &Method, path: &str) -> Response<BodyWrapper<Bytes>> {
            let (status, location, body) = match (method, path) {
                (&Method::GET, "/directory") => (
                    StatusCode::OK,
                    None,
                    r#"{"newNonce":"https://acme.test/nonce","newAccount":"https://acme.test/new-account","newOrder":"https://acme.test/new-order"}"#.to_owned(),
                ),
                (&Method::HEAD, "/nonce") => (StatusCode::OK, None, String::new()),
                (&Method::POST, "/new-account") => (
                    StatusCode::CREATED,
                    Some("https://acme.test/account/1"),
                    "{}".to_owned(),
                ),
                (&Method::POST, "/new-order") => (
                    StatusCode::CREATED,
                    Some("https://acme.test/order/1"),
                    order("pending", None),
                ),
                (&Method::POST, "/authorization/1") => (
                    StatusCode::OK,
                    None,
                    r#"{"identifier":{"type":"dns","value":"example.test"},"status":"pending","challenges":[{"type":"http-01","url":"https://acme.test/challenge/1","token":"http01-token","status":"pending"}],"wildcard":false}"#.to_owned(),
                ),
                (&Method::POST, "/challenge/1") => {
                    let visible = self
                        .challenges
                        .resolve(
                            "example.test",
                            "/.well-known/acme-challenge/http01-token",
                        )
                        .is_some_and(|value| value.starts_with("http01-token."));
                    self.challenge_was_visible.store(visible, Ordering::SeqCst);
                    (
                        StatusCode::OK,
                        None,
                        r#"{"type":"http-01","url":"https://acme.test/challenge/1","token":"http01-token","status":"pending"}"#.to_owned(),
                    )
                }
                (&Method::POST, "/order/1") => {
                    if self.finalized.load(Ordering::SeqCst) {
                        (
                            StatusCode::OK,
                            None,
                            order("valid", Some("https://acme.test/certificate/1")),
                        )
                    } else {
                        (StatusCode::OK, None, order("ready", None))
                    }
                }
                (&Method::POST, "/finalize/1") => {
                    self.finalized.store(true, Ordering::SeqCst);
                    (
                        StatusCode::OK,
                        None,
                        order("processing", Some("https://acme.test/certificate/1")),
                    )
                }
                (&Method::POST, "/certificate/1") => (
                    StatusCode::OK,
                    None,
                    "-----BEGIN CERTIFICATE-----\nTU9DSw==\n-----END CERTIFICATE-----\n"
                        .to_owned(),
                ),
                _ => panic!("unexpected ACME request: {method} {path}"),
            };
            let mut builder = Response::builder().status(status).header(
                "Replay-Nonce",
                format!("nonce-{}", self.nonce.fetch_add(1, Ordering::SeqCst)),
            );
            if let Some(location) = location {
                builder = builder.header(header::LOCATION, location);
            }
            builder.body(BodyWrapper::from(body.into_bytes())).unwrap()
        }
    }

    #[derive(Clone)]
    struct ArcClient(Arc<MockAcme>);

    impl HttpClient for ArcClient {
        fn request(
            &self,
            request: http::Request<BodyWrapper<Bytes>>,
        ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, AcmeError>> + Send>> {
            let response = self.0.response(request.method(), request.uri().path());
            Box::pin(async move { Ok(BytesResponse::from(response)) })
        }
    }

    fn order(status: &str, certificate: Option<&str>) -> String {
        serde_json::json!({
            "status": status,
            "authorizations": ["https://acme.test/authorization/1"],
            "finalize": "https://acme.test/finalize/1",
            "certificate": certificate
        })
        .to_string()
    }
}
