//! How the user's conversations use the Cloud (`sessions-and-targets.md`
//! § How a conversation uses a device): as the `target` their files and
//! commands are on, as the `provider` their model's process runs on, or
//! `attached` as an extra Host. Every role keeps the Cloud awake while the
//! conversation works; an idle stop and a reset hold the conversations that
//! cannot work without the Cloud, the first two roles, and leave an
//! attached one alone: it runs on its own target. The roles are read when
//! the lifecycle needs them; the `provider` role follows from the providers
//! the conversation's nodes infer with, its root's and its live subagents',
//! which the placement runs on the Cloud whenever one needs a process
//! (`claude-code.md` § Where it runs).

use std::collections::HashMap;
use std::time::Duration;

use demi_backend_database::StorageError;
use demi_backend_database::managed::CloudUseRecord;
use demi_web_api_protocol::ids::{ConversationId, ProviderId};

use crate::machine::CloudError;
use crate::{CloudShard, ConversationHold};

/// How a conversation uses the Cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Target,
    /// Its model's process runs on the Cloud; `attached` when the Cloud is
    /// one of its Hosts too.
    Provider {
        attached: bool,
    },
    Attached,
}

impl Role {
    /// Whether the conversation cannot work without the Cloud, so a stop or
    /// a reset holds it.
    fn needs_cloud(self) -> bool {
        matches!(self, Self::Target | Self::Provider { .. })
    }
}

impl dyn CloudShard {
    /// Each of the user's conversations that uses the Cloud, with its
    /// strongest role.
    pub(crate) async fn cloud_uses(&self) -> Result<Vec<(ConversationId, Role)>, StorageError> {
        let control = self.control();
        let cloud = control
            .managed_device(self.user().clone())
            .await?
            .map(|device| device.id);
        let conversations = control.cloud_uses(self.user().clone(), cloud).await?;
        let mut process_providers: HashMap<ProviderId, bool> = HashMap::new();
        let mut uses = Vec::new();
        for conversation in conversations {
            let provider = !conversation.on_cloud
                && self
                    .needs_a_process(&conversation, &mut process_providers)
                    .await;
            let role = if conversation.on_cloud {
                Role::Target
            } else if provider {
                Role::Provider {
                    attached: conversation.attached,
                }
            } else if conversation.attached {
                Role::Attached
            } else {
                continue;
            };
            uses.push((conversation.id, role));
        }
        Ok(uses)
    }

    /// Whether a node of the conversation infers with a provider that runs
    /// a process: the provider its record names, which its root infers
    /// with, or one a live subagent of its tree infers with.
    async fn needs_a_process(
        &self,
        conversation: &CloudUseRecord,
        known: &mut HashMap<ProviderId, bool>,
    ) -> bool {
        let mut providers = self.tree_providers(&conversation.id);
        providers.extend(conversation.provider.clone());
        for provider in &providers {
            if self.runs_a_process(provider, known).await {
                return true;
            }
        }
        false
    }

    /// Whether the provider entry's provider runs a process on a Host, which
    /// then is the user's Cloud; `known` keeps the answers of one reading.
    /// An entry that cannot be read runs nothing.
    async fn runs_a_process(
        &self,
        provider: &ProviderId,
        known: &mut HashMap<ProviderId, bool>,
    ) -> bool {
        if let Some(runs) = known.get(provider) {
            return *runs;
        }
        let runs = match self.vault().visible(self.user(), provider).await {
            Ok(Some(entry)) => match self.assembly().runs_a_process(&entry).await {
                Ok(runs) => runs,
                Err(error) => {
                    tracing::warn!(provider = %provider, "a conversation's provider could not be built: {error}");
                    false
                }
            },
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(provider = %provider, "a conversation's provider could not be read: {error}");
                false
            }
        };
        known.insert(provider.clone(), runs);
        runs
    }

    /// Holds the conversations an idle stop reaches, if none of them works:
    /// each one's tree and file gate reserved now, or nothing held at all.
    pub(crate) fn hold_uses_for_idle(
        &self,
        uses: &[(ConversationId, Role)],
    ) -> Option<Vec<Box<dyn ConversationHold>>> {
        let mut held = Vec::new();
        for (id, role) in uses {
            if role.needs_cloud() {
                held.push(self.hold_for_idle(id)?);
            }
        }
        Some(held)
    }

    /// Holds the conversations a reset reaches: each one's turn is
    /// interrupted and its tree held, the file transfers and user streams
    /// of one whose files are on the Cloud end, and its file gate is
    /// reserved once the operations holding it ended. Each wait has `hold`;
    /// a conversation that does not let go within it fails the reset.
    pub(crate) async fn hold_uses_for_reset(
        &self,
        uses: &[(ConversationId, Role)],
        hold: Duration,
    ) -> Result<Vec<Box<dyn ConversationHold>>, CloudError> {
        let mut held = Vec::new();
        for (id, role) in uses {
            if !role.needs_cloud() {
                continue;
            }
            let files_on_cloud = *role == Role::Target;
            let conversation = self
                .hold_for_reset(id, files_on_cloud, hold)
                .await
                .ok_or_else(|| {
                    CloudError::Failed(format!("The conversation {id} did not stop for the reset"))
                })?;
            held.push(conversation);
        }
        Ok(held)
    }
}
