//! Setup, sign-in and the caller's own account (`web-api.md` § Account API).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use demi_web_api::auth::{
    Credentials, EmailChangeConfirm, EmailChangeStart, EmailChangeStarted, Identity, NicknamePatch,
    PasswordChange, SetupRequest, SetupStatus,
};
use demi_web_api::error::ErrorCode;

use super::body::JsonBody;
use super::cookies::{self, Https, SESSION_COOKIE};
use super::error::ApiError;
use super::gate::AuthUser;
use crate::auth::email_change::{ChallengeOutcome, StartOutcome, StartRefusal};
use crate::backend::Services;

pub(super) async fn setup_status(State(services): State<Arc<Services>>) -> Result<Json<SetupStatus>, ApiError> {
    let needed = !services.control.has_users().await?;
    Ok(Json(SetupStatus { needed }))
}

/// Creates the master account while the instance has none, and signs it in.
pub(super) async fn setup(
    State(services): State<Arc<Services>>,
    jar: CookieJar,
    Https(https): Https,
    JsonBody(request): JsonBody<SetupRequest>,
) -> Result<Response, ApiError> {
    let password_hash = services.hasher.hash(request.password).await?;
    let Some(user) = services.control.create_master(request.email, password_hash).await? else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::AlreadySetUp,
            "This instance has its master account",
        ));
    };
    let session = services.sessions.open(user.id.clone()).await?;
    let jar = jar.add(cookies::session(&session.token, session.expires_at, https));
    Ok((StatusCode::CREATED, jar, Json(Identity { user })).into_response())
}

pub(super) async fn login(
    State(services): State<Arc<Services>>,
    jar: CookieJar,
    Https(https): Https,
    JsonBody(credentials): JsonBody<Credentials>,
) -> Result<Response, ApiError> {
    let email = credentials.email;
    if services.limiter.locked(&email) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::TooManyAttempts,
            "Too many failed logins; try again in a minute",
        ));
    }
    let account = services.control.account_by_email(email.clone()).await?;
    let stored = account.as_ref().map(|account| account.password_hash.clone());
    let verified = services.hasher.verify(credentials.password, stored).await?;
    let Some(account) = account.filter(|_| verified) else {
        services.limiter.failed(&email);
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidCredentials,
            "Wrong email or password",
        ));
    };
    services.limiter.succeeded(&email);
    let session = services.sessions.open(account.user.id.clone()).await?;
    let jar = jar.add(cookies::session(&session.token, session.expires_at, https));
    Ok((jar, Json(Identity { user: account.user })).into_response())
}

/// Ends the session the cookie names and clears the cookie.
pub(super) async fn logout(
    State(services): State<Arc<Services>>,
    jar: CookieJar,
    Https(https): Https,
) -> Result<Response, ApiError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        services.sessions.close(cookie.value()).await?;
    }
    Ok((cookies::remove(jar, https), StatusCode::NO_CONTENT).into_response())
}

pub(super) async fn me(AuthUser(user): AuthUser) -> Json<Identity> {
    Json(Identity { user })
}

pub(super) async fn set_nickname(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(patch): JsonBody<NicknamePatch>,
) -> Result<Json<Identity>, ApiError> {
    let user = services
        .control
        .set_nickname(user.id, patch.nickname.into_string())
        .await?
        .ok_or_else(ApiError::unauthenticated)?;
    Ok(Json(Identity { user }))
}

/// Changes the caller's password when the current one checks.
pub(super) async fn change_password(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(change): JsonBody<PasswordChange>,
) -> Result<StatusCode, ApiError> {
    let stored = services
        .control
        .account(user.id.clone())
        .await?
        .map(|account| account.password_hash);
    if !services.hasher.verify(change.current, stored).await? {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidCredentials,
            "Current password is wrong",
        ));
    }
    let password_hash = services.hasher.hash(change.next).await?;
    services.control.set_password(user.id, password_hash).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn start_email_change(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(start): JsonBody<EmailChangeStart>,
) -> Result<(StatusCode, Json<EmailChangeStarted>), ApiError> {
    match services.email.start(user.id, start.email, start.password).await? {
        StartOutcome::Issued(challenge) => Ok((StatusCode::ACCEPTED, Json(EmailChangeStarted { challenge }))),
        StartOutcome::Refused(refusal) => Err(refused(refusal)),
    }
}

fn refused(refusal: StartRefusal) -> ApiError {
    match refusal {
        StartRefusal::InvalidCredentials => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidCredentials,
            "Current password is wrong",
        ),
        StartRefusal::EmailTaken => email_taken(),
        StartRefusal::CoolingDown => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::TooManyAttempts,
            "Wait a minute before requesting another code",
        ),
        StartRefusal::MailUnavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::MailUnavailable,
            "Email delivery is not configured",
        ),
        StartRefusal::MailFailed => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::MailFailed,
            "Verification email could not be delivered; try again",
        ),
    }
}

fn email_taken() -> ApiError {
    ApiError::new(StatusCode::CONFLICT, ErrorCode::EmailTaken, "That email is already in use")
}

pub(super) async fn confirm_email_change(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(confirm): JsonBody<EmailChangeConfirm>,
) -> Result<Json<Identity>, ApiError> {
    match services.email.confirm(user.id, confirm.id, &confirm.code).await? {
        ChallengeOutcome::Changed(user) => Ok(Json(Identity { user })),
        ChallengeOutcome::InvalidCode => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidCode,
            "Invalid or expired verification code",
        )),
        ChallengeOutcome::EmailTaken => Err(email_taken()),
    }
}
