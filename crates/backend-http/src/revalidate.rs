//! How a browser reuses what a `GET` answered: only after the server says it
//! has not changed.
//!
//! The one rule is that an answer is reused only once the server has
//! confirmed it is still the answer:
//!
//! - A JSON answer carries an `ETag` of its content and `Cache-Control:
//!   private, no-cache`. The browser asks again on every use, with
//!   `If-None-Match`, and an unchanged answer is a 304 with the headers the
//!   full answer would have had, such as the Host's number
//!   (`X-Demi-Read-At`), and no body. The page reads the answer it gets and
//!   knows nothing of tags.
//! - An answer that already names its version, such as a file's text or a
//!   raw file, keeps the `ETag` its route made, compared by the route, and
//!   any `Cache-Control` it set; one that set none gets `private, no-cache`.
//! - An answer that carries a secret or a session's own state is never kept:
//!   `no-store`, no tag. These are the routes of `is_sensitive`.
//! - Signing out answers `Clear-Site-Data: "cache"` (`clear_cache`), so the
//!   browser drops what it kept of the account's answers.
//!
//! Streams, sockets and downloads that are not JSON pass through as they are.

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::headers::{ETag, HeaderMapExt as _, IfNoneMatch};
use sha2::{Digest as _, Sha256};

/// Whether the answer of a `GET` of `path`, relative to `/api`, holds a secret
/// or is a session's own: the authentication routes, a credential's values, an
/// automation's URL secret, the direct channel's offer and a subscription
/// sign-in's code.
fn is_sensitive(path: &str) -> bool {
    path == "/auth"
        || path.starts_with("/auth/")
        || path == "/credentials"
        || path.starts_with("/providers/subscription-login/")
        || path.ends_with("/secret")
        || path.ends_with("/direct")
}

/// `response` that tells the browser to drop its cache of this origin, as
/// signing out does: what the account's reads left there is not the next
/// person's to find.
pub(super) fn clear_cache(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert("clear-site-data", HeaderValue::from_static("\"cache\""));
    response
}

/// The layer of the session API's routes: the rule of this module for every
/// `GET`.
pub(super) async fn reads(request: Request, next: Next) -> Response {
    if request.method() != Method::GET {
        return next.run(request).await;
    }
    let sensitive = is_sensitive(request.uri().path());
    let held = request.headers().typed_get::<IfNoneMatch>();
    let response = next.run(request).await;
    if sensitive {
        return no_store(response);
    }
    let named = response.headers().contains_key(ETAG) || response.status() == StatusCode::NOT_MODIFIED;
    let json = response.status() == StatusCode::OK
        && response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"));
    if named || !json {
        return if named { no_cache(response) } else { response };
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, usize::MAX).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let etag: ETag = format!("\"{:x}\"", Sha256::digest(&bytes))
        .parse()
        .expect("a quoted hex digest is an entity tag");
    let unchanged = held.is_some_and(|held| !held.precondition_passes(&etag));
    parts.headers.typed_insert(etag);
    if !parts.headers.contains_key(CACHE_CONTROL) {
        parts
            .headers
            .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    }
    if unchanged {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(CONTENT_TYPE);
        parts.headers.remove(CONTENT_LENGTH);
        return Response::from_parts(parts, Body::empty());
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// `response` with `Cache-Control: private, no-cache`, unless its route set one.
fn no_cache(mut response: Response) -> Response {
    if !response.headers().contains_key(CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    }
    response
}

/// `response` that no cache keeps, whatever its route set.
fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.remove(ETAG);
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response
}
