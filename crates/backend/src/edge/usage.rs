//! The usage ledger's totals (`usage-and-quota.md` § Usage ledger): each
//! user's own, and on a shared instance every account's for an
//! administrator.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_web_api::auth::Role;
use demi_web_api::settings::InstanceMode;
use demi_web_api::usage::{InstanceUsage, UsageTotals};

use super::error::ApiError;
use super::gate::AuthUser;
use crate::backend::Services;

pub(super) async fn totals(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<UsageTotals>, ApiError> {
    let totals = services.control.usage_totals(user.id).await?;
    Ok(Json(UsageTotals { totals }))
}

pub(super) async fn instance(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<InstanceUsage>, ApiError> {
    if services.mode != InstanceMode::Shared {
        return Err(ApiError::forbidden("The instance's usage is a shared instance's view"));
    }
    if user.role == Role::User {
        return Err(ApiError::forbidden("The instance's usage is for administrators"));
    }
    let users = services.control.instance_usage().await?;
    Ok(Json(InstanceUsage { users }))
}
