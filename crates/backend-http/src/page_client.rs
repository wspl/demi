//! Where a page connection comes from, which its log lines and its lanes'
//! spans name.
//!
//! - The request ID of its upgrade.
//! - The page load that opened it.
//! - The edge's request id and the client's address that a proxy such as
//!   Cloudflare passes, else the connection's own peer.
//! - The browser's user agent.

use std::net::IpAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use axum::http::request::Parts;
use demi_shared_types::RequestId;

use super::error::ApiError;
use super::listener::Peer;
use super::request_id::{self, CF_RAY};

/// The client's address as Cloudflare saw it.
const CF_CONNECTING_IP: &str = "cf-connecting-ip";

/// The client of a page connection's upgrade request.
#[derive(Debug, Clone)]
pub(super) struct PageClient {
    pub(super) request: RequestId,
    pub(super) page: Option<String>,
    pub(super) ray: Option<String>,
    pub(super) client: Option<IpAddr>,
    pub(super) user_agent: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for PageClient {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let request = parts
            .extensions
            .get::<RequestId>()
            .cloned()
            .ok_or_else(|| ApiError::internal_message("the request reached a page connection without its ID"))?;
        let headers = &parts.headers;
        Ok(Self {
            request,
            page: request_id::page_of(headers, &parts.uri).map(str::to_owned),
            ray: header(headers, CF_RAY),
            client: client_address(parts),
            user_agent: header(headers, USER_AGENT.as_str()),
        })
    }
}

/// The address the request came from: the one the proxy in front passes,
/// else the connection's.
pub(crate) fn client_address(parts: &axum::http::request::Parts) -> Option<std::net::IpAddr> {
    // An address a proxy passes that does not read as one is not taken.
    let passed = header(&parts.headers, CF_CONNECTING_IP).and_then(|address| address.parse().ok());
    let peer = parts
        .extensions
        .get::<ConnectInfo<Peer>>()
        .and_then(|ConnectInfo(peer)| peer.address);
    passed.or(peer)
}

/// A header's value, when it is there and visible ASCII, which a log line
/// can carry as it is.
fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
