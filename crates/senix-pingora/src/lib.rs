//! Pingora adapter for the Senix gateway runtime.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use bytes::Bytes;
use http::Method;
use pingora_core::protocols::tls::ALPN;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::{Error as PingoraError, HTTPStatus, Result as PingoraResult, server::Server};
use pingora_http::{RequestHeader, ResponseHeader};
use pingora_proxy::{ProxyHttp, Session};
use senix_core::{
    Error as CoreError, GatewayRuntime, Http01ChallengeRegistry, RequestLease, UpstreamAlpn,
};

mod tls;

pub use tls::{
    InstalledCertificate, PreparedTlsCertificate, TlsCertificateError, TlsCertificateRegistry,
};

#[derive(Debug, Default)]
pub struct RequestContext {
    lease: Option<RequestLease>,
    response_recorded: bool,
}

/// Monotonic process-local proxy counters suitable for Prometheus collection.
#[derive(Debug, Default)]
pub struct ProxyMetrics {
    requests: AtomicU64,
    responses_1xx: AtomicU64,
    responses_2xx: AtomicU64,
    responses_3xx: AtomicU64,
    responses_4xx: AtomicU64,
    responses_5xx: AtomicU64,
    errors: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProxyMetricsSnapshot {
    pub requests: u64,
    pub responses_1xx: u64,
    pub responses_2xx: u64,
    pub responses_3xx: u64,
    pub responses_4xx: u64,
    pub responses_5xx: u64,
    pub errors: u64,
}

impl ProxyMetrics {
    fn record_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    fn record_response(&self, status: u16) {
        let counter = match status / 100 {
            1 => &self.responses_1xx,
            2 => &self.responses_2xx,
            3 => &self.responses_3xx,
            4 => &self.responses_4xx,
            5 => &self.responses_5xx,
            _ => return,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn snapshot(&self) -> ProxyMetricsSnapshot {
        ProxyMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            responses_1xx: self.responses_1xx.load(Ordering::Relaxed),
            responses_2xx: self.responses_2xx.load(Ordering::Relaxed),
            responses_3xx: self.responses_3xx.load(Ordering::Relaxed),
            responses_4xx: self.responses_4xx.load(Ordering::Relaxed),
            responses_5xx: self.responses_5xx.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SenixProxy {
    runtime: Arc<GatewayRuntime>,
    http01_challenges: Http01ChallengeRegistry,
    metrics: Arc<ProxyMetrics>,
}

impl SenixProxy {
    #[must_use]
    pub fn new(runtime: Arc<GatewayRuntime>, http01_challenges: Http01ChallengeRegistry) -> Self {
        Self::with_metrics(runtime, http01_challenges, Arc::default())
    }

    #[must_use]
    pub fn with_metrics(
        runtime: Arc<GatewayRuntime>,
        http01_challenges: Http01ChallengeRegistry,
        metrics: Arc<ProxyMetrics>,
    ) -> Self {
        Self {
            runtime,
            http01_challenges,
            metrics,
        }
    }

    fn http01_response(&self, method: &Method, host: &str, path: &str) -> Option<Arc<str>> {
        if method != Method::GET && method != Method::HEAD {
            return None;
        }
        self.http01_challenges.resolve(host, path)
    }
}

#[async_trait]
impl ProxyHttp for SenixProxy {
    type CTX = RequestContext;

    fn new_ctx(&self) -> Self::CTX {
        RequestContext::default()
    }

    async fn request_filter(
        &self,
        session: &mut Session,
        context: &mut Self::CTX,
    ) -> PingoraResult<bool> {
        self.metrics.record_request();
        let request = session.req_header();
        let host = request_host(request);
        let Some(response) = self.http01_response(&request.method, host, request.uri.path()) else {
            return Ok(false);
        };

        let mut header = ResponseHeader::build(200, None)?;
        header.insert_header("content-type", "application/octet-stream")?;
        header.insert_header("cache-control", "no-store")?;
        header.insert_header("content-length", response.len().to_string())?;
        let head_only = request.method == Method::HEAD;
        session
            .write_response_header(Box::new(header), head_only)
            .await?;
        if !head_only {
            session
                .write_response_body(Some(Bytes::copy_from_slice(response.as_bytes())), true)
                .await?;
        }
        self.metrics.record_response(200);
        context.response_recorded = true;
        Ok(true)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        context: &mut Self::CTX,
    ) -> PingoraResult<Box<HttpPeer>> {
        context.lease.take();
        let header = session.req_header();
        let long_lived = is_long_lived_request(header);
        let host = request_host(header);
        let mut lease = self
            .runtime
            .acquire(host, header.uri.path())
            .map_err(|error| {
                let status = match &error {
                    CoreError::RouteNotFound { .. } => 404,
                    CoreError::NoAvailableBackend(_) => 503,
                    _ => 500,
                };
                PingoraError::explain(HTTPStatus(status), error.to_string())
            })?;
        if long_lived {
            lease.mark_long_lived();
        }
        let mut peer = if let Some(tls) = lease.upstream_tls() {
            let mut peer = HttpPeer::new(lease.address(), true, tls.server_name.clone());
            peer.options.verify_cert = tls.verify_certificate;
            peer.options.verify_hostname = tls.verify_certificate;
            peer.options.alpn = match tls.alpn {
                UpstreamAlpn::Http1 => ALPN::H1,
                UpstreamAlpn::Http2 => ALPN::H2,
                UpstreamAlpn::Http2OrHttp1 => ALPN::H2H1,
            };
            peer
        } else {
            HttpPeer::new(lease.address(), false, String::new())
        };
        peer.group_key = lease.generation();
        context.lease = Some(lease);
        Ok(Box::new(peer))
    }

    async fn response_filter(
        &self,
        _session: &mut Session,
        response: &mut ResponseHeader,
        context: &mut Self::CTX,
    ) -> PingoraResult<()> {
        if is_long_lived_response(response)
            && let Some(lease) = context.lease.as_mut()
        {
            lease.mark_long_lived();
        }
        if !context.response_recorded {
            self.metrics.record_response(response.status.as_u16());
            context.response_recorded = true;
        }
        Ok(())
    }

    async fn logging(
        &self,
        _session: &mut Session,
        error: Option<&PingoraError>,
        context: &mut Self::CTX,
    ) {
        if error.is_some() {
            self.metrics.record_error();
        }
        context.lease.take();
    }
}

/// Adds the HTTP listener and an optional TLS listener to one proxy service.
///
/// # Errors
///
/// Returns an error when Pingora cannot load the certificate or private key.
pub fn add_http_proxy(
    server: &mut Server,
    listen: &str,
    tls: Option<(&str, TlsCertificateRegistry)>,
    runtime: Arc<GatewayRuntime>,
    http01_challenges: Http01ChallengeRegistry,
    metrics: Arc<ProxyMetrics>,
) -> PingoraResult<()> {
    let mut proxy = pingora_proxy::http_proxy_service(
        &server.configuration,
        SenixProxy::with_metrics(runtime, http01_challenges, metrics),
    );
    proxy.add_tcp(listen);
    if let Some((tls_listen, certificates)) = tls {
        let mut settings =
            pingora_core::listeners::tls::TlsSettings::with_callbacks(Box::new(certificates))?;
        settings.enable_h2();
        proxy.add_tls_with_settings(tls_listen, None, settings);
    }
    server.add_service(proxy);
    Ok(())
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        host.strip_prefix('[')
            .and_then(|host| host.split_once(']'))
            .map_or(host, |(address, _)| address)
    } else {
        host.split_once(':').map_or(host, |(name, _)| name)
    }
}

fn request_host(header: &RequestHeader) -> &str {
    header.uri.host().unwrap_or_else(|| {
        header
            .headers
            .get("host")
            .and_then(|value| value.to_str().ok())
            .map(strip_port)
            .unwrap_or_default()
    })
}

fn is_long_lived_request(header: &RequestHeader) -> bool {
    is_content_type(&header.headers, "application/grpc")
        || (has_header_token(&header.headers, "connection", "upgrade")
            && has_header_token(&header.headers, "upgrade", "websocket"))
}

fn is_long_lived_response(header: &ResponseHeader) -> bool {
    is_content_type(&header.headers, "text/event-stream")
        || is_content_type(&header.headers, "application/grpc")
        || (header.status.as_u16() == 101
            && has_header_token(&header.headers, "upgrade", "websocket"))
}

fn is_content_type(headers: &http::HeaderMap, expected: &str) -> bool {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            let value = value.trim();
            value.eq_ignore_ascii_case(expected)
                || (value
                    .get(..expected.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(expected))
                    && value.as_bytes().get(expected.len()) == Some(&b'+'))
        })
}

fn has_header_token(headers: &http::HeaderMap, name: &str, expected: &str) -> bool {
    headers.get_all(name).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        })
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{SenixProxy, is_long_lived_request, is_long_lived_response, request_host};
    use http::Method;
    use pingora_http::{RequestHeader, ResponseHeader};
    use senix_core::{GatewayRuntime, Http01ChallengeRegistry};

