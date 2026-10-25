//! Settings › Updates's routes, for the master and administrators
//! of a server that offers updates (`crates/backend-user-shard/src/upgrades.rs`).
//! A server that offers none answers them 404.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::shard::Shards;
use demi_backend_user_shard::upgrades::RunRefused;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::server_upgrade::{RunUpgrade, ServerUpgrade, SetUpgradeChannel, WorkingAgents};

use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AdminUser;

/// The page of a server that offers no updates.
fn not_offered() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "This server offers no updates")
}

/// The updates now.
fn now(services: &Services) -> Result<Json<ServerUpgrade>, ApiError> {
    services.upgrades.snapshot().map(Json).ok_or_else(not_offered)
}

pub(super) async fn read(
    State(services): State<Arc<Services>>,
    AdminUser(_): AdminUser,
) -> Result<Json<ServerUpgrade>, ApiError> {
    now(&services)
}

pub(super) async fn set_channel(
    State(services): State<Arc<Services>>,
    AdminUser(_): AdminUser,
    JsonBody(request): JsonBody<SetUpgradeChannel>,
) -> Result<Json<ServerUpgrade>, ApiError> {
    if !services.upgrades.offered() {
        return Err(not_offered());
    }
    services.upgrades.set_channel(request.channel).await?;
    now(&services)
}

pub(super) async fn check(
    State(services): State<Arc<Services>>,
    AdminUser(_): AdminUser,
) -> Result<Json<ServerUpgrade>, ApiError> {
    if !services.upgrades.offered() {
        return Err(not_offered());
    }
    services.upgrades.check().await;
    now(&services)
}

pub(super) async fn run(
    State(services): State<Arc<Services>>,
    AdminUser(_): AdminUser,
    JsonBody(request): JsonBody<RunUpgrade>,
) -> Result<Json<ServerUpgrade>, ApiError> {
    services.upgrades.run(&request.version).map_err(|refused| match refused {
        RunRefused::NotOffered => not_offered(),
        RunRefused::Running => ApiError::new(StatusCode::CONFLICT, ErrorCode::UpgradeRunning, refused.to_string()),
        RunRefused::NotNewer(_) | RunRefused::NotAVersion(_) => ApiError::invalid_body(refused.to_string()),
        RunRefused::Unwritten(_) => ApiError::internal_message(refused.to_string()),
    })?;
    now(&services)
}

pub(super) async fn agents(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AdminUser(_): AdminUser,
) -> Result<Json<WorkingAgents>, ApiError> {
    if !services.upgrades.offered() {
        return Err(not_offered());
    }
    let census = shards.census().await;
    Ok(Json(WorkingAgents {
        count: u32::try_from(census.running_turns).unwrap_or(u32::MAX),
    }))
}
