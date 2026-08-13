use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;

use crate::{Error, Result};

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
    use super::Http01ChallengeRegistry;

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
}
