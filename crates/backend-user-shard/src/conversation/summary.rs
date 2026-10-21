//! Conversations as the web app lists and shows them: the sidebar entry of
//! each, and the metadata of one a page shows.
//!
//! An entry comes with its status, from the live tree or else from its last
//! checkpoint, and its output revision. Metadata comes with the directory its
//! work runs in, its root's latest ended turn, from that same checkpoint, the
//! cost of its rows of no agent, and each list its view shows whole under its
//! revision (`docs/decisions/0029-a-conversation-opens-whole.md`): its messages
//! saved for later as the composer lists them, its work panel's tabs with the states plugins hold, its
//! attached hosts, its directory's workflows and its permission requests. Each
//! is read with an indexed query that touches no transcript row.
//!
//! The persisted facts come from a read-only connection, so listing hundreds of
//! conversations takes no writer from a running one.

use demi_backend_database::StorageError;
use demi_backend_database::columns::decode;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::tree::{self, EndedTurn, SummaryFacts, Terminal};
use demi_shared_types::{ModelSelection, SessionPhase};
use demi_web_api_protocol::conversations::{
    ConversationMetadata, ConversationStatus, ConversationSummary, LastTurn, ModelSettings, TurnOutcome,
};
use demi_web_api_protocol::hosts::{AttachedHost, ConversationHosts};
use demi_web_api_protocol::ids::{ConversationId, ProviderId};
use demi_web_api_protocol::shares::ConversationShare;
use futures_util::future::try_join_all;
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;
use demi_backend_host_access::root_of;

impl Shard {
    /// The sidebar entries of the user's conversations that are archived, or
    /// that are not, in sidebar order.
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

    /// `record` as the sidebar lists it.
    ///
    /// The live tree is asked before the database is read: a tree does nothing by
    /// itself only once the save that ends its action has committed, so the facts
    /// read afterwards are at least that save's, and a status is never an idle
    /// tree's view of an older checkpoint.
    pub async fn conversation_summary(
        &self,
        record: ConversationRecord,
    ) -> Result<ConversationSummary, StorageError> {
        let live = self
            .agent()
            .tree(&root_of(&record.id))
            .map(|tree| (tree.works(), tree.root().session().phase()));
        let facts = self.summary_facts(&record.id).await;
        let resolved = self.host_shard().resolve_target(&record).await?;
        Ok(ConversationSummary {
            unread: facts.revision > record.read_revision,
            status: status(live, &facts),
            target: record.target.shown(&resolved),
            id: record.id,
            title: record.title,
            archived: record.archived,
            pinned: record.pinned,
            read_revision: record.read_revision,
            created_at: record.created_at,
            updated_at: record.updated_at,
            revision: facts.revision,
            permission_requests: record.permission_requests,
            unread_permission_requests: record.unread_permission_requests,
            automation: record.filed,
        })
    }

    /// What a page shows of `record` while it shows the conversation.
    pub async fn conversation_metadata(&self, record: ConversationRecord) -> Result<ConversationMetadata, StorageError> {
        let tree = self.agent().tree(&root_of(&record.id));
        let plugins_changed = match &tree {
            Some(tree) => tree.toolset() != self.plugins().revision().await?,
            None => false,
        };
        let facts = self.summary_facts(&record.id).await;
        let resolved = self.host_shard().resolve_target(&record).await?;
        let services = self.services();
        let shares = services
            .control
            .shares_of(record.id.clone())
            .await?
            .into_iter()
            .map(|share| ConversationShare::new(share.token, share.target))
            .collect();
        let workflows = self.conversation_workflows(&record.id).await;
        let hosts = services
            .control
            .attached_host_listing(record.id.clone())
            .await?
            .into_iter()
            .map(|(host, attached_at)| AttachedHost {
                device_id: host.device,
                name: host.name,
                cwd: host.cwd,
                attached_at,
            })
            .collect();
        let saved = services.control.saved_listing(record.id.clone()).await?;
        let panel = self
            .plugins()
            .panel_read(record.id.clone(), CancellationToken::new())
            .await?;
        let permissions = demi_backend_permissions::read(self.permission_shard(), &record.id).await?;
        let other_cost = services
            .control
            .conversation_costs(record.id.clone())
            .await?
            .into_iter()
            .find(|kept| kept.agent.is_none())
            .map(|kept| kept.cost)
            .unwrap_or_default();
        Ok(ConversationMetadata {
            model: record.model.as_ref().map(settings).transpose()?,
            workflow: record.workflow,
            cwd: resolved.path().to_owned(),
            last_turn: facts.last_turn.as_ref().map(last_turn),
            turn_speed: facts.turn_speed,
            plugins_changed,
            other_cost,
            saved,
            panel,
            hosts: ConversationHosts {
                revision: record.hosts_revision,
                hosts,
            },
            workflows,
            last_attachment: facts.last_attachment,
            plugin_revisions: self.plugins().plugin_revisions(&record.id),
            permissions,
            shares,
            id: record.id,
        })
    }

    /// The facts of the conversation `id`'s checkpoint an entry and its
    /// metadata show.
    ///
    /// A conversation whose database cannot be opened, as one a build that
    /// does not know its schema made, is shown without its facts: it alone
    /// fails to open, and the user's other conversations and pages work on
    /// (`crates/backend-database/src/lib.rs`).
    async fn summary_facts(&self, id: &ConversationId) -> SummaryFacts {
        match self.services().conversations.read(id, tree::summary).await {
            Ok(facts) => facts.unwrap_or(SummaryFacts::EMPTY),
            Err(error) => {
                tracing::error!(
                    conversation = %id,
                    error = &error as &dyn std::error::Error,
                    "a conversation's database could not be read for its summary"
                );
                SummaryFacts::EMPTY
            }
        }
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

/// The root's latest ended turn as the metadata carries it.
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
