//! Passkeys (`web-api.md` § Account API): the public sign-in with one, and
//! the caller's own, which adding takes the password for. A change reaches
//! the caller's pages as the `passkeys` part.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use demi_backend_accounts::passkeys::{
    self, IssueRefusal, Refusal, RelyingParty, passkey_list,
};
use demi_backend_page_sync::Part;
use demi_backend_user_shard::services::Services;
use demi_web_api_protocol::auth::{Identity, UserDto};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::passkeys::{
    CreationChallenge, NewPasskey, PasskeyAnswer, PasskeyId, PasskeyList, PasskeyPassword,
    PasskeyPatch, PasskeySignIn, SignInChallenge,
};

use super::body::JsonBody;
use super::cookies::{self, Https};
use super::error::ApiError;
use super::gate::{self, AuthUser};

/// The relying party for this request, or 409 `passkeys_unavailable` when
/// the backend offers no passkeys.
fn relying_party(services: &Services, headers: &HeaderMap) -> Result<RelyingParty, ApiError> {
    let public = services
        .public_url
        .get()
        .expect("the backend listens before it serves a request")
        .url();
    let page = gate::product_origin(headers, Some(public));
    RelyingParty::new(public, page.as_ref()).ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::PasskeysUnavailable,
            "This server offers no passkeys: its address is not a host name",
        )
    })
}

fn too_many(IssueRefusal::TooMany: IssueRefusal) -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::TooManyAttempts,
        "Too many passkey sign-ins are under way. Try again in a minute.",
    )
}

fn refused(refusal: Refusal) -> ApiError {
    match refusal {
        Refusal::Expired => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::PasskeyExpired,
            "The passkey request expired. Try again.",
        ),
        Refusal::UnknownPasskey(id) => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::UnknownPasskey,
            "This passkey is no longer on any account. Sign in with your password.",
        )
        .with_credential_id(id),
        Refusal::Failed => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidCredentials,
            "The passkey could not be verified",
        ),
    }
}

/// `POST /auth/passkey/options`.
pub(super) async fn sign_in_options(
    State(services): State<Arc<Services>>,
    headers: HeaderMap,
) -> Result<Json<SignInChallenge>, ApiError> {
    let party = relying_party(&services, &headers)?;
    let challenge = passkeys::start_sign_in(&services.control, &party)
        .await?
        .map_err(too_many)?;
    Ok(Json(challenge))
}

/// `POST /auth/passkey`: signs in as `POST /auth/login` does. A refusal is
/// not counted toward the address lockout, since a signature cannot be
/// guessed.
pub(super) async fn sign_in(
    State(services): State<Arc<Services>>,
    headers: HeaderMap,
    jar: CookieJar,
    Https(https): Https,
    JsonBody(request): JsonBody<PasskeySignIn>,
) -> Result<Response, ApiError> {
    let party = relying_party(&services, &headers)?;
    let user = passkeys::finish_sign_in(
        &services.control,
        &party,
        request.challenge_id,
        &request.credential,
    )
    .await?
    .map_err(refused)?;
    let account = services
        .control
        .account(user.clone())
        .await?
        .ok_or_else(|| refused(Refusal::Failed))?;
    services.sync.mark(&user, Part::Passkeys);
    let session = services.sessions.open(user).await?;
    let jar = jar.add(cookies::session(&session.token, session.expires_at, https));
    Ok((jar, Json(Identity { user: account.user })).into_response())
}

/// `GET /auth/passkeys`.
pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<PasskeyList>, ApiError> {
    Ok(Json(passkey_list(&services.control, &user.id).await?))
}

/// `POST /auth/passkeys/options`: the password first, checked as a password
/// change checks the current one.
pub(super) async fn add_options(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    JsonBody(request): JsonBody<PasskeyPassword>,
) -> Result<Json<CreationChallenge>, ApiError> {
    let party = relying_party(&services, &headers)?;
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
    let challenge = passkeys::start_registration(&services.control, &party, &user)
        .await?
        .map_err(too_many)?;
    Ok(Json(challenge))
}

/// `POST /auth/passkeys`.
pub(super) async fn add(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    JsonBody(request): JsonBody<NewPasskey>,
) -> Result<(StatusCode, Json<PasskeyAnswer>), ApiError> {
    let party = relying_party(&services, &headers)?;
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let record = passkeys::finish_registration(
        &services.control,
        &party,
        &user.id,
        request.challenge_id,
        &request.credential,
        user_agent,
    )
    .await?
    .map_err(refused)?;
    services.sync.mark(&user.id, Part::Passkeys);
    Ok((
        StatusCode::CREATED,
        Json(PasskeyAnswer {
            passkey: record.dto(),
        }),
    ))
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::PasskeyNotFound,
        "This passkey is no longer on your account",
    )
}

/// The passkey a path names; any text names one, which only an account
/// that has it finds.
fn path_id(id: String) -> Result<PasskeyId, ApiError> {
    PasskeyId::try_from(id).map_err(|_| not_found())
}

/// `PATCH /auth/passkeys/:id`.
pub(super) async fn rename(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<PasskeyPatch>,
) -> Result<StatusCode, ApiError> {
    changed(
        &services,
        &user,
        services
            .control
            .rename_passkey(user.id.clone(), path_id(id)?, patch.name.into_string())
            .await?,
    )
}

/// `DELETE /auth/passkeys/:id`.
pub(super) async fn remove(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    changed(
        &services,
        &user,
        services
            .control
            .remove_passkey(user.id.clone(), path_id(id)?)
            .await?,
    )
}

/// 204 once a change of the user's passkey is made, marked for the user's
/// pages; 404 when the user has no such passkey.
fn changed(services: &Services, user: &UserDto, found: bool) -> Result<StatusCode, ApiError> {
    if !found {
        return Err(not_found());
    }
    services.sync.mark(&user.id, Part::Passkeys);
    Ok(StatusCode::NO_CONTENT)
}
