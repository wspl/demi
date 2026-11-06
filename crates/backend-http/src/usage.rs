//! On a shared instance, every account's usage totals, for an administrator
//! (`crates/core-store/src/usage.rs`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::settings::InstanceMode;
use demi_web_api_protocol::usage::{InstanceUsage, UserUsage};

use demi_core_api::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::services::Services;

pub(super) async fn instance(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<InstanceUsage>, ApiError> {
    if services.vault.mode() != InstanceMode::Shared {
        return Err(ApiError::forbidden(
            "The instance's usage is a shared instance's view",
        ));
    }
    if user.role == Role::User {
        return Err(ApiError::forbidden(
            "The instance's usage is for administrators",
        ));
    }
    let mut users = Vec::new();
    for listed in services.control.users().await? {
        let totals = services.stores.of(&listed.user.id).await?.usage_totals().await?;
        users.push(UserUsage {
            user_id: listed.user.id,
            email: listed.user.email,
            totals,
        });
    }
    Ok(Json(InstanceUsage { users }))
}
