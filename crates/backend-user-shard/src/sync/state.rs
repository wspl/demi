//! The product state a channel sends first, and each part of it a channel
//! sends when it changed (`web-api.md` § Page synchronization). The user's
//! shard reads each part from the stores and its own state when the channel
//! sends it; the state is not one atomic read across databases, and need not
//! be, since a change made while it is read is marked and sent after it.

use demi_backend_database::StorageError;
use demi_backend_page_sync::Part;
use demi_plugin_interface::PluginError;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::providers::{ProviderReading, ProviderState};
use demi_web_api_protocol::state::{ProductState, SyncEvent};
use futures_util::future::join_all;

use crate::shard::Shard;

/// Why a part of the product state could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StateError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A plugin could not answer its page state.
    #[error("a plugin's page state could not be read: {0}")]
    Plugin(#[from] PluginError),
}

impl Shard {
    /// The product state for `user`, this shard's user as the session
    /// resolved them.
    pub(crate) async fn product_state(&self, user: UserDto) -> Result<ProductState, StateError> {
        let services = self.services();
        let preferences = services.control.preferences(self.user().clone()).await?;
        let providers = self.provider_states(&user).await?;
        let workspaces = services.control.workspaces(self.user().clone()).await?;
        let devices = self.device_list().await?;
        let mut conversations = self.conversation_summaries(false).await?;
        conversations.extend(self.conversation_summaries(true).await?);
        let cloud = self.cloud_shard().cloud_status().await?;
        let subagents = services
            .control
            .subagent_settings(self.user().clone())
            .await?;
        let plugins = self.plugins().entries().await?;
        let plugin_states = self.plugins().page_states().await?;
        Ok(ProductState {
            user,
            mode: services.mode,
            preferences,
            providers,
            workspaces: workspaces
                .into_iter()
                .map(|workspace| workspace.dto())
                .collect(),
            devices,
            public_url: services
                .public_url
                .get()
                .expect("the backend listens before it serves a request")
                .as_str()
                .to_owned(),
            conversations,
            cloud,
            subagents,
            plugins,
            plugin_states,
            web_build: services.web_build.clone(),
            run: services.run.clone(),
        })
    }

    /// The part as the message that carries it, read now for `user`, this
    /// shard's user; `None` for a conversation the user no longer has.
    pub(crate) async fn read_part(
        &self,
        part: &Part,
        user: &UserDto,
    ) -> Result<Option<SyncEvent>, StateError> {
        let control = &self.services().control;
        let event = match part {
            Part::Conversation(id) => {
                let record = control.conversation(id.clone()).await?;
                let Some(record) = record.filter(|record| record.owner == *self.user()) else {
                    return Ok(None);
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
                provider: entry.dto(),
                details,
            }
        }))
        .await;
        Ok(providers)
    }
}
