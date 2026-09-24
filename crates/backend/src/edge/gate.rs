//! The session gate over every `/api` path except the public entrances
//! (`backend.md` § Authentication and ownership).

use std::sync::Arc;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::SET_COOKIE;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use demi_web_api::auth::{Role, UserDto};

use super::cookies::{self, SESSION_COOKIE};
use super::error::ApiError;
use crate::backend::Services;

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
