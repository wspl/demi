//! Where a conversation's work runs (`crates/backend-database/src/selection.rs`): its selection resolved to a device and the directory its work
//! starts in there, which the control store's one rule owns.

use demi_backend_database::conversation_index::{ConversationRecord, ExecutionTarget};
use demi_backend_database::{Selection, StorageError};
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::ids::{ConversationId, UserId};

use crate::HostShard;

impl dyn HostShard + '_ {
    /// The conversation's selection as its device and directory. Resolving
    /// never starts a machine.
    pub async fn resolve_target(
        &self,
        record: &ConversationRecord,
    ) -> Result<ExecutionTarget, StorageError> {
        self.resolve(&record.id, &record.owner, &record.target).await
    }

    /// `selection`, of `owner`'s conversation `id`, as its device and
    /// directory.
    pub async fn resolve(
        &self,
        id: &ConversationId,
        owner: &UserId,
        selection: &Selection,
    ) -> Result<ExecutionTarget, StorageError> {
        self.control()
            .execution_target(owner.clone(), id.clone(), selection.clone())
            .await
    }

    /// The conversation's target as the web app reads it.
    pub async fn shown_target(
        &self,
        record: &ConversationRecord,
    ) -> Result<ConversationTarget, StorageError> {
        let resolved = self.resolve_target(record).await?;
        Ok(record.target.shown(&resolved))
    }
}
