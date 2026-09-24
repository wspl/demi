//! A process provider's command-line tool (`web-api.md` § Model
//! configuration and provider inspection, `claude-code.md` § What the user
//! sees): the vendor's newest version, the last install on the caller's
//! Cloud and the versions that Cloud has, read without waking it; the
//! install again; and the install that adding an account starts.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_web_api::auth::UserDto;
use demi_web_api::providers::{CliInstallAnswer, NewestVersion, ProviderCli};
use demi_web_api::query::Refresh;

use super::error::ApiError;
use super::gate::AuthUser;
use super::providers::scoped;
use super::query::QueryParams;
use crate::backend::Services;
use crate::llm::claude_cli::start_install;
use crate::shard::Shards;
use crate::vault::entries::ProviderEntry;

/// The entry the path names, when its provider runs a process; an entry
/// whose provider runs none has no tool to read or install.
async fn process_entry(services: &Services, user: &UserDto, id: &str) -> Result<ProviderEntry, ApiError> {
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
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(Refresh { refresh }): QueryParams<Refresh>,
) -> Result<Json<ProviderCli>, ApiError> {
    let entry = process_entry(&services, &user, &id).await?;
    let shard = shards.of(&user.id);
    let entry_id = entry.id.clone();
    let machines = shard.call(move |shard, cancel| async move { shard.cli_machines(&entry_id, &cancel).await });
    let (newest, machines) = tokio::join!(services.claude_releases.latest(refresh.0), machines);
    let newest = match newest {
        Ok(release) => NewestVersion::Read {
            version: release.version,
        },
        Err(error) => NewestVersion::Unreadable {
            message: error.to_string(),
        },
    };
    Ok(Json(ProviderCli {
        newest,
        install: services.cli_installs.state(&user.id, &entry.id),
        machines: machines??,
    }))
}

/// Starts the install on the caller's Cloud again.
pub(super) async fn install(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<CliInstallAnswer>), ApiError> {
    let entry = process_entry(&services, &user, &id).await?;
    let install = start_install(&services, &shards, &user.id, &entry.id).await;
    Ok((StatusCode::ACCEPTED, Json(CliInstallAnswer { install })))
}

/// Starts the install of the tool on the acting user's Cloud after an
/// account was added to `entry`, when its provider runs a process. Its
/// failure is the install's state, never the failure of adding the account.
pub(super) async fn install_for_account(services: &Services, shards: &Shards, user: &UserDto, entry: &ProviderEntry) {
    match services.assembly.runs_a_process(entry).await {
        Ok(true) => {
            start_install(services, shards, &user.id, &entry.id).await;
        }
        Ok(false) => {}
        // The account stands; the settings page offers the install again.
        Err(error) => tracing::warn!(provider = %entry.id, "the CLI install was not started: {error}"),
    }
}
