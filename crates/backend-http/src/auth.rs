//! Setup, sign-in and the caller's own account.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use demi_backend_accounts::email_change::{StartOutcome, StartRefusal};
use demi_backend_accounts::passkeys::relying_party_id;
use demi_backend_database::accounts::ChallengeOutcome;
use demi_backend_database::accounts::TokenHash;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::auth::{
    Credentials, EmailChangeConfirm, EmailChangeStart, EmailChangeStarted, Identity, NicknamePatch,
    PasswordChange, SetupRequest, SetupStatus,
};
use demi_web_api_protocol::error::ErrorCode;

use super::body::JsonBody;
use super::cookies::{self, Https, SESSION_COOKIE};
use super::error::ApiError;
use super::gate::AuthUser;
use super::sessions::user_agent;
use demi_backend_user_shard::services::Services;

pub(super) async fn setup_status(
    State(services): State<Arc<Services>>,
) -> Result<Json<SetupStatus>, ApiError> {
    let needed = !services.control.has_users().await?;
    let passkey_rp_id = services
        .public_url
        .get()
        .and_then(|url| relying_party_id(url.url()))
        .map(str::to_owned);
    Ok(Json(SetupStatus {
        needed,
        passkey_rp_id,
    }))
}

/// Creates the master account while the instance has none, and signs it in.
pub(super) async fn setup(
    State(services): State<Arc<Services>>,
    headers: HeaderMap,
    jar: CookieJar,
    Https(https): Https,
    JsonBody(request): JsonBody<SetupRequest>,
) -> Result<Response, ApiError> {
    let password_hash = services.hasher.hash(request.password).await?;
    let Some(user) = services
        .control
        .create_master(request.nickname.into_string(), request.email, password_hash)
        .await?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::AlreadySetUp,
            "This instance has its master account",
        ));
    };
    if let Some(account) = &services.master_account {
        services
            .vault
            .seed(user.id.clone(), account, services.clock.now())
            .await?;
    }
    let session = services
        .sessions
        .open(user.id.clone(), user_agent(&headers))
        .await?;
    let jar = jar.add(cookies::session(&session.token, session.expires_at, https));
    Ok((StatusCode::CREATED, jar, Json(Identity { user })).into_response())
}

pub(super) async fn login(
    State(services): State<Arc<Services>>,
    headers: HeaderMap,
    jar: CookieJar,
    Https(https): Https,
    JsonBody(credentials): JsonBody<Credentials>,
) -> Result<Response, ApiError> {
    let email = credentials.email;
    if let Some(wait) = services.limiter.locked_for(&email) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::TooManyAttempts,
            "Too many failed sign-ins. Try again in a minute.",
        )
        .with_retry_after(wait));
    }
    let account = services.control.account_by_email(email.clone()).await?;
    let stored = account
        .as_ref()
        .map(|account| account.password_hash.clone());
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
    if account.user.disabled {
        return Err(ApiError::account_disabled());
    }
    let session = services
        .sessions
        .open(account.user.id.clone(), user_agent(&headers))
        .await?;
    services.sync.mark(&account.user.id, Part::Sessions);
    // Its last activity, which the administrators' lists show.
    services.sync.mark_everyone(&Part::Users);
    let jar = jar.add(cookies::session(&session.token, session.expires_at, https));
    Ok((jar, Json(Identity { user: account.user })).into_response())
}

/// Ends the session the cookie names, with the synchronization channels
/// and conversation sockets that opened with it, and clears the cookie.
pub(super) async fn logout(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    jar: CookieJar,
    Https(https): Https,
) -> Result<Response, ApiError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        services.sessions.close(cookie.value()).await?;
        services
            .sync
            .end_session(&user.id, &TokenHash::of(cookie.value()));
        services.sync.mark(&user.id, Part::Sessions);
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
    services.sync.mark(&user.id, Part::User);
    Ok(Json(Identity { user }))
}

/// Changes the caller's password when the current one checks; an account
/// still to choose its own sends the next one alone, which is its choice.
pub(super) async fn change_password(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(change): JsonBody<PasswordChange>,
) -> Result<StatusCode, ApiError> {
    match (user.chooses_password, change.current) {
        (true, None) => {}
        (false, Some(current)) => {
            let stored = services
                .control
                .account(user.id.clone())
                .await?
                .map(|account| account.password_hash);
            if !services.hasher.verify(current, stored).await? {
                return Err(ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::InvalidCredentials,
                    "Current password is wrong",
                ));
            }
        }
        (true, Some(_)) => {
            return Err(ApiError::invalid_body("A password still to be chosen has no current one"));
        }
        (false, None) => return Err(ApiError::invalid_body("The current password is needed")),
    }
    let password_hash = services.hasher.hash(change.next).await?;
    services
        .control
        .set_chosen_password(user.id.clone(), password_hash)
        .await?;
    if user.chooses_password {
        // Every page of the account waiting on the choice goes on, and the
        // administrators' lists lose *Hasn't Chosen a Password*.
        services.sync.mark(&user.id, Part::User);
        services.sync.mark_everyone(&Part::Users);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn start_email_change(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(start): JsonBody<EmailChangeStart>,
) -> Result<(StatusCode, Json<EmailChangeStarted>), ApiError> {
    match services
        .email
        .start(user.id, start.email, start.password)
        .await?
    {
        StartOutcome::Issued(challenge) => {
            Ok((StatusCode::ACCEPTED, Json(EmailChangeStarted { challenge })))
        }
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
            "Verification email could not be delivered. Try again.",
        ),
    }
}

fn email_taken() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        ErrorCode::EmailTaken,
        "That email is already in use",
    )
}

pub(super) async fn confirm_email_change(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(confirm): JsonBody<EmailChangeConfirm>,
) -> Result<Json<Identity>, ApiError> {
    match services
        .email
        .confirm(user.id, confirm.id, &confirm.code)
        .await?
    {
        ChallengeOutcome::Changed(user) => {
            services.sync.mark(&user.id, Part::User);
            Ok(Json(Identity { user }))
        }
        ChallengeOutcome::InvalidCode => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidCode,
            "Invalid or expired verification code",
        )),
        ChallengeOutcome::EmailTaken => Err(email_taken()),
    }
}
