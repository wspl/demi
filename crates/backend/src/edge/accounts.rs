//! A subscription entry's accounts and device logins (`web-api.md` §
//! Subscription accounts): setup-token imports, the account list and its
//! active account, removal, and logins into a new or an existing entry.
//! Changing accounts is the configuring user's, like every other change of
//! an entry; tokens are never returned.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::{CredentialId, LoginId};
use demi_web_api::providers::{
    Accounts, ActivateAccount, ActiveAccount, AddToken, AddedAccount, CredentialKind, LoginAnswer, LoginStarted,
    PendingStatus, ProviderAnswer, SetupTokenImport, StartedLogin, SubscriptionLogin,
};

use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::provider_cli::install_for_account;
use super::providers::{configures, reserve, scoped};
use crate::backend::Services;
use crate::shard::Shards;
use crate::vault::accounts;

/// Creates the caller's Claude Code entry from a setup token, and starts
/// the install of its CLI on the caller's Cloud.
pub(super) async fn import_setup_token(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    JsonBody(import): JsonBody<SetupTokenImport>,
) -> Result<(StatusCode, Json<ProviderAnswer>), ApiError> {
    configures(&services, &user)?;
    let owner = services.vault.owner_for(&user.id).await?;
    let entry = accounts::import_setup_token(
        &services.assembly,
        owner,
        import.label.into_string(),
        import.token.into_string(),
    )
    .await?;
    install_for_account(&services, &shards, &user, &entry).await;
    Ok((StatusCode::CREATED, Json(ProviderAnswer { provider: entry.dto() })))
}

/// The entry's accounts; a user who only infers sees none.
pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Accounts>, ApiError> {
    let entry = scoped(&services, &user, &id).await?;
    let disclose = services.vault.configures(&user);
    Ok(Json(accounts::list(&services.assembly, &entry, disclose).await?))
}

/// Adds another account to the entry from a setup token, and starts the
/// install of the entry's CLI on the caller's Cloud when it runs one.
pub(super) async fn add_token(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(add): JsonBody<AddToken>,
) -> Result<(StatusCode, Json<AddedAccount>), ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let account = {
        let _held = reserve(&services, &entry)?;
        accounts::add_token(&services.assembly, &entry, add.token.into_string()).await?
    };
    install_for_account(&services, &shards, &user, &entry).await;
    Ok((StatusCode::CREATED, Json(AddedAccount { account })))
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

/// Removes an account other than the active one.
pub(super) async fn remove(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path((id, account)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let _held = reserve(&services, &entry)?;
    let account = CredentialId::try_from(account).map_err(|_| ApiError::account_not_found())?;
    accounts::remove(&services.assembly, &entry, account).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn started(id: LoginId) -> (StatusCode, Json<LoginStarted>) {
    let login = StartedLogin {
        id,
        status: PendingStatus::Pending,
    };
    (StatusCode::ACCEPTED, Json(LoginStarted { login }))
}

/// Starts a device login into a new entry of the family the body names.
pub(super) async fn start_login(
    State(services): State<Arc<Services>>,
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
    let id = services.logins.start(owner, user.id, family, label, None).await?;
    Ok(started(id))
}

/// Starts a device login of another account into the entry, which the login
/// holds until it ends.
pub(super) async fn login_into(
    State(services): State<Arc<Services>>,
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
    let id = services
        .logins
        .start(owner, user.id, family, label, Some(entry))
        .await?;
    Ok(started(id))
}

fn login_not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::LoginNotFound, "No such login")
}

/// Where the caller's login is.
pub(super) async fn login_state(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<LoginAnswer>, ApiError> {
    let id = LoginId::try_from(id).map_err(|_| login_not_found())?;
    let login = services.logins.state(&id, &user.id).ok_or_else(login_not_found)?;
    Ok(Json(LoginAnswer { login }))
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
