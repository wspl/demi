//! Where a page's socket comes from, which its log lines name
//! (`web-application.md` § Liveness and reconnection): the edge's request id
//! and the client's address that a proxy such as Cloudflare passes, else the
//! connection's own peer, and the browser's user agent.

use std::net::IpAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use demi_backend_user_shard::shard::page_socket::{PageSocketKind, PageSocketOrigin};
use demi_web_api_protocol::ids::UserId;

use super::listener::Peer;

/// The request id Cloudflare gives each request it passes on.
const CF_RAY: &str = "cf-ray";
/// The client's address as Cloudflare saw it.
const CF_CONNECTING_IP: &str = "cf-connecting-ip";

/// The client of a page socket's upgrade request.
pub(super) struct PageClient {
    ray: Option<String>,
    client: Option<IpAddr>,
    user_agent: Option<String>,
}

impl PageClient {
    /// The origin of a socket of `kind` that this client opened for `user`;
    /// the caller names what else it serves.
    pub(super) fn origin(self, kind: PageSocketKind, user: UserId) -> PageSocketOrigin {
        PageSocketOrigin {
            kind,
            user,
            conversation: None,
            device: None,
            stream: None,
            ray: self.ray,
            client: self.client,
            user_agent: self.user_agent,
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for PageClient {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;
        // An address a proxy passes that does not read as one is not taken.
        let passed = header(headers, CF_CONNECTING_IP).and_then(|address| address.parse().ok());
        let peer = parts
            .extensions
            .get::<ConnectInfo<Peer>>()
            .and_then(|ConnectInfo(peer)| peer.address);
        Ok(Self {
            ray: header(headers, CF_RAY),
            client: passed.or(peer),
            user_agent: header(headers, USER_AGENT.as_str()),
        })
    }
}

/// A header's value, when it is there and visible ASCII, which a log line
/// can carry as it is.
fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