    #[test]
    fn reads_authority_from_http2_request_uri() {
        let mut request = RequestHeader::build("GET", b"/resource", None).unwrap();
        request.uri = "https://example.test:8443/resource".parse().unwrap();

        assert_eq!(request_host(&request), "example.test");
    }

    #[test]
    fn reads_host_header_from_http1_request() {
        let mut request = RequestHeader::build("GET", b"/resource", None).unwrap();
        request.append_header("host", "example.test:8080").unwrap();

        assert_eq!(request_host(&request), "example.test");
    }

    #[test]
    fn reads_bracketed_ipv6_host_without_port_syntax() {
        let mut request = RequestHeader::build("GET", b"/resource", None).unwrap();
        request.append_header("host", "[::1]:8080").unwrap();

        assert_eq!(request_host(&request), "::1");
    }

    #[test]
    fn serves_only_the_active_domain_bound_http01_response() {
        let challenges = Http01ChallengeRegistry::new();
        let _guard = challenges
            .publish("example.test", "token_42", "token_42.thumbprint")
            .unwrap();
        let proxy = SenixProxy::new(Arc::new(GatewayRuntime::new()), challenges);
        let path = "/.well-known/acme-challenge/token_42";

        assert_eq!(
            proxy
                .http01_response(&Method::GET, "example.test:80", path)
                .as_deref(),
            Some("token_42.thumbprint")
        );
        assert!(
            proxy
                .http01_response(&Method::POST, "example.test", path)
                .is_none()
        );
        assert!(
            proxy
                .http01_response(&Method::GET, "other.test", path)
                .is_none()
        );
    }

