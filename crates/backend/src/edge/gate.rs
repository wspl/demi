//! What stands in front of the browser's routes (`backend.md` § Authentication
//! and ownership): the check that a request that could act comes from a page
//! of the product, over every browser route, and the session gate over every
//! `/api` path except the public entrances.

use std::sync::Arc;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{HOST, ORIGIN, SET_COOKIE, UPGRADE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use demi_web_api::auth::{Role, UserDto};
use demi_web_api::error::ErrorCode;
use url::Url;

use super::Site;
use super::cookies::{self, SESSION_COOKIE};
use super::error::ApiError;
use crate::backend::Services;

/// Refuses a request that could act with the user's session and comes from
/// a page other than the product's, with 403 `forbidden_origin`, before any
/// route sees it. The browser sends the session cookie from every page of
/// the product's site, an expose's among them, and lets such a page send a
/// POST or open a WebSocket without asking the backend. A request could act
/// when its method is unsafe or it upgrades the connection, as a WebSocket
/// does. One without `Origin` passes: every browser sends `Origin` with such
/// a request, so it comes from a program that is not a browser, such as
/// curl, which could send any origin it liked.
pub(super) async fn product_pages(State(site): State<Arc<Site>>, request: Request, next: Next) -> Response {
    let acts = !request.method().is_safe() || request.headers().contains_key(UPGRADE);
    if acts && from_another_page(request.headers(), site.public_url.as_ref()) {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            ErrorCode::ForbiddenOrigin,
            "Only a page of the product acts with its session",
        )
        .into_response();
    }
    next.run(request).await
}

/// Whether the request's `Origin` names a page other than the product's. A
/// request without `Origin` names no page.
fn from_another_page(headers: &HeaderMap, public_url: Option<&Url>) -> bool {
    headers
        .get(ORIGIN)
        .is_some_and(|origin| !is_product(origin, headers, public_url))
}

/// Whether `origin` is the product's: the public URL's origin, or the host
/// and port the request was sent to, as when a development server passes
/// the page's requests on.
fn is_product(origin: &HeaderValue, headers: &HeaderMap, public_url: Option<&Url>) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if public_url.is_some_and(|url| url.origin().ascii_serialization() == origin) {
        return true;
    }
    let Ok(origin) = Url::parse(origin) else {
        return false;
    };
    let Some(host) = origin.host_str() else {
        return false;
    };
    let authority = match origin.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    headers.get(HOST).is_some_and(|sent_to| sent_to.as_bytes() == authority.as_bytes())
}

/// The signed-in caller, which the gate resolved from the session cookie.
#[derive(Debug, Clone)]
pub(super) struct AuthUser(pub(super) UserDto);

impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or_else(ApiError::unauthenticated)
    }
}

/// A signed-in administrator: the master or an admin. Any other caller
/// answers 403 `forbidden`.
#[derive(Debug, Clone)]
pub(super) struct AdminUser(pub(super) UserDto);

impl<S: Send + Sync> FromRequestParts<S> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        if !user.role.outranks(Role::User) {
            return Err(ApiError::forbidden("Account administration is for administrators"));
        }
        Ok(Self(user))
    }
}

/// Passes a request whose cookie names a live session, with its user, and
/// renews the cookie with the session. A missing or expired session answers
/// 401 `unauthenticated`, and a cookie that names no live session is
/// cleared.
pub(super) async fn session(
    State(services): State<Arc<Services>>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    let https = cookies::over_https(request.uri(), request.headers());
    let Some(token) = jar.get(SESSION_COOKIE).map(|cookie| cookie.value().to_owned()) else {
        return ApiError::unauthenticated().into_response();
    };
    let session = match services.sessions.resolve(&token).await {
        Ok(Some(session)) => session,
        Ok(None) => return (cookies::remove(jar, https), ApiError::unauthenticated()).into_response(),
        Err(error) => return ApiError::from(error).into_response(),
    };
    request.extensions_mut().insert(AuthUser(session.user));
    let response = next.run(request).await;
    // A handler that sets the session cookie itself, as logout removes it,
    // decides the cookie.
    if !session.renewed || sets_session_cookie(&response) {
        return response;
    }
    let renewed = jar.add(cookies::session(&token, session.expires_at, https));
    (renewed, response).into_response()
}

fn sets_session_cookie(response: &Response) -> bool {
    let prefix = format!("{SESSION_COOKIE}=");
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .any(|value| value.as_bytes().starts_with(prefix.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_of_the_product_is_on_the_public_origin_or_on_the_host_asked() {
        let headers = |origin: Option<HeaderValue>, host: &str| {
            let mut headers = HeaderMap::new();
            if let Some(origin) = origin {
                headers.insert(ORIGIN, origin);
            }
            headers.insert(HOST, HeaderValue::from_str(host).unwrap());
            headers
        };
        let origin = |origin: &str| Some(HeaderValue::from_str(origin).unwrap());
        let public: Url = "https://demi.example.com/".parse().unwrap();
        for (origin, host, public_url) in [
            (origin("https://demi.example.com"), "10.0.0.2:3271", Some(&public)),
            (origin("http://127.0.0.1:3271"), "127.0.0.1:3271", None),
            (origin("https://demi.example.com"), "demi.example.com", None),
        ] {
            assert!(!from_another_page(&headers(origin.clone(), host), public_url), "{origin:?}");
        }
        for (origin, host) in [
            (origin("https://elsewhere.example"), "127.0.0.1:3271"),
            (origin("https://a1b2.expose.demi.example.com"), "demi.example.com"),
            (origin("http://127.0.0.1:9999"), "127.0.0.1:3271"),
            (origin("null"), "127.0.0.1:3271"),
            // An origin that is there but unreadable is not a missing one.
            (Some(HeaderValue::from_bytes(b"https://demi.example.com\xff").unwrap()), "demi.example.com"),
        ] {
            assert!(from_another_page(&headers(origin.clone(), host), Some(&public)), "{origin:?}");
        }
    }
}
