//! Conversations as the browser lists them (`web-api.md` § Sidebar
//! mutations, read state and page synchronization): each record with the
//! directory its work runs in, its status from the live tree or else from its
//! last checkpoint, and its output revision. The persisted facts come from a
//! read-only connection, so listing hundreds of conversations takes no
//! writer from a running one.

use demi_core::SessionPhase;
use demi_web_api::conversations::{ConversationStatus, ConversationSummary};
use futures_util::future::try_join_all;

use super::root_of;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::ConversationRecord;
use crate::storage::tree::{self, SummaryFacts, Terminal};

impl Shard {
    /// The summaries of the user's conversations that are archived, or that
    /// are not, in sidebar order.
    pub(crate) async fn conversation_summaries(&self, archived: bool) -> Result<Vec<ConversationSummary>, StorageError> {
        let records = self.services().control.conversations(self.user().clone(), archived).await?;
        try_join_all(records.into_iter().map(|record| self.conversation_summary(record))).await
    }

    /// `record` as the browser lists it.
    pub(crate) async fn conversation_summary(&self, record: ConversationRecord) -> Result<ConversationSummary, StorageError> {
        let facts = self
            .services()
            .conversations
            .read(&record.id, tree::summary)
            .await?
            .unwrap_or(SummaryFacts::EMPTY);
        let live = self
            .agent()
            .tree(&root_of(&record.id))
            .map(|tree| (tree.is_quiescent(), tree.root().session().phase()));
        let status = status(live, &facts);
        let cwd = self.resolve_target(&record).await?.path().to_owned();
        let (provider_id, model_id) = match record.model {
            Some(model) => (Some(model.provider), Some(model.model)),
            None => (None, None),
        };
        Ok(ConversationSummary {
            unread: facts.revision > record.read_revision,
            id: record.id,
            title: record.title,
            archived: record.archived,
            pinned: record.pinned,
            read_revision: record.read_revision,
            target: record.target,
            context_version: record.context_version,
            provider_id,
            model_id,
            created_at: record.created_at,
            updated_at: record.updated_at,
            cwd,
            status,
            revision: facts.revision,
        })
    }
}

/// A conversation's status: running or compacting while its live tree will
/// go on working without the user, interrupted when its checkpoint was saved
/// in a turn and no tree is live, else what its latest terminal block says.
fn status(live: Option<(bool, SessionPhase)>, facts: &SummaryFacts) -> ConversationStatus {
    match live {
        Some((false, SessionPhase::Compacting)) => return ConversationStatus::Compacting,
        Some((false, _)) => return ConversationStatus::Running,
        None if facts.phase != SessionPhase::Idle => return ConversationStatus::Interrupted,
        Some((true, _)) | None => {}
    }
    match facts.last {
        Some(Terminal::Response) => ConversationStatus::Completed,
        Some(Terminal::Error) => ConversationStatus::Error,
        Some(Terminal::Abort) => ConversationStatus::Stopped,
        None => ConversationStatus::Idle,
    }
}