    #[test]
    fn classifies_websocket_and_grpc_requests() {
        let mut websocket = RequestHeader::build("GET", b"/socket", None).unwrap();
        websocket
            .append_header("connection", "keep-alive, Upgrade")
            .unwrap();
        websocket.append_header("upgrade", "websocket").unwrap();
        assert!(is_long_lived_request(&websocket));

        let mut grpc = RequestHeader::build("POST", b"/service.Call", None).unwrap();
        grpc.append_header("content-type", "application/grpc+proto; charset=utf-8")
            .unwrap();
        assert!(is_long_lived_request(&grpc));
    }

    #[test]
    fn classifies_sse_and_upgrade_responses() {
        let mut sse = ResponseHeader::build(200, None).unwrap();
        sse.append_header("content-type", "text/event-stream; charset=utf-8")
            .unwrap();
        assert!(is_long_lived_response(&sse));

        let mut websocket = ResponseHeader::build(101, None).unwrap();
        websocket.append_header("upgrade", "WebSocket").unwrap();
        assert!(is_long_lived_response(&websocket));

        let mut ordinary = ResponseHeader::build(200, None).unwrap();
        ordinary
            .append_header("content-type", "application/json")
            .unwrap();
        assert!(!is_long_lived_response(&ordinary));
    }
}
