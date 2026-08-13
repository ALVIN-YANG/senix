use std::{
    collections::HashMap,
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, Generate, Key, KeyInit, Payload},
};
use parking_lot::Mutex;
use serde::Serialize;
use uuid::Uuid;

use crate::{Error, Result, SqliteStateStore};

const SECRET_KEY_LENGTH: usize = 32;

/// Master key for encrypting durable certificate private keys and external credentials.
#[derive(Clone)]
pub struct SecretVault {
    cipher: Arc<XChaCha20Poly1305>,
}

impl SecretVault {
    /// Parses a URL-safe base64 encoded 256-bit master key.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not valid base64 or does not decode to 32 bytes.
    pub fn from_base64(encoded: &str) -> Result<Self> {
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded.trim())
            .map_err(|error| Error::Crypto(format!("secret key is not valid base64: {error}")))?;
        let key: [u8; SECRET_KEY_LENGTH] = decoded.try_into().map_err(|decoded: Vec<u8>| {
            Error::Crypto(format!(
                "secret key must decode to {SECRET_KEY_LENGTH} bytes, got {}",
                decoded.len()
            ))
        })?;
        Ok(Self {
            cipher: Arc::new(XChaCha20Poly1305::new(&Key::<XChaCha20Poly1305>::from(key))),
        })
    }

    #[must_use]
    pub fn generate_base64() -> String {
        URL_SAFE_NO_PAD.encode(Key::<XChaCha20Poly1305>::generate())
    }

    fn seal(&self, context: &str, plaintext: &[u8]) -> Result<SealedSecret> {
        let nonce = XNonce::generate();
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| Error::Crypto("could not encrypt secret".to_owned()))?;
        Ok(SealedSecret {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    fn open(&self, context: &str, sealed: &SealedSecret) -> Result<SecretBytes> {
        let nonce: [u8; 24] = sealed.nonce.as_slice().try_into().map_err(|_| {
            Error::InvalidState("stored secret nonce has an invalid length".to_owned())
        })?;
        let plaintext = self
            .cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| Error::Crypto("could not decrypt stored secret".to_owned()))?;
        Ok(SecretBytes::new(plaintext))
    }
}

impl fmt::Debug for SecretVault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretVault([REDACTED])")
    }
}

#[derive(Clone)]
pub struct SecretBytes(Arc<[u8]>);

impl SecretBytes {
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretBytes([REDACTED])")
    }
}

#[derive(Clone, Debug)]
pub struct CertificateMaterial {
    pub domains: Vec<String>,
    pub certificate_chain_pem: Arc<[u8]>,
    pub private_key_pem: SecretBytes,
    pub not_before_ms: i64,
    pub not_after_ms: i64,
}

#[derive(Clone, Debug)]
pub struct ManagedCertificate {
    pub certificate_id: Uuid,
    pub domains: Vec<String>,
    pub certificate_chain_pem: Arc<[u8]>,
    pub private_key_pem: SecretBytes,
    pub not_before_ms: i64,
    pub not_after_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CertificateSummary {
    pub certificate_id: Uuid,
    pub domains: Vec<String>,
    pub not_before_ms: i64,
    pub not_after_ms: i64,
    pub created_at_ms: i64,
    pub active: bool,
}

#[derive(Debug)]
pub struct CertificateController {
    store: Arc<SqliteStateStore>,
    vault: SecretVault,
}

impl CertificateController {
    #[must_use]
    pub fn new(store: Arc<SqliteStateStore>, vault: SecretVault) -> Self {
        Self { store, vault }
    }

    /// Encrypts and replaces a named external credential.
    ///
    /// # Errors
    ///
    /// Returns an error when encryption or storage fails.
    pub fn save_secret(&self, name: &str, secret: &[u8]) -> Result<()> {
        validate_secret_name(name)?;
        let context = format!("managed-secret:{name}");
        let sealed = self.vault.seal(&context, secret)?;
        self.store.save_managed_secret(name, &sealed, now_ms())
    }

    /// Loads and decrypts a named external credential.
    ///
    /// # Errors
    ///
    /// Returns an error when storage is invalid or authentication of ciphertext fails.
    pub fn load_secret(&self, name: &str) -> Result<Option<SecretBytes>> {
        validate_secret_name(name)?;
        self.store
            .load_managed_secret(name)?
            .map(|sealed| self.vault.open(&format!("managed-secret:{name}"), &sealed))
            .transpose()
    }

    /// Encrypts a certificate private key and makes this domain set the active durable version.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata is invalid, encryption fails, or storage cannot commit.
    pub fn replace(&self, mut material: CertificateMaterial) -> Result<CertificateSummary> {
        material.domains.sort_unstable();
        material.domains.dedup();
        if material.domains.is_empty()
            || material.certificate_chain_pem.is_empty()
            || material.private_key_pem.expose().is_empty()
            || material.not_after_ms <= material.not_before_ms
        {
            return Err(Error::InvalidState(
                "certificate material is incomplete or has an invalid lifetime".to_owned(),
            ));
        }
        let certificate_id = Uuid::new_v4();
        let created_at_ms = now_ms();
        let context = format!("certificate:{certificate_id}:private-key");
        let sealed_private_key = self
            .vault
            .seal(&context, material.private_key_pem.expose())?;
        let row = StoredCertificateRow {
            certificate_id,
            domains: material.domains.clone(),
            certificate_chain_pem: material.certificate_chain_pem,
            sealed_private_key,
            not_before_ms: material.not_before_ms,
            not_after_ms: material.not_after_ms,
            created_at_ms,
            active: true,
        };
        self.store.replace_managed_certificate(&row)?;
        Ok(row.summary())
    }

