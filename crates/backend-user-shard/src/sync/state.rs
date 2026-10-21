//! The product state a channel sends first, and each part of it a channel sends
//! when it changed.
//!
//! The user's shard reads each part from the stores and its own state when the
//! channel sends it. The state is not one atomic read across databases, and need
//! not be, since a change made while it is read is marked and sent after it.

use demi_backend_accounts::passkeys::{passkey_list, relying_party_id};
use demi_backend_database::StorageError;
use demi_backend_database::accounts::TokenHash;
use demi_backend_page_sync::Part;
use demi_backend_runners::install::install_commands;
use demi_plugin_interface::PluginError;
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::providers::{ProviderReading, ProviderState};
use demi_web_api_protocol::state::{ProductState, SyncEvent};
use futures_util::future::join_all;

use crate::provider_settings::CatalogError;
use crate::shard::Shard;

/// Why a part of the product state could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StateError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A plugin could not answer its page state.
    #[error("a plugin's page state could not be read: {0}")]
    Plugin(#[from] PluginError),
    /// The model catalog could not be read.
    #[error("the model catalog could not be read: {0}")]
    Catalog(#[from] CatalogError),
}

impl Shard {
    /// The product state for `user`, this shard's user as the session
    /// resolved them, read for the page of the session `reader`.
    pub(crate) async fn product_state(
        &self,
        user: UserDto,
        reader: &TokenHash,
    ) -> Result<ProductState, StateError> {
        let services = self.services();
        // The logins before the providers: a login that completed after
        // this read is learned from the logins part, and its account is
        // then read below or with that part.
        let logins = services.logins.listed(self.user());
        let preferences = services.control.preferences(self.user().clone()).await?;
        let providers = self.provider_states(&user).await?;
        let models = services.known_model_catalog(self.user()).await?;
        let workspaces = services.control.workspaces(self.user().clone()).await?;
        let devices = self.device_list().await?;
        let mut conversations = self.conversation_summaries(false).await?;
        conversations.extend(self.conversation_summaries(true).await?);
        let cloud = self.cloud_shard().cloud_status().await?;
        let runner_release = match services.runner_releases.current().await {
            Ok(release) => release.map(|release| release.release),
            Err(error) => {
                // The installers and the runners' updates report the
                // deployment's broken record; the pages show the runners'
                // releases as unknown meanwhile.
                tracing::warn!("the current runner release cannot be read: {error}");
                None
            }
        };
        let subagents = services
            .control
            .subagent_settings(self.user().clone())
            .await?;
        let instructions = services.control.instructions(self.user().clone()).await?;
        let credentials = self.credential_list().await?;
        let port_forwards = self.port_forward_list().await?;
        let exposed_ports = self.exposed_port_list().await?;
        let automations = self.automation_list().await?;
        let automations_unread = services.control.unread_automation_runs(self.user().clone()).await?;
        let passkeys = passkey_list(&services.control, self.user()).await?;
        let sessions = services.sessions.list(self.user(), reader).await?.sessions;
        let users = match user.role {
            Role::User => None,
            Role::Master | Role::Admin => Some(services.control.users().await?),
        };
        let plugins = self.plugins().entries().await?;
        let plugin_states = self.plugins().page_states().await?;
        let public_url = services
            .public_url
            .get()
            .expect("the backend listens before it serves a request")
            .url();
        Ok(ProductState {
            user,
            mode: services.mode,
            preferences,
            providers,
            models,
            workspaces: workspaces
                .into_iter()
                .map(|workspace| workspace.dto())
                .collect(),
            devices,
            public_url: public_url.as_str().to_owned(),
            install_commands: install_commands(public_url),
            stun_urls: services.stun.clone(),
            runner_release,
            conversations,
            cloud,
            logins,
            subagents,
            instructions,
            credentials,
            port_forwards,
            exposed_ports,
            automations,
            automations_unread,
            passkeys,
            passkey_rp_id: relying_party_id(public_url).map(str::to_owned),
            sessions,
            users,
            plugins,
            plugin_states,
            web_build: services.web_build.clone(),
            mail: services.email.delivers(),
            run: services.run.clone(),
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
            Part::Devices => SyncEvent::Devices {
                devices: self.device_list().await?,
            },
            Part::Subagents => SyncEvent::Subagents {
                subagents: control.subagent_settings(self.user().clone()).await?,
            },
            Part::Instructions => SyncEvent::Instructions {
                instructions: control.instructions(self.user().clone()).await?,
            },
            Part::Credentials => SyncEvent::Credentials {
                credentials: self.credential_list().await?,
            },
            Part::PortForwards => SyncEvent::PortForwards {
                port_forwards: self.port_forward_list().await?,
            },
            Part::ExposedPorts => SyncEvent::ExposedPorts {
                exposed_ports: self.exposed_port_list().await?,
            },
            Part::Automations => SyncEvent::Automations {
                automations: self.automation_list().await?,
                automations_unread: self.services().control.unread_automation_runs(self.user().clone()).await?,
            },
            Part::Passkeys => {
                let list = passkey_list(control, self.user()).await?;
                SyncEvent::Passkeys {
                    passkeys: list.passkeys,
                    user_handle: list.user_handle,
                }
            }
            Part::Sessions => SyncEvent::Sessions {
                sessions: self.services().sessions.list(self.user(), reader).await?.sessions,
            },
            Part::Users => {
                // A user's channel never carries the accounts.
                if user.role == Role::User {
                    return Ok(None);
                }
                SyncEvent::Users {
                    users: control.users().await?,
                }
            }
            Part::Plugins => SyncEvent::Plugins {
                plugins: self.plugins().entries().await?,
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
        let owner = services.vault.owner_for(&user.id).await?;
        let entries = services.vault.entries(owner).await?;
        let disclose = services.vault.configures(user);
        let providers = join_all(entries.iter().map(|entry| async move {
            let details = match services.assembly.details(entry, disclose).await {
                Ok(details) => ProviderReading::Read(Box::new(details)),
                Err(error) => ProviderReading::Failed {
                    message: error.to_string(),
                },
            };
            ProviderState {
                provider: services.assembly.provider_dto(entry),
                details,
                cli_install: services.cli_installs.state(&user.id, &entry.id),
            }
        }))
        .await;
        #[cfg(feature = "testing")]
        services.syncs.pass(super::SyncStep::Providers).await;
        Ok(providers)
    }
}
