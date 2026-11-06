//! Where a conversation's work runs
//! (`crates/core-store/src/selection.rs`): its selection resolved to a
//! device and the directory its work starts in there.
//!
//! The core store's one rule owns the resolution.

use demi_core_store::conversation_index::{ConversationRecord, ExecutionTarget};
use demi_core_store::Selection;
use demi_core_store::StorageError;
use demi_web_api_protocol::ids::ConversationId;

use crate::HostShard;

impl dyn HostShard + '_ {
    /// The conversation's selection as its device and directory. Resolving
    /// never starts a machine.
    pub async fn resolve_target(
        &self,
        record: &ConversationRecord,
    ) -> Result<ExecutionTarget, StorageError> {
        self.resolve(&record.id, &record.target).await
    }

    /// `selection`, of the conversation `id`, as its device and directory.
    pub async fn resolve(
        &self,
        id: &ConversationId,
        selection: &Selection,
    ) -> Result<ExecutionTarget, StorageError> {
        self.store()
            .execution_target(id.clone(), selection.clone())
            .await
    }

}
