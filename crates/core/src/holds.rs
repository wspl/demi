//! What a stop or a reset of a machine whoever started the core runs, such
//! as the server's Cloud, asks of the user's conversations: whether one is
//! attended, and a hold that keeps its tree and its files still.

use std::time::Duration;

use demi_core_host_access::root_of;
use demi_core_host_access::transfer::TransfersClosed;
use demi_shared_gates::Reservation;
use demi_web_api_protocol::ids::{ConversationId, ProviderId};

use super::Shard;

/// A conversation an idle stop holds: its tree and its file gate reserved.
/// The fields drop in order, the tree first.
pub struct IdleHold {
    _tree: Option<Reservation>,
    _files: Reservation,
}

/// A conversation a reset holds: its file gate reserved, its transfers and
/// streams ended if its files are on the machine, and its tree interrupted.
/// The fields drop in order, the file gate first.
pub struct ResetHold {
    _files: Reservation,
    _transfers: Option<TransfersClosed>,
    _tree: Option<Reservation>,
}

impl Shard {
    /// The providers whose processes the conversation's tree runs.
    pub fn tree_providers(&self, conversation: &ConversationId) -> Vec<ProviderId> {
        let Some(tree) = self.agent.tree(&root_of(conversation)) else {
            return Vec::new();
        };
        // An id that is empty names no entry, so no process.
        tree.providers()
            .into_iter()
            .filter_map(|provider| ProviderId::try_from(provider).ok())
            .collect()
    }

    /// Whether the conversation's tree takes a turn or one of its transfers
    /// is open.
    pub fn attended(&self, conversation: &ConversationId) -> bool {
        let turning = self
            .agent
            .tree(&root_of(conversation))
            .is_some_and(|tree| tree.admission().state().demand > 0);
        turning || self.conversations.slot(conversation).transfers.any_open()
    }

    /// A hold of the conversation for an idle stop, taken only when nothing
    /// holds its tree or its files now.
    pub fn hold_for_idle(&self, conversation: &ConversationId) -> Option<IdleHold> {
        let tree = match self.agent.tree(&root_of(conversation)) {
            Some(tree) => Some(tree.admission().try_reserve()?),
            None => None,
        };
        let files = self
            .conversations
            .slot(conversation)
            .file_gate()
            .try_reserve()?;
        Some(IdleHold {
            _tree: tree,
            _files: files,
        })
    }

    /// A hold of the conversation for a reset: its tree interrupted and its
    /// files reserved, its transfers ended when `files_on_machine`; none
    /// when either takes longer than `hold`.
    pub async fn hold_for_reset(
        &self,
        conversation: &ConversationId,
        files_on_machine: bool,
        hold: Duration,
    ) -> Option<ResetHold> {
        let tree = match self.agent.tree(&root_of(conversation)) {
            Some(tree) => Some(
                tokio::time::timeout(hold, tree.interrupt("the Cloud is being reset"))
                    .await
                    .ok()?,
            ),
            None => None,
        };
        let slot = self.conversations.slot(conversation);
        let transfers = if files_on_machine {
            Some(slot.transfers.close().await)
        } else {
            None
        };
        let files = tokio::time::timeout(hold, slot.file_gate().reserve())
            .await
            .ok()?;
        Some(ResetHold {
            _files: files,
            _transfers: transfers,
            _tree: tree,
        })
    }
}
