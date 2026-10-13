//! Account administration (`web-api.md` § Account API; `product.md` § User
//! system): administrators list every account, create accounts of a role
//! they outrank with a first password the server makes, make another first
//! password, change a role, disable and enable, sign an account out
//! everywhere and make a sign-in code for it. Every route acts only on a
//! role below the caller's: nobody acts on a peer or on the master, and no
//! account is deleted.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_accounts::passwords::generate_first_password;
use demi_backend_database::accounts::Account;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::sessions::{HandoffCode, ProductPath};
use demi_web_api_protocol::users::{
    CreateUser, CreatedUser, FirstPassword, NewFirstPassword, UserChange, Users,
};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::{AdminUser, CurrentSession};
use super::sessions::{make_handoff, require_verification};
use demi_backend_user_shard::services::Services;

pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AdminUser(_): AdminUser,
) -> Result<Json<Users>, ApiError> {
    Ok(Json(Users {
        users: services.control.users().await?,
    }))
}

/// Creates an account of a role the caller outranks, with a first password
/// the server makes, which its owner replaces at once; the session must have
/// been verified in the last five minutes.
pub(super) async fn create(
    State(services): State<Arc<Services>>,
    AdminUser(caller): AdminUser,
    session: CurrentSession,
    JsonBody(request): JsonBody<CreateUser>,
) -> Result<(StatusCode, Json<CreatedUser>), ApiError> {
    let role = Role::from(request.role);
    if !caller.role.outranks(role) {
        return Err(ApiError::forbidden("Only the master creates admins"));
    }
    require_verification(&services, &session)?;
    let password = generate_first_password();
    let password_hash = services.hasher.hash(password.clone()).await?;
    let user = services
        .control
        .create_user(request.email, password_hash, role)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::EmailTaken,
                "An account has that email",
            )
        })?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedUser {
            user,
            password: FirstPassword::new(password.expose().to_owned()),
        }),
    ))
}

/// The account `id` names, which the caller outranks: 404 `user_not_found`
/// for an id that names none and 403 `forbidden` for a role not below the
/// caller's, both before a body is checked.
async fn lower_account(
    services: &Services,
    caller: &UserDto,
    id: String,
) -> Result<Account, ApiError> {
    let not_found = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::UserNotFound,
            "No such user",
        )
    };
    let id = UserId::try_from(id).map_err(|_| not_found())?;
    let target = services.control.account(id).await?.ok_or_else(not_found)?;
    if !caller.role.outranks(target.user.role) {
        return Err(ApiError::forbidden("A role acts on lower roles only"));
    }
    Ok(target)
}

/// Ends the pages' channels and sockets of the sessions `tokens` name, and
/// tells the account's pages its sessions changed.
fn end_sessions(services: &Services, user: &UserId, tokens: &[demi_backend_database::accounts::TokenHash]) {
    for token in tokens {
        services.sync.end_session(user, token);
    }
    services.sync.mark(user, Part::Sessions);
}

/// Makes a new first password for an account the caller outranks, its
/// earlier password stopping, and answers it once. The account chooses its
/// own at its next request: its sessions stay, and their pages' channels end
/// and open again to be refused until it has.
pub(super) async fn new_first_password(
    State(services): State<Arc<Services>>,
    AdminUser(caller): AdminUser,
    session: CurrentSession,
    Path(id): Path<String>,
) -> Result<Json<NewFirstPassword>, ApiError> {
    let target = lower_account(&services, &caller, id).await?;
    require_verification(&services, &session)?;
    let password = generate_first_password();
    let password_hash = services.hasher.hash(password.clone()).await?;
    services
        .control
        .set_first_password(target.user.id.clone(), password_hash)
        .await?;
    let tokens = services.control.session_tokens(target.user.id.clone()).await?;
    end_sessions(&services, &target.user.id, &tokens);
    Ok(Json(NewFirstPassword {
        password: FirstPassword::new(password.expose().to_owned()),
    }))
}

/// Changes the role of an account the caller outranks to a role the caller
/// outranks too, or disables or enables it. Disabling ends its sessions and
/// its devices' runner connections at once.
pub(super) async fn change(
    State(state): State<AppState>,
    AdminUser(caller): AdminUser,
    Path(id): Path<String>,
    body: Result<JsonBody<UserChange>, ApiError>,
) -> Result<StatusCode, ApiError> {
    let services = &state.services;
    let target = lower_account(services, &caller, id).await?;
    let JsonBody(change) = body?;
    match (change.role, change.disabled) {
        (Some(role), None) => {
            let role = Role::from(role);
            if !caller.role.outranks(role) {
                return Err(ApiError::forbidden("A role gives lower roles only"));
            }
            services.control.set_role(target.user.id.clone(), role).await?;
            services.sync.mark(&target.user.id, Part::User);
        }
        (None, Some(disabled)) => {
            let ended = services
                .control
                .set_disabled(target.user.id.clone(), disabled)
                .await?;
            end_sessions(services, &target.user.id, &ended);
            if disabled {
                let user = target.user.id.clone();
                state
                    .shards
                    .of(&user)
                    .call(|shard, _| async move { shard.disconnect_runners("the account was disabled").await })
                    .await??;
            }
        }
        _ => return Err(ApiError::invalid_body("Change the role or whether the account is disabled")),
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Signs an account the caller outranks out of every browser at once, its
/// open pages included.
pub(super) async fn sign_out(
    State(services): State<Arc<Services>>,
    AdminUser(caller): AdminUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let target = lower_account(&services, &caller, id).await?;
    let ended = services
        .control
        .sign_out_everywhere(target.user.id.clone())
        .await?;
    end_sessions(&services, &target.user.id, &ended);
    Ok(StatusCode::NO_CONTENT)
}

/// A sign-in code for an account the caller outranks, which signs a phone in
/// to it as a handoff code does and names the caller on the session it makes.
pub(super) async fn sign_in_code(
    State(services): State<Arc<Services>>,
    AdminUser(caller): AdminUser,
    session: CurrentSession,
    Path(id): Path<String>,
) -> Result<Json<HandoffCode>, ApiError> {
    let target = lower_account(&services, &caller, id).await?;
    require_verification(&services, &session)?;
    let home = ProductPath::try_from("/".to_owned()).expect("the home is a path of the product");
    Ok(Json(
        make_handoff(&services, &target.user.id, Some(&caller.id), home).await?,
    ))
}
