//! The core's part of the product state: `GET /api/state`, which a page
//! reads once its synchronization channel is open, and each part of it the
//! channel sends when it changed.
//!
//! The lists only a settings pane shows are not in it; each has its own route.
//! The answer names the version of the state it was read at, which the page
//! follows on the channel. The server answers its own `GET /api/state`, this
//! one with its own parts beside it.
//!
//! The core reads each part from its store and its own state when the channel
//! sends it. The state is not one atomic read across databases, and need not
//! be: its version is taken before it is read, so a change made while it is
//! read is one after the version, which the page follows the state from.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use demi_conversation_lane_protocol::Version;
use demi_core::Core;
use demi_core::provider_settings::CatalogError;
use demi_core_providers::vault::entries::Vault;
use demi_core_store::StorageError;
use demi_core_sync::{Part, etag_of};
use demi_plugin_interface::PluginError;
use demi_web_api_protocol::instructions::InstructionsAnswer;
use demi_web_api_protocol::port_forwards::PortForwardList;
use demi_web_api_protocol::providers::{ProviderReading, ProviderState};
use demi_web_api_protocol::state::{CoreState, SyncEvent};
use futures_util::future::join_all;

use crate::api::Api;
use crate::caller::Principal;
use crate::error::ApiError;

/// Why a part of the product state could not be read.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A plugin could not answer its page state.
    #[error("a plugin's page state could not be read: {0}")]
    Plugin(#[from] PluginError),
    /// The model catalog could not be read.
    #[error("the model catalog could not be read: {0}")]
    Catalog(#[from] CatalogError),
}

/// `GET /state`: the core's part of the product state.
pub async fn read(State(state): State<Api>, Principal(user): Principal) -> Result<Json<CoreState>, ApiError> {
    let core_state = state
        .shards
        .of(&user)
        .call(move |core, _| async move { core_state(&core).await.map_err(|error| ApiError::internal(&error)) })
        .await??;
    Ok(Json(core_state))
}

/// The milliseconds each part of the core's state's read took.
#[derive(Default)]
struct PartTimes {
    preferences: u64,
    providers: u64,
    models: u64,
    workspaces: u64,
    projects: u64,
    devices: u64,
    conversations: u64,
    archived: u64,
    plugins: u64,
    plugin_states: u64,
    manages: u64,
}

/// `work`'s output, and the milliseconds it took in `ms`.
pub async fn timed<T>(ms: &mut u64, work: impl Future<Output = T>) -> T {
    let started = Instant::now();
    let output = work.await;
    *ms = millis(started);
    output
}

pub fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The core's part of the product state, read on `core`.
///
/// It logs how long each part took, which a page with no offline copy waits
/// for before it shows anything.
pub async fn core_state(core: &Core) -> Result<CoreState, StateError> {
    let services = core.services();
    let started = Instant::now();
    let mut ms = PartTimes::default();
    // The version before anything is read: a change made while the state is
    // read is one after it.
    let version = services.sync.version(core.user());
    // The logins before the providers: a login that completed after this
    // read is learned from the logins part, and its account is then read
    // below or with that part.
    let logins = services.logins.listed(core.user());
    let preferences = timed(&mut ms.preferences, core.store().preferences()).await?;
    let providers = timed(&mut ms.providers, provider_states(core)).await?;
    let models = timed(&mut ms.models, services.known_model_catalog(core.user())).await?;
    let workspaces = timed(&mut ms.workspaces, core.store().workspaces()).await?;
    let projects = timed(&mut ms.projects, core.store().projects()).await?;
    let devices = timed(&mut ms.devices, core.device_list()).await?;
    let mut conversations = timed(&mut ms.conversations, core.conversation_summaries(false)).await?;
    let active = conversations.len();
    conversations.extend(timed(&mut ms.archived, core.conversation_summaries(true)).await?);
    let plugins = timed(&mut ms.plugins, core.plugins().entries()).await.map_err(StorageError::from)?;
    let plugin_states = timed(&mut ms.plugin_states, core.plugins().page_states()).await?;
    let manages = timed(&mut ms.manages, services.vault.managed_kinds(core.user())).await?;
    tracing::info!(
        event = "state.read",
        ms = millis(started),
        preferences_ms = ms.preferences,
        providers_ms = ms.providers,
        providers = providers.len(),
        models_ms = ms.models,
        workspaces_ms = ms.workspaces,
        projects_ms = ms.projects,
        devices_ms = ms.devices,
        conversations_ms = ms.conversations,
        conversations = active,
        archived_ms = ms.archived,
        archived = conversations.len() - active,
        plugins_ms = ms.plugins,
        plugins = plugins.len(),
        plugin_states_ms = ms.plugin_states,
        plugin_states = plugin_states.len(),
        manages_ms = ms.manages,
        "the core's state was read"
    );
    Ok(CoreState {
        mode: services.vault.mode(),
        shared_subscriptions: services.vault.shared_subscriptions(),
        manages,
        preferences,
        providers,
        models,
        workspaces: workspaces.into_iter().map(|workspace| workspace.dto()).collect(),
        projects: projects.into_iter().map(|project| project.dto()).collect(),
        devices,
        stun_urls: services.stun.clone(),
        conversations,
        logins,
        plugins,
        plugin_states,
        run: services.run.clone(),
        version: Version {
            epoch: version.epoch,
            revision: version.revision,
        },
    })
}

/// Whether `part` is one of the core's own, which [`read_core_part`] reads;
/// the others are the mount's.
pub fn is_core_part(part: &Part) -> bool {
    match part {
        Part::Conversation(_)
        | Part::Metadata(_)
        | Part::ConversationOrder
        | Part::Instance
        | Part::Preferences
        | Part::Workspaces
        | Part::Projects
        | Part::Devices
        | Part::Subagents
        | Part::Instructions
        | Part::Credentials
        | Part::PortForwards
        | Part::Plugins
        | Part::Plugin(_)
        | Part::Providers
        | Part::Models
        | Part::Logins => true,
        Part::User
        | Part::Passkeys
        | Part::Sessions
        | Part::Users
        | Part::CloudIsolation
        | Part::ServerUpgrade
        | Part::Update
        | Part::Cloud => false,
    }
}

/// The core's part `part` as the message that carries it, read now on
/// `core`; none when the page is sent nothing of it. A conversation the
/// core no longer has was deleted.
pub async fn read_core_part(core: &Core, part: &Part) -> Result<Option<SyncEvent>, StateError> {
    let store = core.store();
    let services = core.services();
    let event = match part {
        // No channel of a page's sidebar follows it.
        Part::Metadata(_) => return Ok(None),
        Part::Conversation(id) => {
            let record = store.conversation(id.clone()).await?;
            let Some(record) = record else {
                return Ok(Some(SyncEvent::ConversationDeleted { id: id.clone() }));
            };
            SyncEvent::Conversation {
                conversation: Box::new(core.conversation_summary(record).await?),
            }
        }
        Part::ConversationOrder => SyncEvent::ConversationOrder {
            ids: store.conversation_order().await?,
        },
        Part::Instance => SyncEvent::Instance {
            mode: services.vault.mode(),
            manages: services.vault.managed_kinds(core.user()).await?,
        },
        Part::Preferences => SyncEvent::Preferences {
            preferences: store.preferences().await?,
        },
        Part::Workspaces => SyncEvent::Workspaces {
            workspaces: store.workspaces().await?.into_iter().map(|workspace| workspace.dto()).collect(),
        },
        Part::Projects => SyncEvent::Projects {
            projects: store.projects().await?.into_iter().map(|project| project.dto()).collect(),
        },
        Part::Devices => SyncEvent::Devices {
            devices: core.device_list().await?,
        },
        Part::Subagents => {
            let subagents = store.subagent_settings().await?;
            SyncEvent::Subagents {
                etag: etag_of(&subagents),
                subagents,
            }
        }
        Part::Instructions => {
            let instructions = store.instructions().await?;
            SyncEvent::Instructions {
                etag: etag_of(&InstructionsAnswer { instructions: instructions.clone() }),
                instructions,
            }
        }
        // The credentials' route sends no tag.
        Part::Credentials => SyncEvent::Credentials {
            credentials: core.credential_list().await?,
            etag: String::new(),
        },
        Part::PortForwards => {
            let forwards = core.port_forward_list().await?;
            let list = PortForwardList { forwards };
            SyncEvent::PortForwards {
                etag: etag_of(&list),
                port_forwards: list.forwards,
            }
        }
        Part::Plugins => SyncEvent::Plugins {
            plugins: core.plugins().entries().await.map_err(StorageError::from)?,
        },
        Part::Plugin(plugin) => {
            let Some(state) = core.plugins().page_state(plugin).await? else {
                return Ok(None);
            };
            SyncEvent::Plugin {
                plugin: plugin.clone(),
                state,
            }
        }
        Part::Providers => SyncEvent::Providers {
            providers: provider_states(core).await?,
        },
        Part::Models => SyncEvent::Models {
            models: services.known_model_catalog(core.user()).await?,
        },
        Part::Logins => SyncEvent::Logins {
            logins: services.logins.listed(core.user()),
        },
        Part::User
        | Part::Passkeys
        | Part::Sessions
        | Part::Users
        | Part::CloudIsolation
        | Part::ServerUpgrade
        | Part::Update
        | Part::Cloud => return Ok(None),
    };
    Ok(Some(event))
}

/// The entries the core's principal infers with, each with what its
/// provider says now, which a principal who only infers sees without
/// accounts, plan or usage. One entry that cannot be read leaves the others
/// intact.
async fn provider_states(core: &Core) -> Result<Vec<ProviderState>, StorageError> {
    let services = core.services();
    let user = core.user();
    let entries = services.vault.entries_for(user).await?;
    let providers = join_all(entries.iter().map(|entry| async move {
        let disclose = Vault::manages(user, entry);
        let started = Instant::now();
        let details = match services.provider_registry.details(entry, disclose).await {
            Ok(details) => ProviderReading::Read(Box::new(details)),
            Err(error) => ProviderReading::Failed {
                message: error.to_string(),
            },
        };
        // The entries are read at once: the slowest one is what the state's
        // read waits for.
        tracing::info!(
            event = "provider_details.read",
            provider = %entry.id,
            family = %entry.family,
            ms = millis(started),
            "an entry's details were read"
        );
        ProviderState {
            provider: services.provider_registry.provider_dto(entry),
            details,
            cli_install: services.cli_installs.state(user, &entry.id),
        }
    }))
    .await;
    Ok(providers)
}
