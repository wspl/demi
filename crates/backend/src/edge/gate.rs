//! The session gate over every `/api` path except the public entrances, and
//! the product's origin that every browser WebSocket route requires
//! (`backend.md` § Authentication and ownership).

use std::sync::Arc;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{HOST, ORIGIN, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use demi_web_api::auth::{Role, UserDto};
use demi_web_api::error::ErrorCode;
use url::Url;

use super::AppState;
use super::cookies::{self, SESSION_COOKIE};
use super::error::ApiError;
use crate::backend::Services;

/// An upgrade from a page of the product. Every browser WebSocket route
/// takes it first: the browser lets any page open a WebSocket to the
/// product, and sends the session cookie from every page of its site, an
/// expose's among them. Any other origin, or none, answers 403
/// `forbidden_origin`.
pub(super) struct ProductPage;

impl FromRequestParts<AppState> for ProductPage {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if !from_product(&parts.headers, state.site.public_url.as_ref()) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                ErrorCode::ForbiddenOrigin,
                "A WebSocket of the product opens only from a page of the product",
            ));
        }
        Ok(Self)
    }
}

/// Whether the request comes from a page of the product: its `Origin` is
/// the public URL's origin, or names the host the request was sent to.
fn from_product(headers: &HeaderMap, public_url: Option<&Url>) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|origin| origin.to_str().ok()) else {
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
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn a_page_of_the_product_is_on_the_public_origin_or_on_the_host_asked() {
        let headers = |origin: Option<&str>, host: &str| {
            let mut headers = HeaderMap::new();
            if let Some(origin) = origin {
                headers.insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
            }
            headers.insert(HOST, HeaderValue::from_str(host).unwrap());
            headers
        };
        let public: Url = "https://demi.example.com/".parse().unwrap();
        assert!(from_product(&headers(Some("https://demi.example.com"), "10.0.0.2:3271"), Some(&public)));
        assert!(from_product(&headers(Some("http://127.0.0.1:3271"), "127.0.0.1:3271"), None));
        assert!(from_product(&headers(Some("https://demi.example.com"), "demi.example.com"), None));
        for (origin, host) in [
            (Some("https://elsewhere.example"), "127.0.0.1:3271"),
            (Some("https://a1b2.expose.demi.example.com"), "demi.example.com"),
            (Some("http://127.0.0.1:9999"), "127.0.0.1:3271"),
            (Some("null"), "127.0.0.1:3271"),
            (None, "127.0.0.1:3271"),
        ] {
            assert!(!from_product(&headers(origin, host), Some(&public)), "{origin:?}");
        }
    }
}
