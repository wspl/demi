//! The session cookie (`backend.md` § Authentication and ownership):
//! `demi_session`, `HttpOnly`, `SameSite=Lax` and `Path=/`, and `Secure` for a
//! request that came over HTTPS, directly or through a proxy.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::uri::Scheme;
use axum::http::{HeaderMap, Uri};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, Expiration, SameSite};
use cookie::time::OffsetDateTime;
use demi_core::Timestamp;

pub(super) const SESSION_COOKIE: &str = "demi_session";

/// The cookie that carries a session's token until the session expires.
pub(super) fn session(token: &str, expires_at: Timestamp, https: bool) -> Cookie<'static> {
    let expires = OffsetDateTime::from_unix_timestamp(expires_at.to_jiff().as_second())
        .expect("every jiff timestamp is in the cookie time range");
    base(token.to_owned(), https)
        .expires(Expiration::DateTime(expires))
        .build()
}

/// The jar with the session cookie removed, so the browser stops sending it.
pub(super) fn remove(jar: CookieJar, https: bool) -> CookieJar {
    jar.remove(base(String::new(), https).build())
}

fn base(value: String, https: bool) -> cookie::CookieBuilder<'static> {
    Cookie::build((SESSION_COOKIE, value))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(https)
}

/// Whether the request came over HTTPS: by its own scheme, or by the first
/// value of `X-Forwarded-Proto`.
pub(super) fn over_https(uri: &Uri, headers: &HeaderMap) -> bool {
    uri.scheme() == Some(&Scheme::HTTPS)
        || headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .is_some_and(|first| first.trim().eq_ignore_ascii_case("https"))
}

/// Whether the request came over HTTPS, as a handler's argument.
pub(super) struct Https(pub(super) bool);

impl<S: Send + Sync> FromRequestParts<S> for Https {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(over_https(&parts.uri, &parts.headers)))
    }
}
