//! Pingora adapter for the Senix gateway runtime.

use std::sync::Arc;

use async_trait::async_trait;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::{Error as PingoraError, HTTPStatus, Result as PingoraResult, server::Server};
use pingora_proxy::{ProxyHttp, Session};
use senix_core::{Error as CoreError, GatewayRuntime, RequestLease};

#[derive(Debug, Default)]
pub struct RequestContext {
    lease: Option<RequestLease>,
}

#[derive(Clone, Debug)]
pub struct SenixProxy {
    runtime: Arc<GatewayRuntime>,
}

impl SenixProxy {
    #[must_use]
    pub fn new(runtime: Arc<GatewayRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl ProxyHttp for SenixProxy {
    type CTX = RequestContext;

    fn new_ctx(&self) -> Self::CTX {
        RequestContext::default()
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        context: &mut Self::CTX,
    ) -> PingoraResult<Box<HttpPeer>> {
        context.lease.take();
        let header = session.req_header();
        let host = header
            .headers
            .get("host")
            .and_then(|value| value.to_str().ok())
            .map(strip_port)
            .unwrap_or_default();
        let lease = self
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
        let mut peer = HttpPeer::new(lease.address(), false, String::new());
        peer.group_key = lease.generation();
        context.lease = Some(lease);
        Ok(Box::new(peer))
    }

    async fn logging(
        &self,
        _session: &mut Session,
        _error: Option<&PingoraError>,
        context: &mut Self::CTX,
    ) {
        context.lease.take();
    }
}

pub fn add_http_proxy(server: &mut Server, listen: &str, runtime: Arc<GatewayRuntime>) {
    let mut proxy =
        pingora_proxy::http_proxy_service(&server.configuration, SenixProxy::new(runtime));
    proxy.add_tcp(listen);
    server.add_service(proxy);
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        host.split_once(']').map_or(host, |(address, _)| address)
    } else {
        host.split_once(':').map_or(host, |(name, _)| name)
    }
}
