//! A subscription entry's accounts and logins (`web-api.md` § Subscription
//! accounts): the account list and its active account, removal, and logins
//! into a new or an existing entry, with the code a sign-in takes. Changing
//! accounts is the configuring user's, like every other change of an entry;
//! tokens and codes are never returned.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_providers::vault::accounts;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{CredentialId, LoginId};
use demi_backend_providers::vault::logins::CodeRefusal;
use demi_backend_user_shard::conversation::claude_cli::ShardMachine;
use demi_provider_common::Secret;
use demi_web_api_protocol::providers::{
    Accounts, ActivateAccount, ActiveAccount, CredentialKind, LoginAnswer, LoginCode,
    LoginStarted, PendingStatus, RemovedAccount, StartedLogin, SubscriptionLogin,
};

use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::providers::{configures, reserve, scoped};
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::shard::Shards;

/// The entry's accounts; a user who only infers sees none.
pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Accounts>, ApiError> {
    let entry = scoped(&services, &user, &id).await?;
    let disclose = services.vault.configures(&user);
    Ok(Json(
        accounts::list(&services.assembly, &entry, disclose).await?,
    ))
}

/// Selects the account the entry infers with.
pub(super) async fn activate(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(activate): JsonBody<ActivateAccount>,
) -> Result<Json<ActiveAccount>, ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let _held = reserve(&services, &entry)?;
    let active = accounts::activate(&services.assembly, &entry, activate.credential_id).await?;
    Ok(Json(ActiveAccount { active }))
}

/// Removes an account, the active one included, and answers the account the
/// entry now infers with.
pub(super) async fn remove(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, account)): Path<(String, String)>,
) -> Result<Json<RemovedAccount>, ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let _held = reserve(&services, &entry)?;
    let account = CredentialId::try_from(account).map_err(|_| ApiError::account_not_found())?;
    let active = accounts::remove(&services.assembly, &entry, account).await?;
    Ok(Json(RemovedAccount { active }))
}

fn started(id: LoginId) -> (StatusCode, Json<LoginStarted>) {
    let login = StartedLogin {
        id,
        status: PendingStatus::Pending,
    };
    (StatusCode::ACCEPTED, Json(LoginStarted { login }))
}

/// Starts a login into a new entry of the family the body names; a
/// sign-in that runs a process runs it on the caller's Cloud.
pub(super) async fn start_login(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    JsonBody(login): JsonBody<SubscriptionLogin>,
) -> Result<(StatusCode, Json<LoginStarted>), ApiError> {
    configures(&services, &user)?;
    let family = login.provider_type;
    let label = match login.label {
        Some(label) => label.into_string(),
        None => format!("{family} subscription"),
    };
    let owner = services.vault.owner_for(&user.id).await?;
    let machine = Arc::new(ShardMachine::new(shards, user.id.clone()));
    let id = services
        .logins
        .start(owner, user.id, family, label, None, machine)
        .await?;
    Ok(started(id))
}

/// Starts a login of another account into the entry, which the login holds
/// until it ends.
pub(super) async fn login_into(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<LoginStarted>), ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    if entry.kind() != CredentialKind::Subscription {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::AccountsUnsupported,
            "This provider does not use subscription accounts",
        ));
    }
    let family = entry.family.clone();
    let label = entry.label.clone();
    let owner = entry.owner.clone();
    let machine = Arc::new(ShardMachine::new(shards, user.id.clone()));
    let id = services
        .logins
        .start(owner, user.id, family, label, Some(entry), machine)
        .await?;
    Ok(started(id))
}

fn login_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::LoginNotFound,
        "No such login",
    )
}

/// Where the caller's login is.
pub(super) async fn login_state(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<LoginAnswer>, ApiError> {
    let id = LoginId::try_from(id).map_err(|_| login_not_found())?;
    let login = services
        .logins
        .state(&id, &user.id)
        .ok_or_else(login_not_found)?;
    Ok(Json(LoginAnswer { login }))
}

/// Hands the code the caller pasted to the caller's login, which waits for
/// one once it has shown its link.
pub(super) async fn submit_code(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<LoginCode>,
) -> Result<StatusCode, ApiError> {
    configures(&services, &user)?;
    let id = LoginId::try_from(id).map_err(|_| login_not_found())?;
    // The body allows one line only, which is what a secret is.
    let code = Secret::try_from(body.code.into_string()).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidBody,
            "A code is one line of text",
        )
    })?;
    match services.logins.submit_code(&id, &user.id, code) {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(CodeRefusal::NotFound) => Err(login_not_found()),
        Err(refusal @ CodeRefusal::NotWaiting) => Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::LoginNotWaiting,
            refusal.to_string(),
        )),
    }
}

/// Cancels the caller's login, which stops at once, even between polls.
pub(super) async fn cancel_login(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    configures(&services, &user)?;
    let id = LoginId::try_from(id).map_err(|_| login_not_found())?;
    if services.logins.cancel(&id, &user.id).await {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(login_not_found())
    }
}
