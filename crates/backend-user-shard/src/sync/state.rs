//! The product state `GET /state` answers, and each part of it a channel sends
//! when it changed.
//!
//! The user's shard reads each part from the stores and its own state when the
//! channel sends it. The state is not one atomic read across databases, and need
//! not be: its version is taken before it is read, so a change made while it
//! is read is one after the version, which the page follows the state from.

use demi_backend_providers::vault::entries::Vault;
use demi_backend_accounts::passkeys::{passkey_list, relying_party_id};
use demi_backend_database::StorageError;
use demi_backend_database::accounts::TokenHash;
use demi_backend_page_sync::Part;
use demi_backend_runners::install::install_commands;
use demi_conversation_lane_protocol::Version;
use demi_plugin_interface::PluginError;
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::instructions::InstructionsAnswer;
use demi_web_api_protocol::port_forwards::PortForwardList;
use demi_web_api_protocol::users::Users;
use demi_web_api_protocol::providers::{ProviderReading, ProviderState};
use demi_web_api_protocol::state::{ProductState, SyncEvent};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::time::Instant;
use futures_util::future::join_all;

use crate::provider_settings::CatalogError;
use crate::shard::Shard;

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

/// The milliseconds each part of the product state's read took.
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
    cloud: u64,
    runner_release: u64,
    plugins: u64,
    plugin_states: u64,
    manages: u64,
}

/// `work`'s output, and the milliseconds it took in `ms`.
async fn timed<T>(ms: &mut u64, work: impl Future<Output = T>) -> T {
    let started = Instant::now();
    let output = work.await;
    *ms = millis(started);
    output
}

fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The entity tag of `body`, the value a read answers with: the quoted hash
/// of its JSON, which is what the `ETag` of the read carries
/// (`crates/backend-http/src/revalidate.rs`) and what a push of the value
/// names.
pub fn etag_of<T: Serialize>(body: &T) -> String {
    // The messages' types serialize as JSON, which serde_json never refuses.
    etag_of_json(&serde_json::to_vec(body).expect("a body serializes to JSON"))
}

/// The entity tag of the JSON `json`.
pub fn etag_of_json(json: &[u8]) -> String {
    format!("\"{:x}\"", Sha256::digest(json))
}

