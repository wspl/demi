//! Conversations as the web app lists them.
//!
//! Each record comes with:
//!
//! - the directory its work runs in;
//! - its status, from the live tree or else from its last checkpoint;
//! - its root's latest ended turn, from that same checkpoint;
//! - its output revision.
//!
//! The persisted facts come from a read-only connection, so listing hundreds of
//! conversations takes no writer from a running one.

use demi_backend_database::StorageError;
use demi_backend_database::columns::decode;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::tree::{self, EndedTurn, SummaryFacts, Terminal};
use demi_shared_types::{ModelSelection, SessionPhase};
use demi_web_api_protocol::conversations::{
    ConversationStatus, ConversationSummary, LastTurn, ModelSettings,
    TurnOutcome,
};
use demi_web_api_protocol::ids::ProviderId;
use demi_web_api_protocol::shares::ConversationShare;
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

    /// `record` as the web app lists it.
    ///
    /// The live tree is asked before the database is read: a tree does nothing by
    /// itself only once the save that ends its action has committed, so the facts
    /// read afterwards are at least that save's, and a status is never an idle
    /// tree's view of an older checkpoint.
    pub async fn conversation_summary(
        &self,
        record: ConversationRecord,
    ) -> Result<ConversationSummary, StorageError> {
        let tree = self.agent().tree(&root_of(&record.id));
        let live = tree
            .as_ref()
            .map(|tree| (tree.works(), tree.root().session().phase()));
        let only_waits_for_host = tree.as_ref().is_some_and(|tree| tree.only_waits());
        // A conversation whose database cannot be opened, as one a build
        // that does not know its schema made, is listed without its facts:
        // it alone fails to open, and the user's other conversations and
        // pages work on (`crates/backend-database/src/lib.rs`).
        let facts = match self
            .services()
            .conversations
            .read(&record.id, tree::summary)
            .await
        {
            Ok(facts) => facts.unwrap_or(SummaryFacts::EMPTY),
            Err(error) => {
                tracing::error!(
                    conversation = %record.id,
                    error = &error as &dyn std::error::Error,
                    "a conversation's database could not be read for its summary"
                );
                SummaryFacts::EMPTY
            }
        };
        let status = status(live, &facts);
        let resolved = self.host_shard().resolve_target(&record).await?;
        let cwd = resolved.path().to_owned();
        let target = record.target.shown(&resolved);
        let model = record.model.as_ref().map(settings).transpose()?;
        let plugins_changed = match &tree {
            Some(tree) => tree.toolset() != self.plugins().revision().await?,
            None => false,
        };
        let plugin_revisions = self.plugins().plugin_revisions(&record.id);
        let permissions_revision = self.permission_revisions().of(&record.id);
        let shares = self
            .services()
            .control
            .shares_of(record.id.clone())
            .await?
            .into_iter()
            .map(|share| ConversationShare::new(share.token, share.target))
            .collect();
        let side_questions_revision = self.side_revisions().of(&record.id);
        Ok(ConversationSummary {
            side_questions_revision,
            unread: facts.revision > record.read_revision,
            plugins_changed,
            only_waits_for_host,
            id: record.id,
            title: record.title,
            archived: record.archived,
            pinned: record.pinned,
            read_revision: record.read_revision,
            target,
            context_version: record.context_version,
            model,
            workflow: record.workflow,
            created_at: record.created_at,
            updated_at: record.updated_at,
            cwd,
            status,
            revision: facts.revision,
            last_turn: facts.last_turn.as_ref().map(last_turn),
            draft_revision: record.draft_revision,
            saved: record.saved,
            saved_revision: record.saved_revision,
            panel_revision: record.panel_revision,
            hosts_revision: record.hosts_revision,
            last_attachment: facts.last_attachment,
            plugin_revisions,
            permission_requests: record.permission_requests,
            unread_permission_requests: record.unread_permission_requests,
            permissions_revision,
            shares,
            automation: record.filed,
        })
    }
}

/// The model settings a conversation's selection shows (`crates/shared-types/src/catalog.rs`).
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

/// The root's latest ended turn as the summary carries it.
fn last_turn(turn: &EndedTurn) -> LastTurn {
    LastTurn {
        id: turn.id.clone(),
        outcome: match turn.outcome {
            Terminal::Response => TurnOutcome::Finished,
            Terminal::Error => TurnOutcome::Failed,
            Terminal::Abort => TurnOutcome::Stopped,
        },
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
