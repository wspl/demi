//! Account administration (`web-api.md` § Account API; `product.md` § User
//! system): administrators list every account, create accounts of a role
//! they outrank, and reset the password of an account they outrank. Nobody
//! acts on a peer or on the master, and no account is deleted.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::UserId;
use demi_web_api::users::{CreateUser, CreatedUser, PasswordReset, Users};

use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AdminUser;
use crate::backend::Services;

pub(super) async fn list(State(services): State<Arc<Services>>, AdminUser(_): AdminUser) -> Result<Json<Users>, ApiError> {
    Ok(Json(Users {
        users: services.control.users().await?,
    }))
}

pub(super) async fn create(
    State(services): State<Arc<Services>>,
    AdminUser(caller): AdminUser,
    JsonBody(request): JsonBody<CreateUser>,
) -> Result<(StatusCode, Json<CreatedUser>), ApiError> {
    let role = Role::from(request.role);
    if !caller.role.outranks(role) {
        return Err(ApiError::forbidden("Only the master creates admins"));
    }
    let password_hash = services.hasher.hash(request.password).await?;
    let user = services
        .control
        .create_user(request.email, password_hash, role)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, ErrorCode::EmailTaken, "An account has that email"))?;
    Ok((StatusCode::CREATED, Json(CreatedUser { user })))
}

/// Resets the password of an account the caller outranks. Who the account
/// is and whether the caller may reset it are answered before the body.
pub(super) async fn reset_password(
    State(services): State<Arc<Services>>,
    AdminUser(caller): AdminUser,
    Path(id): Path<String>,
    body: Result<JsonBody<PasswordReset>, ApiError>,
) -> Result<StatusCode, ApiError> {
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, ErrorCode::UserNotFound, "No such user");
    let id = UserId::try_from(id).map_err(|_| not_found())?;
    let target = services.control.account(id).await?.ok_or_else(not_found)?;
    if !caller.role.outranks(target.user.role) {
        return Err(ApiError::forbidden("A role acts on lower roles only"));
    }
    let JsonBody(reset) = body?;
    let password_hash = services.hasher.hash(reset.password).await?;
    services.control.set_password(target.user.id, password_hash).await?;
    Ok(StatusCode::NO_CONTENT)
}