impl Shard {
    /// The product state for `user`, this shard's user as the session
    /// resolved them.
    ///
    /// It writes how long each part took, which a page with no offline copy
    /// waits for before it shows anything.
    pub async fn product_state(
        &self,
        user: UserDto,
    ) -> Result<ProductState, StateError> {
        let services = self.services();
        let started = Instant::now();
        let mut ms = PartTimes::default();
        // The version before anything is read: a change made while the state
        // is read is one after it.
        let version = services.sync.version(self.user());
        // The logins before the providers: a login that completed after
        // this read is learned from the logins part, and its account is
        // then read below or with that part.
        let logins = services.logins.listed(self.user());
        let preferences = timed(&mut ms.preferences, services.control.preferences(self.user().clone())).await?;
        let providers = timed(&mut ms.providers, self.provider_states(&user)).await?;
        let models = timed(&mut ms.models, services.known_model_catalog(self.user())).await?;
        let workspaces = timed(&mut ms.workspaces, services.control.workspaces(self.user().clone())).await?;
        let projects = timed(&mut ms.projects, services.control.projects(self.user().clone())).await?;
        let devices = timed(&mut ms.devices, self.device_list()).await?;
        let mut conversations = timed(&mut ms.conversations, self.conversation_summaries(false)).await?;
        let active = conversations.len();
        conversations.extend(timed(&mut ms.archived, self.conversation_summaries(true)).await?);
        let cloud = timed(&mut ms.cloud, self.cloud_shard().cloud_status()).await?;
        let runner_release = match timed(&mut ms.runner_release, services.runner_releases.current()).await {
            Ok(release) => release.map(|release| release.release),
            Err(error) => {
                // The installers and the runners' updates report the
                // deployment's broken record; the pages show the runners'
                // releases as unknown meanwhile.
                tracing::warn!(event = "runner_release.read_failed", %error, "the current runner release cannot be read");
                None
            }
        };
        let plugins = timed(&mut ms.plugins, self.plugins().entries()).await.map_err(StorageError::from)?;
        let plugin_states = timed(&mut ms.plugin_states, self.plugins().page_states()).await?;
        let manages = timed(&mut ms.manages, services.vault.managed_kinds(self.user())).await?;
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
            cloud_ms = ms.cloud,
            runner_release_ms = ms.runner_release,
            plugins_ms = ms.plugins,
            plugins = plugins.len(),
            plugin_states_ms = ms.plugin_states,
            plugin_states = plugin_states.len(),
            manages_ms = ms.manages,
            "the product state was read"
        );
        let public_url = services
            .public_url
            .get()
            .expect("the backend listens before it serves a request")
            .url();
        let server_upgrade = if user.role == Role::User { None } else { services.upgrades.snapshot() };
        Ok(ProductState {
            user,
            mode: services.vault.mode(),
            shared_subscriptions: services.vault.shared_subscriptions(),
            manages,
            preferences,
            providers,
            models,
            workspaces: workspaces.into_iter().map(|workspace| workspace.dto()).collect(),
            projects: projects
                .into_iter()
                .map(|project| project.dto())
                .collect(),
            devices,
            public_url: public_url.as_str().to_owned(),
            install_commands: install_commands(public_url),
            stun_urls: services.stun.clone(),
            runner_release,
            conversations,
            cloud,
            logins,
            passkey_rp_id: relying_party_id(public_url).map(str::to_owned),
            plugins,
            plugin_states,
            web_build: services.web_build.clone(),
            mail: services.email.delivers(),
            server_upgrade,
            run: services.run.clone(),
            version: Version {
                epoch: version.epoch,
                revision: version.revision,
            },
        })
    }

    /// The part as the message that carries it, read now for `user`, this
    /// shard's user, and the page of the session `reader`; a conversation
    /// the user no longer has was deleted.
    pub(crate) async fn read_part(
        &self,
        part: &Part,
        user: &UserDto,
        reader: &TokenHash,
    ) -> Result<Option<SyncEvent>, StateError> {
        let control = &self.services().control;
        let event = match part {
            // No channel of a page's sidebar follows it.
            Part::Metadata(_) => return Ok(None),
            Part::Conversation(id) => {
                let record = control.conversation(id.clone()).await?;
                let Some(record) = record.filter(|record| record.owner == *self.user()) else {
                    return Ok(Some(SyncEvent::ConversationDeleted { id: id.clone() }));
                };
                SyncEvent::Conversation {
                    conversation: Box::new(self.conversation_summary(record).await?),
                }
            }
            Part::ConversationOrder => SyncEvent::ConversationOrder {
                ids: control.conversation_order(self.user().clone()).await?,
            },
            Part::Instance => SyncEvent::Instance {
                mode: self.services().vault.mode(),
                manages: self.services().vault.managed_kinds(self.user()).await?,
            },
            Part::Preferences => SyncEvent::Preferences {
                preferences: control.preferences(self.user().clone()).await?,
            },
            Part::User => {
                let Some(account) = control.account(self.user().clone()).await? else {
                    return Ok(None);
                };
                SyncEvent::User { user: account.user }
            }
            Part::Workspaces => SyncEvent::Workspaces {
                workspaces: control
                    .workspaces(self.user().clone())
                    .await?
                    .into_iter()
                    .map(|workspace| workspace.dto())
                    .collect(),
            },
            Part::Projects => SyncEvent::Projects {
                projects: control
                    .projects(self.user().clone())
                    .await?
                    .into_iter()
                    .map(|project| project.dto())
                    .collect(),
            },
            Part::Devices => SyncEvent::Devices {
                devices: self.device_list().await?,
            },
            Part::Subagents => {
                let subagents = control.subagent_settings(self.user().clone()).await?;
                SyncEvent::Subagents {
                    etag: etag_of(&subagents),
                    subagents,
                }
            }
            Part::Instructions => {
                let instructions = control.instructions(self.user().clone()).await?;
                SyncEvent::Instructions {
                    etag: etag_of(&InstructionsAnswer { instructions: instructions.clone() }),
                    instructions,
                }
            }
            // The credentials' and the sessions' routes send no tag.
            Part::Credentials => SyncEvent::Credentials {
                credentials: self.credential_list().await?,
                etag: String::new(),
            },
            Part::PortForwards => {
                let forwards = self.port_forward_list().await?;
                let list = PortForwardList { forwards };
                SyncEvent::PortForwards {
                    etag: etag_of(&list),
                    port_forwards: list.forwards,
                }
            }
            Part::Passkeys => {
                let list = passkey_list(control, self.user()).await?;
                SyncEvent::Passkeys {
                    etag: etag_of(&list),
                    passkeys: list.passkeys,
                    user_handle: list.user_handle,
                }
            }
            Part::Sessions => SyncEvent::Sessions {
                sessions: self.services().sessions.list(self.user(), reader).await?.sessions,
                etag: String::new(),
            },
            Part::Users => {
                // A user's channel never carries the accounts.
                if user.role == Role::User {
                    return Ok(None);
                }
                let list = Users {
                    users: control.users().await?,
                    cloud_storage_limits: demi_backend_cloud::isolation::storage_limits(control, &self.services().cloud).await?,
                };
                SyncEvent::Users {
                    etag: etag_of(&list),
                    users: list.users,
                    cloud_storage_limits: list.cloud_storage_limits,
                }
            }
            Part::CloudIsolation => {
                // Only the master chooses the isolation.
                if user.role != Role::Master {
                    return Ok(None);
                }
                SyncEvent::CloudIsolation {
                    isolation: demi_backend_cloud::isolation::setting(control, &self.services().cloud).await?,
                    etag: String::new(),
                }
            }
            Part::ServerUpgrade => {
                // A user's channel never carries the server's updates.
                if user.role == Role::User {
                    return Ok(None);
                }
                let Some(upgrade) = self.services().upgrades.snapshot() else {
                    return Ok(None);
                };
                SyncEvent::ServerUpgrade { upgrade }
            }
            Part::Update => SyncEvent::Update {
                state: self.services().upgrades.update_state(),
            },
            Part::Plugins => SyncEvent::Plugins {
                plugins: self.plugins().entries().await.map_err(StorageError::from)?,
            },
            Part::Plugin(plugin) => {
                let Some(state) = self.plugins().page_state(plugin).await? else {
                    return Ok(None);
                };
                SyncEvent::Plugin {
                    plugin: plugin.clone(),
                    state,
                }
            }
            Part::Providers => SyncEvent::Providers {
                providers: self.provider_states(user).await?,
            },
            Part::Models => SyncEvent::Models {
                models: self.services().known_model_catalog(self.user()).await?,
            },
            Part::Logins => SyncEvent::Logins {
                logins: self.services().logins.listed(self.user()),
            },
            Part::Cloud => SyncEvent::Cloud {
                cloud: self.cloud_shard().cloud_status().await?,
            },
        };
        Ok(Some(event))
    }

    /// The entries `user` infers with, each with what its provider says now,
    /// which a user who only infers sees without accounts, plan or usage.
    /// One entry that cannot be read leaves the others intact.
    async fn provider_states(&self, user: &UserDto) -> Result<Vec<ProviderState>, StorageError> {
        let services = self.services();
        let entries = services.vault.entries_for(&user.id).await?;
        let providers = join_all(entries.iter().map(|entry| async move {
            let disclose = Vault::manages(&user.id, entry);
            let started = Instant::now();
            let details = match services.provider_registry.details(entry, disclose).await {
                Ok(details) => ProviderReading::Read(Box::new(details)),
                Err(error) => ProviderReading::Failed {
                    message: error.to_string(),
                },
            };
            // The entries are read at once: the slowest one is what the
            // state's read waits for.
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
                cli_install: services.cli_installs.state(&user.id, &entry.id),
            }
        }))
        .await;
        Ok(providers)
    }
}
