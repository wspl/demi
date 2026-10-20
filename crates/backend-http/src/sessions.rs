//! The account's signed-in browsers, verifying the session, and handing the
//! account to a phone by a code. A session that starts or ends reaches the
//! account's pages as the `sessions` part.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use demi_backend_accounts::passkeys;
use demi_backend_accounts::sessions::{PhoneSession, SignOut, Spend};
use demi_backend_page_sync::Part;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::passkeys::{PasskeySignIn, SignInChallenge};
use demi_web_api_protocol::sessions::{
    HandoffAnswer, HandoffCode, HandoffRequest, HandoffSpend, ProductPath, SessionId, SessionList,
    VerifyPassword,
};

use super::body::JsonBody;
use super::cookies::{self, Https, SESSION_COOKIE};
use super::error::ApiError;
use super::gate::{AuthUser, CurrentSession};
use super::passkeys::{refused, relying_party, too_many};

/// The User-Agent a request names its browser with, which names the
/// session it signs in and a passkey it adds.
pub(super) fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers.get(USER_AGENT).and_then(|value| value.to_str().ok())
}

/// `POST /auth/verify`: the password, checked as a password change checks
/// the current one.
pub(super) async fn verify(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    JsonBody(request): JsonBody<VerifyPassword>,
) -> Result<StatusCode, ApiError> {
    let stored = services
        .control
        .account(user.id.clone())
        .await?
        .map(|account| account.password_hash);
    if !services.hasher.verify(request.password, stored).await? {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidCredentials,
            "Wrong password",
        ));
    }
    services.sessions.verify(&session.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /auth/verify/passkey/options`: a challenge as a passkey sign-in's.
pub(super) async fn verify_passkey_options(
    State(services): State<Arc<Services>>,
    headers: HeaderMap,
) -> Result<Json<SignInChallenge>, ApiError> {
    let party = relying_party(&services, &headers)?;
    let challenge = passkeys::start_sign_in(&services.control, &party)
        .await?
        .map_err(too_many)?;
    Ok(Json(challenge))
}

/// `POST /auth/verify/passkey`: the browser's answer, checked as a passkey
/// sign-in's, with a passkey of the caller's own account.
pub(super) async fn verify_passkey(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    headers: HeaderMap,
    JsonBody(request): JsonBody<PasskeySignIn>,
) -> Result<StatusCode, ApiError> {
    let party = relying_party(&services, &headers)?;
    passkeys::finish_sign_in(
        &services.control,
        &party,
        request.challenge_id,
        &request.credential,
        Some(&user.id),
    )
    .await?
    .map_err(refused)?;
    services.sync.mark(&user.id, Part::Passkeys);
    services.sessions.verify(&session.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The session's verification must hold: it signed in, or was verified, in
/// the last five minutes. What needs the user's confirmation (a handoff
/// code, adding a passkey, revealing a credential's values) is gated by
/// this, and by nothing else.
pub(super) fn require_verification(services: &Services, session: &CurrentSession) -> Result<(), ApiError> {
    if services.sessions.verification_holds(session.verified_at) {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        ErrorCode::VerificationRequired,
        "Confirm it is you first",
    ))
}

/// `POST /auth/handoff/code`: a code that opens `to` on a phone, for a
/// session verified in the last five minutes; it ends the account's
/// earlier code.
pub(super) async fn make_code(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    JsonBody(request): JsonBody<HandoffRequest>,
) -> Result<Json<HandoffCode>, ApiError> {
    require_verification(&services, &session)?;
    Ok(Json(make_handoff(&services, &user.id, None, request.to).await?))
}

/// A code for `user` that opens `to`, which an administrator `issued_by`
/// makes as a sign-in code; the caller checked the session's verification.
pub(super) async fn make_handoff(
    services: &Services,
    user: &UserId,
    issued_by: Option<&UserId>,
    to: ProductPath,
) -> Result<HandoffCode, ApiError> {
    let public = services
        .public_url
        .get()
        .expect("the backend listens before it serves a request")
        .url();
    Ok(services
        .sessions
        .make_handoff(user, issued_by, to, public)
        .await?)
}

/// `POST /auth/handoff`: spends a code and opens its account in the
/// browser: in the session its cookie holds when that is of the account,
/// setting no cookie, else signed in as `POST /auth/login` does, ending the
/// other account's session the cookie held, its pages with it.
pub(super) async fn spend_code(
    State(services): State<Arc<Services>>,
    headers: HeaderMap,
    jar: CookieJar,
    Https(https): Https,
    JsonBody(request): JsonBody<HandoffSpend>,
) -> Result<Response, ApiError> {
    let current = jar.get(SESSION_COOKIE).map(|cookie| cookie.value());
    let handed = match services
        .sessions
        .spend_handoff(&request.code, current, user_agent(&headers))
        .await?
    {
        Spend::Opened(handed) => handed,
        Spend::Disabled => return Err(ApiError::account_disabled()),
        Spend::Unknown => {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::HandoffExpired,
                "This code has expired. Show a new one on your other device.",
            ));
        }
    };
    services.sync.mark(&handed.user, Part::Sessions);
    services.sync.mark_everyone(&Part::Users);
    let answer = Json(HandoffAnswer { to: handed.to });
    match handed.session {
        PhoneSession::Kept => Ok(answer.into_response()),
        PhoneSession::Opened { session, ended } => {
            if let Some(ended) = ended {
                services.sync.end_session(&ended.user, &ended.token);
                services.sync.mark(&ended.user, Part::Sessions);
            }
            let jar = jar.add(cookies::session(&session.token, session.expires_at, https));
            Ok((jar, answer).into_response())
        }
    }
}

/// `GET /auth/sessions`.
pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
) -> Result<Json<SessionList>, ApiError> {
    Ok(Json(services.sessions.list(&user.id, &session.token).await?))
}

/// `DELETE /auth/sessions/:id`: signs another session of the account out,
/// closing its pages' channels and conversation sockets.
pub(super) async fn sign_out(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let not_found = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::SessionNotFound,
            "This browser is no longer signed in",
        )
    };
    // Any text names a session, which only an account that has it finds.
    let id = SessionId::try_from(id).map_err(|_| not_found())?;
    match services.sessions.sign_out(&user.id, id, &session.id).await? {
        SignOut::Done(token) => {
            services.sync.end_session(&user.id, &token);
            services.sync.mark(&user.id, Part::Sessions);
            Ok(StatusCode::NO_CONTENT)
        }
        SignOut::Current => Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::CurrentSession,
            "Use Sign Out to end this browser's session",
        )),
        SignOut::NotFound => Err(not_found()),
    }
}
