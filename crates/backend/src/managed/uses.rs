//! How the user's conversations use the Cloud (`sessions-and-targets.md`
//! § How a conversation uses a device): as the `target` their files and
//! commands are on, as the `provider` their model's process runs on, or
//! `attached` as an extra Host. Every role keeps the Cloud awake while the
//! conversation works; an idle stop and a reset hold the conversations that
//! cannot work without the Cloud, the first two roles, and leave an
//! attached one alone: it runs on its own target. The roles are read when
//! the lifecycle needs them; the `provider` role follows from the
//! conversation's provider, which the placement runs on the Cloud whenever
//! it needs a process (`claude-code.md` § Where it runs).

use std::collections::HashMap;
use std::time::Duration;

use demi_gates::Reservation;
use demi_web_api::ids::{ConversationId, ProviderId};

use super::machine::CloudError;
use crate::conversation::root_of;
use crate::conversation::transfer::TransfersClosed;
use crate::shard::Shard;
use crate::storage::StorageError;

/// How a conversation uses the Cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Target,
    Provider,
    Attached,
}

impl Role {
    /// Whether the conversation cannot work without the Cloud, so a stop or
    /// a reset holds it.
    fn needs_cloud(self) -> bool {
        matches!(self, Self::Target | Self::Provider)
    }
}

/// A conversation a reset holds: its file gate reserved, its transfers and
/// streams ended if its files are on the Cloud, and its tree interrupted.
/// The fields drop in order, the file gate first.
pub(super) struct ResetHold {
    _files: Reservation,
    _transfers: Option<TransfersClosed>,
    _tree: Option<Reservation>,
}

impl Shard {
    /// Each of the user's conversations that uses the Cloud, with its
    /// strongest role.
    pub(super) async fn cloud_uses(&self) -> Result<Vec<(ConversationId, Role)>, StorageError> {
        let control = &self.services().control;
        let cloud = control.managed_device(self.user().clone()).await?.map(|device| device.id);
        let conversations = control.cloud_uses(self.user().clone(), cloud).await?;
        let mut process_providers: HashMap<ProviderId, bool> = HashMap::new();
        let mut uses = Vec::new();
        for conversation in conversations {
            let provider = match &conversation.provider {
                Some(provider) if !conversation.on_cloud => self.runs_a_process(provider, &mut process_providers).await,
                _ => false,
            };
            let role = if conversation.on_cloud {
                Role::Target
            } else if provider {
                Role::Provider
            } else if conversation.attached {
                Role::Attached
            } else {
                continue;
            };
            uses.push((conversation.id, role));
        }
        Ok(uses)
    }

    /// Whether the provider entry's provider runs a process on a Host, which
    /// then is the user's Cloud; `known` keeps the answers of one reading.
    /// An entry that cannot be read runs nothing.
    async fn runs_a_process(&self, provider: &ProviderId, known: &mut HashMap<ProviderId, bool>) -> bool {
        if let Some(runs) = known.get(provider) {
            return *runs;
        }
        let services = self.services();
        let runs = match services.vault.visible(self.user(), provider).await {
            Ok(Some(entry)) => match services.assembly.provider_for(&entry).await {
                Ok(built) => built.capabilities().process_host,
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
    pub(super) fn hold_uses_for_idle(&self, uses: &[(ConversationId, Role)]) -> Option<Vec<Reservation>> {
        let mut held = Vec::new();
        for (id, role) in uses {
            if !role.needs_cloud() {
                continue;
            }
            if let Some(tree) = self.agent().tree(&root_of(id)) {
                held.push(tree.admission().try_reserve()?);
            }
            held.push(self.conversations().slot(id).files.try_reserve()?);
        }
        Some(held)
    }

    /// Holds the conversations a reset reaches: each one's turn is
    /// interrupted and its tree held, the file transfers and user streams
    /// of one whose files are on the Cloud end, and its file gate is
    /// reserved once the operations holding it ended. Each wait has `hold`;
    /// a conversation that does not let go within it fails the reset.
    pub(super) async fn hold_uses_for_reset(
        &self,
        uses: &[(ConversationId, Role)],
        hold: Duration,
    ) -> Result<Vec<ResetHold>, CloudError> {
        let mut held = Vec::new();
        for (id, role) in uses {
            if !role.needs_cloud() {
                continue;
            }
            let stuck = || CloudError::Failed(format!("The conversation {id} did not stop for the reset"));
            let tree = match self.agent().tree(&root_of(id)) {
                Some(tree) => Some(tokio::time::timeout(hold, tree.interrupt()).await.map_err(|_| stuck())?),
                None => None,
            };
            let slot = self.conversations().slot(id);
            let transfers = match role {
                Role::Target => Some(slot.transfers.close().await),
                Role::Provider | Role::Attached => None,
            };
            let files = tokio::time::timeout(hold, slot.files.reserve())
                .await
                .map_err(|_| stuck())?;
            held.push(ResetHold {
                _files: files,
                _transfers: transfers,
                _tree: tree,
            });
        }
        Ok(held)
    }
}