    /// Loads all active certificate versions and decrypts their private keys.
    ///
    /// # Errors
    ///
    /// Returns an error when storage is invalid or a private key fails authentication.
    pub fn load_active(&self) -> Result<Vec<ManagedCertificate>> {
        self.store
            .active_managed_certificates()?
            .into_iter()
            .map(|row| {
                let private_key_pem = self.vault.open(
                    &format!("certificate:{}:private-key", row.certificate_id),
                    &row.sealed_private_key,
                )?;
                Ok(ManagedCertificate {
                    certificate_id: row.certificate_id,
                    domains: row.domains,
                    certificate_chain_pem: row.certificate_chain_pem,
                    private_key_pem,
                    not_before_ms: row.not_before_ms,
                    not_after_ms: row.not_after_ms,
                    created_at_ms: row.created_at_ms,
                })
            })
            .collect()
    }

    /// Lists lifecycle metadata without decrypting or returning private keys.
    ///
    /// # Errors
    ///
    /// Returns an error when storage cannot be read or contains invalid metadata.
    pub fn list(&self) -> Result<Vec<CertificateSummary>> {
        self.store
            .managed_certificate_rows()?
            .into_iter()
            .map(|row| Ok(row.summary()))
            .collect()
    }

    /// Verifies that the configured master key can authenticate every encrypted database value.
    ///
    /// Plaintext is held only for the duration of this check and is never returned.
    ///
    /// # Errors
    ///
    /// Returns an error when a row is malformed or the master key cannot decrypt it.
    pub fn verify_protected_material(&self) -> Result<()> {
        for (name, sealed) in self.store.managed_secret_rows()? {
            self.vault
                .open(&format!("managed-secret:{name}"), &sealed)?;
        }
        for row in self.store.managed_certificate_rows()? {
            self.vault.open(
                &format!("certificate:{}:private-key", row.certificate_id),
                &row.sealed_private_key,
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SealedSecret {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct StoredCertificateRow {
    pub certificate_id: Uuid,
    pub domains: Vec<String>,
    pub certificate_chain_pem: Arc<[u8]>,
    pub sealed_private_key: SealedSecret,
    pub not_before_ms: i64,
    pub not_after_ms: i64,
    pub created_at_ms: i64,
    pub active: bool,
}

impl StoredCertificateRow {
    fn summary(&self) -> CertificateSummary {
        CertificateSummary {
            certificate_id: self.certificate_id,
            domains: self.domains.clone(),
            not_before_ms: self.not_before_ms,
            not_after_ms: self.not_after_ms,
            created_at_ms: self.created_at_ms,
            active: self.active,
        }
    }
}

fn validate_secret_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Error::InvalidState(
            "managed secret name must contain 1-80 safe characters".to_owned(),
        ));
    }
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[derive(Debug, Default)]
struct ChallengeState {
    next_generation: u64,
    responses: HashMap<(String, String), (u64, Arc<str>)>,
}

/// Active HTTP-01 responses shared by the ACME workflow and Pingora data plane.
#[derive(Clone, Debug, Default)]
pub struct Http01ChallengeRegistry {
    state: Arc<Mutex<ChallengeState>>,
}

impl Http01ChallengeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes one domain-bound challenge response until the returned guard is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when the domain, token, or key authorization is malformed or too large.
    pub fn publish(
        &self,
        domain: &str,
        token: &str,
        key_authorization: &str,
    ) -> Result<Http01ChallengeGuard> {
        let domain = normalize_domain(domain)?;
        validate_token(token)?;
        if key_authorization.is_empty()
            || key_authorization.len() > 2_048
            || key_authorization
                .bytes()
                .any(|byte| byte <= b' ' || byte == 0x7f)
        {
            return Err(Error::InvalidState(
                "HTTP-01 key authorization must be 1-2048 visible ASCII bytes".to_owned(),
            ));
        }

        let key = (domain, token.to_owned());
        let mut state = self.state.lock();
        state.next_generation = state.next_generation.saturating_add(1);
        let generation = state.next_generation;
        state
            .responses
            .insert(key.clone(), (generation, Arc::from(key_authorization)));
        drop(state);
        Ok(Http01ChallengeGuard {
            registry: self.clone(),
            key,
            generation,
        })
    }

    #[must_use]
    pub fn resolve(&self, host: &str, path: &str) -> Option<Arc<str>> {
        let token = path.strip_prefix("/.well-known/acme-challenge/")?;
        if validate_token(token).is_err() {
            return None;
        }
        let domain = normalize_domain(host).ok()?;
        self.state
            .lock()
            .responses
            .get(&(domain, token.to_owned()))
            .map(|(_, response)| Arc::clone(response))
    }

    fn remove(&self, key: &(String, String), generation: u64) {
        let mut state = self.state.lock();
        if state
            .responses
            .get(key)
            .is_some_and(|(current, _)| *current == generation)
        {
            state.responses.remove(key);
        }
    }
}

/// Lifetime guard for one published challenge. Dropping it removes only its own generation.
#[derive(Debug)]
pub struct Http01ChallengeGuard {
    registry: Http01ChallengeRegistry,
    key: (String, String),
    generation: u64,
}

impl Drop for Http01ChallengeGuard {
    fn drop(&mut self) {
        self.registry.remove(&self.key, self.generation);
    }
}

fn normalize_domain(domain: &str) -> Result<String> {
    let domain = domain
        .trim()
        .trim_end_matches('.')
        .split_once(':')
        .map_or(domain.trim().trim_end_matches('.'), |(name, _)| name)
        .to_ascii_lowercase();
    if domain.is_empty()
        || domain.len() > 253
        || domain.starts_with("*.")
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(Error::InvalidState(
            "HTTP-01 requires a valid non-wildcard DNS name".to_owned(),
        ));
    }
    Ok(domain)
}

fn validate_token(token: &str) -> Result<()> {
    if token.is_empty()
        || token.len() > 256
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(Error::InvalidState(
            "HTTP-01 token must be 1-256 base64url characters".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        CertificateController, CertificateMaterial, Http01ChallengeRegistry, SecretBytes,
        SecretVault,
    };
    use crate::SqliteStateStore;

    #[test]
    fn challenge_is_domain_bound_and_removed_with_its_guard() {
        let registry = Http01ChallengeRegistry::new();
        let guard = registry
            .publish("Example.TEST.", "token_42", "token_42.thumbprint")
            .unwrap();

        assert_eq!(
            registry
                .resolve("example.test:80", "/.well-known/acme-challenge/token_42")
                .as_deref(),
            Some("token_42.thumbprint")
        );
        assert!(
            registry
                .resolve("other.test", "/.well-known/acme-challenge/token_42")
                .is_none()
        );

        drop(guard);
        assert!(
            registry
                .resolve("example.test", "/.well-known/acme-challenge/token_42")
                .is_none()
        );
    }

    #[test]
    fn stale_guard_cannot_remove_a_republished_challenge() {
        let registry = Http01ChallengeRegistry::new();
        let old = registry
            .publish("example.test", "same-token", "old-value")
            .unwrap();
        let current = registry
            .publish("example.test", "same-token", "new-value")
            .unwrap();

        drop(old);
        assert_eq!(
            registry
                .resolve("example.test", "/.well-known/acme-challenge/same-token")
                .as_deref(),
            Some("new-value")
        );
        drop(current);
    }

    #[test]
    fn rejects_wildcards_and_path_shaped_tokens() {
        let registry = Http01ChallengeRegistry::new();
        assert!(
            registry
                .publish("*.example.test", "token", "value")
                .is_err()
        );
        assert!(
            registry
                .publish("example.test", "../token", "value")
                .is_err()
        );
    }

    #[test]
    fn encrypted_secrets_require_the_same_master_key() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStateStore::open(directory.path().join("state.db")).unwrap());
        let encoded_key = SecretVault::generate_base64();
        let controller = CertificateController::new(
            Arc::clone(&store),
            SecretVault::from_base64(&encoded_key).unwrap(),
        );
        controller
            .save_secret("acme-account", b"account-private-key")
            .unwrap();
        assert_eq!(
            controller
                .load_secret("acme-account")
                .unwrap()
                .unwrap()
                .expose(),
            b"account-private-key"
        );

        let wrong = CertificateController::new(
            store,
            SecretVault::from_base64(&SecretVault::generate_base64()).unwrap(),
        );
        assert!(wrong.load_secret("acme-account").is_err());
    }

    #[test]
    fn replacing_a_domain_set_preserves_history_and_only_loads_the_active_key() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStateStore::open(directory.path().join("state.db")).unwrap());
        let controller = CertificateController::new(
            store,
            SecretVault::from_base64(&SecretVault::generate_base64()).unwrap(),
        );
        let material = |key: &'static [u8], not_after_ms| CertificateMaterial {
            domains: vec!["www.example.test".to_owned(), "example.test".to_owned()],
            certificate_chain_pem: Arc::from(&b"public certificate"[..]),
            private_key_pem: SecretBytes::new(key),
            not_before_ms: 1_000,
            not_after_ms,
        };

        let old = controller.replace(material(b"old key", 10_000)).unwrap();
        let new = controller.replace(material(b"new key", 20_000)).unwrap();
        let summaries = controller.list().unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].certificate_id, new.certificate_id);
        assert!(summaries[0].active);
        assert_eq!(summaries[1].certificate_id, old.certificate_id);
        assert!(!summaries[1].active);

        let active = controller.load_active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].certificate_id, new.certificate_id);
        assert_eq!(active[0].private_key_pem.expose(), b"new key");
    }
}
