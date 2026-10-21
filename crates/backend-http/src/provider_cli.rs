//! A process provider's command-line tool
//! (`crates/backend-user-shard/src/conversation/claude_cli.rs`).
//!
//! The routes:
//!
//! - Read the vendor's newest version, without waking the Claude Code
//!   machine.
//! - Install again; the install's state reaches the pages with the entry's.
//!
//! A sign-in installs the tool it runs itself.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_providers::vault::entries::ProviderEntry;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::providers::{NewestVersion, ProviderCli};
use demi_web_api_protocol::query::Refresh;

use super::error::ApiError;
use super::gate::AuthUser;
use super::providers::scoped;
use super::query::QueryParams;
use demi_backend_user_shard::conversation::claude_cli::start_install;
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::shard::Shards;

/// The entry the path names, when its provider runs a process; an entry
/// whose provider runs none has no tool to read or install.
async fn process_entry(
    services: &Services,
    user: &UserDto,
    id: &str,
) -> Result<ProviderEntry, ApiError> {
    let entry = scoped(services, user, id).await?;
    if services.assembly.runs_a_process(&entry).await? {
        Ok(entry)
    } else {
        Err(ApiError::provider_not_found())
    }
}

/// The entry's tool: `refresh=true` reads the vendor's newest version at
/// once.
pub(super) async fn read(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(Refresh { refresh }): QueryParams<Refresh>,
) -> Result<Json<ProviderCli>, ApiError> {
    process_entry(&services, &user, &id).await?;
    let newest = match services.claude_releases.latest(refresh.0).await {
        Ok(release) => NewestVersion::Read {
            version: release.version,
        },
        Err(error) => NewestVersion::Unreadable {
            message: error.to_string(),
        },
    };
    Ok(Json(ProviderCli { newest }))
}

/// Starts the install on the Claude Code machine again.
pub(super) async fn install(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let entry = process_entry(&services, &user, &id).await?;
    start_install(&services, &shards, &user.id, &entry.id).await;
    Ok(StatusCode::ACCEPTED)
}
