use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    path::Path,
    sync::Arc,
};

use arc_swap::ArcSwap;
use async_trait::async_trait;
use openssl::asn1::{Asn1Time, Asn1TimeRef};
use pingora_core::{
    listeners::TlsAccept,
    protocols::tls::TlsRef,
    tls::{ext, nid::Nid, pkey::PKey, ssl::NameType, x509::X509},
};
use thiserror::Error;
use tracing::error;

#[derive(Debug, Error)]
pub enum TlsCertificateError {
    #[error("read certificate file {path}: {source}")]
    ReadCertificate {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("read private key file {path}: {source}")]
    ReadPrivateKey {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("certificate chain is not valid PEM: {0}")]
    InvalidCertificate(#[source] pingora_core::tls::error::ErrorStack),
    #[error("private key is not valid PEM: {0}")]
    InvalidPrivateKey(#[source] pingora_core::tls::error::ErrorStack),
    #[error("certificate does not contain a supported DNS name")]
    MissingDnsName,
    #[error("certificate contains an invalid DNS name: {0}")]
    InvalidDnsName(String),
    #[error("certificate and private key do not match")]
    KeyMismatch,
}

#[derive(Clone)]
struct PreparedCertificate {
    leaf: X509,
    chain: Vec<X509>,
    private_key: PKey<pingora_core::tls::pkey::Private>,
    domains: Arc<[String]>,
}

impl fmt::Debug for PreparedCertificate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedCertificate")
            .field("domains", &self.domains)
            .field("chain_length", &self.chain.len().saturating_add(1))
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
struct CertificateSet {
    generation: u64,
    by_domain: HashMap<String, Arc<PreparedCertificate>>,
    default: Option<Arc<PreparedCertificate>>,
}

/// Atomically replaceable SNI certificate set shared with the TLS handshake callback.
#[derive(Clone, Debug, Default)]
pub struct TlsCertificateRegistry {
    state: Arc<ArcSwap<CertificateSet>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledCertificate {
    pub generation: u64,
    pub domains: Arc<[String]>,
    pub not_before_ms: i64,
    pub not_after_ms: i64,
}

/// Fully parsed certificate material that can be persisted before its infallible publication.
#[derive(Clone, Debug)]
pub struct PreparedTlsCertificate(Arc<PreparedCertificate>);

impl PreparedTlsCertificate {
    #[must_use]
    pub fn domains(&self) -> &[String] {
        &self.0.domains
    }

    #[must_use]
    pub fn not_before_ms(&self) -> i64 {
        asn1_unix_ms(self.0.leaf.not_before()).unwrap_or(i64::MIN)
    }

    #[must_use]
    pub fn not_after_ms(&self) -> i64 {
        asn1_unix_ms(self.0.leaf.not_after()).unwrap_or(i64::MAX)
    }
}

impl TlsCertificateRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses and installs one certificate chain. Readers see either the old or new complete set.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed PEM, unsupported DNS names, or a mismatched private key.
    pub fn install_pem(
        &self,
        certificate_chain_pem: &[u8],
        private_key_pem: &[u8],
        make_default: bool,
    ) -> Result<InstalledCertificate, TlsCertificateError> {
        let prepared = Self::prepare_pem(certificate_chain_pem, private_key_pem)?;
        Ok(self.install_prepared(&prepared, make_default))
    }

    /// Parses certificate material without changing the active set.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed PEM, unsupported DNS names, or a mismatched private key.
    pub fn prepare_pem(
        certificate_chain_pem: &[u8],
        private_key_pem: &[u8],
    ) -> Result<PreparedTlsCertificate, TlsCertificateError> {
        let mut certificates = X509::stack_from_pem(certificate_chain_pem)
            .map_err(TlsCertificateError::InvalidCertificate)?;
        if certificates.is_empty() {
            return Err(TlsCertificateError::MissingDnsName);
        }
        let leaf = certificates.remove(0);
        let private_key = PKey::private_key_from_pem(private_key_pem)
            .map_err(TlsCertificateError::InvalidPrivateKey)?;
        let public_key = leaf
            .public_key()
            .map_err(TlsCertificateError::InvalidCertificate)?;
        if !public_key.public_eq(&private_key) {
            return Err(TlsCertificateError::KeyMismatch);
        }
        let domains = Arc::<[String]>::from(certificate_domains(&leaf)?);
        Ok(PreparedTlsCertificate(Arc::new(PreparedCertificate {
            leaf,
            chain: certificates,
            private_key,
            domains: Arc::clone(&domains),
        })))
    }

    #[must_use]
    pub fn install_prepared(
        &self,
        prepared: &PreparedTlsCertificate,
        make_default: bool,
    ) -> InstalledCertificate {
        let domains = Arc::clone(&prepared.0.domains);
        let current = self.state.load_full();
        let mut next = (*current).clone();
        next.generation = next.generation.saturating_add(1);
        for domain in domains.iter() {
            next.by_domain
                .insert(domain.clone(), Arc::clone(&prepared.0));
        }
        if make_default || next.default.is_none() {
            next.default = Some(Arc::clone(&prepared.0));
        }
        let generation = next.generation;
        self.state.store(Arc::new(next));

        InstalledCertificate {
            generation,
            domains,
            not_before_ms: prepared.not_before_ms(),
            not_after_ms: prepared.not_after_ms(),
        }
    }

    /// Loads and installs PEM files without retaining their paths.
    ///
    /// # Errors
    ///
    /// Returns an error when either file cannot be read or its contents are invalid.
    pub fn install_files(
        &self,
        certificate_path: &Path,
        private_key_path: &Path,
        make_default: bool,
    ) -> Result<InstalledCertificate, TlsCertificateError> {
        let certificate_chain_pem =
            fs::read(certificate_path).map_err(|source| TlsCertificateError::ReadCertificate {
                path: certificate_path.display().to_string(),
                source,
            })?;
        let private_key_pem =
            fs::read(private_key_path).map_err(|source| TlsCertificateError::ReadPrivateKey {
                path: private_key_path.display().to_string(),
                source,
            })?;
        self.install_pem(&certificate_chain_pem, &private_key_pem, make_default)
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.state.load().generation
    }

    /// Returns each distinct certificate currently reachable by SNI exactly once.
    #[must_use]
    pub fn active_certificates(&self) -> Vec<InstalledCertificate> {
        let state = self.state.load();
        let mut seen = HashSet::new();
        let mut certificates = state
            .by_domain
            .values()
            .filter(|certificate| seen.insert(Arc::as_ptr(certificate).cast::<()>() as usize))
            .map(|certificate| InstalledCertificate {
                generation: state.generation,
                domains: Arc::clone(&certificate.domains),
                not_before_ms: asn1_unix_ms(certificate.leaf.not_before()).unwrap_or(i64::MIN),
                not_after_ms: asn1_unix_ms(certificate.leaf.not_after()).unwrap_or(i64::MAX),
            })
            .collect::<Vec<_>>();
        certificates.sort_unstable_by(|left, right| left.domains.cmp(&right.domains));
        certificates
    }

    fn select(&self, server_name: Option<&str>) -> Option<Arc<PreparedCertificate>> {
        let state = self.state.load();
        let server_name = server_name.and_then(normalize_server_name);
        server_name
            .as_deref()
            .and_then(|name| {
                state.by_domain.get(name).or_else(|| {
                    let (_, suffix) = name.split_once('.')?;
                    state.by_domain.get(&format!("*.{suffix}"))
                })
            })
            .cloned()
            .or_else(|| state.default.clone())
    }
}

#[async_trait]
impl TlsAccept for TlsCertificateRegistry {
    async fn certificate_callback(&self, ssl: &mut TlsRef) {
        let server_name = ssl.servername(NameType::HOST_NAME).map(str::to_owned);
        let Some(certificate) = self.select(server_name.as_deref()) else {
            error!(server_name, "TLS handshake has no installed certificate");
            return;
        };
        if let Err(cause) = install_for_handshake(ssl, &certificate) {
            error!(server_name, error = %cause, "failed to install TLS certificate for handshake");
        }
    }
}

fn install_for_handshake(
    ssl: &mut TlsRef,
    certificate: &PreparedCertificate,
) -> Result<(), pingora_core::tls::error::ErrorStack> {
    ext::ssl_use_certificate(ssl, &certificate.leaf)?;
    ext::ssl_use_private_key(ssl, &certificate.private_key)?;
    for chain_certificate in &certificate.chain {
        ssl.add_chain_cert(chain_certificate.clone())?;
    }
    Ok(())
}

fn certificate_domains(certificate: &X509) -> Result<Vec<String>, TlsCertificateError> {
    let mut domains = Vec::new();
    if let Some(subject_alt_names) = certificate.subject_alt_names() {
        for name in subject_alt_names {
            if let Some(domain) = name.dnsname() {
                domains.push(normalize_certificate_name(domain)?);
            }
        }
    }
    if domains.is_empty()
        && let Some(common_name) = certificate
            .subject_name()
            .entries_by_nid(Nid::COMMONNAME)
            .next()
            .and_then(|entry| entry.data().to_string().ok())
    {
        domains.push(normalize_certificate_name(&common_name)?);
    }
    domains.sort_unstable();
    domains.dedup();
    if domains.is_empty() {
        return Err(TlsCertificateError::MissingDnsName);
    }
    Ok(domains)
}

fn normalize_certificate_name(name: &str) -> Result<String, TlsCertificateError> {
    let normalized = name.trim_end_matches('.').to_ascii_lowercase();
    let labels = normalized
        .strip_prefix("*.")
        .unwrap_or(normalized.as_str())
        .split('.');
    if normalized.is_empty()
        || normalized.len() > 253
        || normalized == "*"
        || labels.clone().count() < 2
        || labels.into_iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(TlsCertificateError::InvalidDnsName(name.to_owned()));
    }
    Ok(normalized)
}

fn normalize_server_name(name: &str) -> Option<String> {
    let normalized = name.trim_end_matches('.').to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn asn1_unix_ms(time: &Asn1TimeRef) -> Option<i64> {
    let epoch = Asn1Time::from_unix(0).ok()?;
    let difference = epoch.diff(time).ok()?;
    let seconds = i64::from(difference.days)
        .checked_mul(86_400)?
        .checked_add(i64::from(difference.secs))?;
    seconds.checked_mul(1_000)
}

#[cfg(test)]
mod tests {
    use super::TlsCertificateRegistry;
    use openssl::{
        asn1::Asn1Time,
        bn::{BigNum, MsbOption},
        hash::MessageDigest,
        nid::Nid,
        pkey::PKey,
        rsa::Rsa,
        x509::{X509, X509NameBuilder, extension::SubjectAlternativeName},
    };

    fn certificate(names: &[&str]) -> (Vec<u8>, Vec<u8>) {
        let key = PKey::from_rsa(Rsa::generate(2_048).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_nid(Nid::COMMONNAME, names[0]).unwrap();
        let name = name.build();
        let mut serial = BigNum::new().unwrap();
        serial.rand(64, MsbOption::MAYBE_ZERO, false).unwrap();
        let serial = serial.to_asn1_integer().unwrap();
        let mut builder = X509::builder().unwrap();
        builder.set_version(2).unwrap();
        builder.set_serial_number(&serial).unwrap();
        builder.set_subject_name(&name).unwrap();
        builder.set_issuer_name(&name).unwrap();
        builder.set_pubkey(&key).unwrap();
        builder
            .set_not_before(Asn1Time::days_from_now(0).unwrap().as_ref())
            .unwrap();
        builder
            .set_not_after(Asn1Time::days_from_now(30).unwrap().as_ref())
            .unwrap();
        let mut subject_alt_name = SubjectAlternativeName::new();
        for name in names {
            subject_alt_name.dns(name);
        }
        let extension = subject_alt_name
            .build(&builder.x509v3_context(None, None))
            .unwrap();
        builder.append_extension(extension).unwrap();
        builder.sign(&key, MessageDigest::sha256()).unwrap();
        (
            builder.build().to_pem().unwrap(),
            key.private_key_to_pem_pkcs8().unwrap(),
        )
    }

    #[test]
    fn exact_wildcard_and_default_selection_are_deterministic() {
        let registry = TlsCertificateRegistry::new();
        let (default_cert, default_key) = certificate(&["default.test"]);
        registry
            .install_pem(&default_cert, &default_key, true)
            .unwrap();
        let (wildcard_cert, wildcard_key) = certificate(&["*.example.test"]);
        registry
            .install_pem(&wildcard_cert, &wildcard_key, false)
            .unwrap();

        assert_eq!(
            registry.select(Some("api.example.test")).unwrap().domains[0],
            "*.example.test"
        );
        assert_eq!(
            registry
                .select(Some("deep.api.example.test"))
                .unwrap()
                .domains[0],
            "default.test"
        );
        assert_eq!(
            registry.select(Some("unknown.test")).unwrap().domains[0],
            "default.test"
        );
    }

    #[test]
    fn replacement_is_atomic_and_old_snapshot_remains_valid() {
        let registry = TlsCertificateRegistry::new();
        let (old_cert, old_key) = certificate(&["example.test"]);
        let old = registry.install_pem(&old_cert, &old_key, true).unwrap();
        let old_snapshot = registry.select(Some("example.test")).unwrap();
        let (new_cert, new_key) = certificate(&["example.test", "www.example.test"]);
        let new = registry.install_pem(&new_cert, &new_key, true).unwrap();

        assert_eq!(new.generation, old.generation + 1);
        assert_eq!(&*old_snapshot.domains, &["example.test".to_owned()]);
        let active = registry.active_certificates();
        assert_eq!(active.len(), 1);
        assert_eq!(
            &*active[0].domains,
            &["example.test".to_owned(), "www.example.test".to_owned()]
        );
        assert_eq!(
            &*registry.select(Some("example.test")).unwrap().domains,
            &["example.test".to_owned(), "www.example.test".to_owned()]
        );
    }

    #[test]
    fn rejects_a_mismatched_private_key_without_changing_generation() {
        let registry = TlsCertificateRegistry::new();
        let (certificate_pem, _) = certificate(&["example.test"]);
        let (_, wrong_key) = certificate(&["other.test"]);

        assert!(
            registry
                .install_pem(&certificate_pem, &wrong_key, true)
                .is_err()
        );
        assert_eq!(registry.generation(), 0);
    }
}
