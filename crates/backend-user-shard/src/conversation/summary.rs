//! Conversations as the web app lists them (`web-api.md` § Sidebar
//! mutations, read state and page synchronization): each record with the
//! directory its work runs in, its status from the live tree or else from its
//! last checkpoint, its root's latest ended turn from that same checkpoint,
//! and its output revision. The persisted facts come from a read-only
//! connection, so listing hundreds of conversations takes no writer from a
//! running one.

use demi_backend_database::StorageError;
use demi_backend_database::columns::decode;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::tree::{self, EndedTurn, SummaryFacts, Terminal};
use demi_shared_types::{ModelSelection, SessionPhase};
use demi_web_api_protocol::conversations::{
    ANSWER_START_CHARS, ConversationStatus, ConversationSummary, LastTurn, ModelSettings,
    TurnOutcome,
};
use demi_web_api_protocol::ids::ProviderId;
use futures_util::future::try_join_all;

use crate::shard::Shard;
use demi_backend_host_access::root_of;

impl Shard {
    /// The summaries of the user's conversations that are archived, or that
    /// are not, in sidebar order.
    pub async fn conversation_summaries(
        &self,
        archived: bool,
    ) -> Result<Vec<ConversationSummary>, StorageError> {
        let records = self
            .services()
            .control
            .conversations(self.user().clone(), archived)
            .await?;
        try_join_all(
            records
                .into_iter()
                .map(|record| self.conversation_summary(record)),
        )
        .await
    }

    /// `record` as the web app lists it. The live tree is asked before the
    /// database is read: a tree does nothing by itself only once the save
    /// that ends its action has committed, so the facts read afterwards are
    /// at least that save's, and a status is never an idle tree's view of an
    /// older checkpoint.
    pub async fn conversation_summary(
        &self,
        record: ConversationRecord,
    ) -> Result<ConversationSummary, StorageError> {
        let live = self
            .agent()
            .tree(&root_of(&record.id))
            .map(|tree| (tree.works(), tree.root().session().phase()));
        let facts = self
            .services()
            .conversations
            .read(&record.id, tree::summary)
            .await?
            .unwrap_or(SummaryFacts::EMPTY);
        let status = status(live, &facts);
        let cwd = self
            .host_shard()
            .resolve_target(&record)
            .await?
            .path()
            .to_owned();
        let model = record.model.as_ref().map(settings).transpose()?;
        let plugins_changed = match self.agent().tree(&root_of(&record.id)) {
            Some(tree) => tree.toolset() != self.plugins().revision().await?,
            None => false,
        };
        let plugin_revisions = self.plugins().plugin_revisions(&record.id);
        let permissions_revision = self.permission_revisions().of(&record.id);
        Ok(ConversationSummary {
            unread: facts.revision > record.read_revision,
            title_current: record.user_messages <= record.titled_messages,
            title_generating: self.titles().generating(&record.id),
            plugins_changed,
            id: record.id,
            title: record.title,
            archived: record.archived,
            pinned: record.pinned,
            read_revision: record.read_revision,
            target: record.target,
            context_version: record.context_version,
            model,
            created_at: record.created_at,
            updated_at: record.updated_at,
            cwd,
            status,
            revision: facts.revision,
            last_turn: facts.last_turn.map(last_turn),
            draft_revision: record.draft_revision,
            panel_revision: record.panel_revision,
            hosts_revision: record.hosts_revision,
            plugin_revisions,
            permission_requests: record.permission_requests,
            permissions_revision,
        })
    }
}

/// The model settings a conversation's selection shows (`models.md` § A
/// conversation's model settings).
fn settings(selection: &ModelSelection) -> Result<ModelSettings, StorageError> {
    Ok(ModelSettings {
        provider_id: decode(
            "conversations",
            "model",
            ProviderId::try_from(selection.provider_id.as_str()),
        )?,
        model_id: selection.model.id.clone(),
        thinking_effort: selection.thinking_effort().map(str::to_owned),
        service_tier_id: selection.service_tier_id.clone(),
    })
}

/// The root's latest ended turn as the summary carries it, its answer cut to
/// its start.
fn last_turn(turn: EndedTurn) -> LastTurn {
    LastTurn {
        id: turn.id,
        outcome: match turn.outcome {
            Terminal::Response => TurnOutcome::Finished,
            Terminal::Error => TurnOutcome::Failed,
            Terminal::Abort => TurnOutcome::Stopped,
        },
        answer_start: turn
            .answer
            .map(|answer| answer.chars().take(ANSWER_START_CHARS).collect()),
    }
}

/// A conversation's status: running or compacting while its live tree will
/// go on working without the user, interrupted when its checkpoint was saved
/// in a turn and no tree is live, else what its latest terminal block says.
/// `live` is whether the live tree works, and its root's phase.
fn status(live: Option<(bool, SessionPhase)>, facts: &SummaryFacts) -> ConversationStatus {
    match live {
        Some((true, SessionPhase::Compacting)) => return ConversationStatus::Compacting,
        Some((true, _)) => return ConversationStatus::Running,
        None if facts.phase != SessionPhase::Idle => return ConversationStatus::Interrupted,
        Some((false, _)) | None => {}
    }
    match facts.last {
        Some(Terminal::Response) => ConversationStatus::Completed,
        Some(Terminal::Error) => ConversationStatus::Error,
        Some(Terminal::Abort) => ConversationStatus::Stopped,
        None => ConversationStatus::Idle,
    }
}
