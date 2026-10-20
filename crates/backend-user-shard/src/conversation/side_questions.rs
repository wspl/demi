//! A conversation's side questions on the backend.
//!
//! - Asking one opens the conversation's tree when it is closed and starts the
//!   side's node there, beside the root's turn.
//! - Its card's title is written by the user's small model, as a
//!   conversation's.
//! - Its follow-ups and Retry are turns of its node.
//! - It has a fold.
//! - It is an agent of the conversation: its title and fold, which the backend
//!   keeps, reach its agent entry as they change
//!   (`crates/agent-server/src/server/tree/agents.rs`).

use demi_agent_server::AskError;
use demi_agent_store::SideOf;
use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::side_questions;
use demi_backend_host_access::root_of;
use demi_shared_types::{NodeId, markdown_quote};
use demi_web_api_protocol::ids::ConversationId;
use demi_conversation_socket_protocol::SideQuestion;
use demi_web_api_protocol::side_questions::AskRequest;

use crate::shard::Shard;

/// Why a side question's request was refused.
#[derive(Debug, thiserror::Error)]
pub enum SideRefusal {
    #[error("No such conversation")]
    ConversationNotFound,
    #[error("The conversation is archived")]
    Archived,
    #[error("The conversation has no side question of that id")]
    NotFound,
    /// The id belongs to another side question, or to an agent.
    #[error("The side question's id belongs to another question")]
    Conflict,
    /// The block is not a completed assistant text of the conversation's
    /// history.
    #[error("{0}")]
    Target(String),
    /// The side's session refused, as a Retry with no failed turn does.
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("{0}")]
    Failed(String),
}

impl From<AskError> for SideRefusal {
    fn from(error: AskError) -> Self {
        match error {
            AskError::NotOpen => Self::Failed("the conversation's tree is not open".to_owned()),
            AskError::Target(reason) => Self::Target(reason),
            AskError::Taken => Self::Conflict,
            AskError::Unknown => Self::NotFound,
            AskError::Refused(reason) => Self::Refused(reason),
            AskError::Failed(reason) => Self::Failed(reason),
        }
    }
}

impl Shard {
    /// The user's conversation `id`, which a side question's request
    /// changes only while it is not archived.
    async fn side_conversation(&self, id: &ConversationId, change: bool) -> Result<ConversationRecord, SideRefusal> {
        let record = self
            .services()
            .control
            .conversation(id.clone())
            .await?
            .filter(|record| record.owner == *self.user())
            .ok_or(SideRefusal::ConversationNotFound)?;
        if change && record.archived {
            return Err(SideRefusal::Archived);
        }
        Ok(record)
    }

    /// Asks the side question `request` of the user's conversation `id`,
    /// opening its tree when it is closed, and starts the request titling
    /// its card. The same request again answers the side question as it is,
    /// and false for whether this request asked it.
    pub async fn ask(&self, id: ConversationId, request: AskRequest) -> Result<(SideQuestion, bool), SideRefusal> {
        let record = self.side_conversation(&id, true).await?;
        let side = NodeId::try_from(request.id).map_err(|_| SideRefusal::Conflict)?;
        {
            // Counted before the open, so the close waits for it before the
            // agent shuts down.
            let _opening = self.tree_openers().token();
            if self.is_closing() {
                return Err(SideRefusal::Failed("the backend is shutting down".to_owned()));
            }
            self.restore_tree(&record.id).await.map_err(SideRefusal::Failed)?;
        }
        let of = SideOf {
            block: request.block_id,
            start: request.start,
            end: request.end,
            quote: request.quote,
            picture: request.picture,
            question: request.question,
            // The tree takes the time it starts the side at.
            asked_at: self.services().clock.now(),
        };
        let message = format!("{}\n\n{}", markdown_quote(&of.quote), of.question);
        let created = self.agent().ask(&root_of(&record.id), side.clone(), of).await?;
        let asked = self.side_question(&record.id, &side).await?;
        if !created {
            return Ok((asked, false));
        }
        if asked.title.is_none()
            && let Some(selection) = &record.model
        {
            let write = {
                let shard = self.this();
                let conversation = record.id.clone();
                let side = side.clone();
                Box::new(move |title: String| {
                    Box::pin(async move {
                        let written = {
                            let side = side.clone();
                            shard
                                .services()
                                .conversations
                                .db(&conversation)
                                .call(move |connection| side_questions::set_title(connection, &side, &title))
                                .await?
                        };
                        if written {
                            shard.card_changed(&conversation, &side).await?;
                        }
                        Ok(())
                    }) as futures_util::future::LocalBoxFuture<'static, Result<(), StorageError>>
                }) as super::titles::WriteSideTitle
            };
            self.titles()
                .start_side(self.tasks(), record.id.clone(), side, selection.clone(), message, write);
        }
        Ok((asked, true))
    }

    /// Sends the follow-up `text` to the side question `side` of the user's
    /// conversation `id`.
    pub async fn ask_more(&self, id: ConversationId, side: NodeId, text: String) -> Result<(), SideRefusal> {
        let record = self.side_conversation(&id, true).await?;
        self.side_question(&record.id, &side).await?;
        self.open_for_side(&record.id).await?;
        self.agent().ask_more(&root_of(&record.id), side, text).await?;
        Ok(())
    }

    /// Runs the failed turn of the side question `side` of the user's
    /// conversation `id` again.
    pub async fn retry_side(&self, id: ConversationId, side: NodeId) -> Result<(), SideRefusal> {
        let record = self.side_conversation(&id, true).await?;
        self.side_question(&record.id, &side).await?;
        self.open_for_side(&record.id).await?;
        self.agent().retry_side(&root_of(&record.id), side).await?;
        Ok(())
    }

    /// Folds the card of the side question `side` of the user's
    /// conversation `id` into the margin's mark, or opens it again.
    pub async fn fold_side(&self, id: ConversationId, side: NodeId, folded: bool) -> Result<SideQuestion, SideRefusal> {
        let record = self.side_conversation(&id, false).await?;
        let folded = {
            let side = side.clone();
            self.services()
                .conversations
                .db(&record.id)
                .call(move |connection| side_questions::set_folded(connection, &side, folded))
                .await?
                .ok_or(SideRefusal::NotFound)?
        };
        self.card_changed(&record.id, &side).await?;
        Ok(folded)
    }

    async fn side_question(&self, conversation: &ConversationId, side: &NodeId) -> Result<SideQuestion, SideRefusal> {
        let side = side.clone();
        self.services()
            .conversations
            .read(conversation, move |connection| side_questions::one(connection, &side))
            .await?
            .flatten()
            .ok_or(SideRefusal::NotFound)
    }

    async fn open_for_side(&self, conversation: &ConversationId) -> Result<(), SideRefusal> {
        let _opening = self.tree_openers().token();
        if self.is_closing() {
            return Err(SideRefusal::Failed("the backend is shutting down".to_owned()));
        }
        self.restore_tree(conversation).await.map_err(SideRefusal::Failed)
    }

    /// The card of the side question `side` of `conversation` changed: the
    /// live tree's entry of it shows it as it is now. A tree that is not live
    /// reads it when it opens.
    async fn card_changed(&self, conversation: &ConversationId, side: &NodeId) -> Result<(), StorageError> {
        let Some(tree) = self.agent().tree(&root_of(conversation)) else {
            return Ok(());
        };
        let Some(card) = self.side_question(conversation, side).await.ok() else {
            return Ok(());
        };
        tree.side_card_changed(
            side,
            demi_agent_server::SideCard {
                title: card.title,
                folded: card.folded,
            },
        );
        Ok(())
    }
}

/// The side questions context source: what the root learns of the
/// questions the user asked aside, which its tree decides; nothing while
/// the tree is not open, as a closed tree's root makes no request.
pub(crate) struct Told {
    shard: std::rc::Weak<Shard>,
}

impl Told {
    pub(crate) fn new(shard: std::rc::Weak<Shard>) -> Self {
        Self { shard }
    }
}

impl demi_agent_tools::ContextSource for Told {
    fn name(&self) -> &str {
        demi_shared_types::SIDE_QUESTIONS_SOURCE
    }

    fn context<'a>(
        &'a self,
        node: demi_agent_tools::NodeContext<'a>,
        _: &'a demi_shared_types::TurnId,
        seen: &'a [&'a str],
    ) -> futures_util::future::LocalBoxFuture<'a, Result<Option<demi_agent_tools::ContextAnswer>, String>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            let Some(tree) = shard.agent().tree(node.root) else {
                return Ok(None);
            };
            Ok(tree
                .side_questions_context(node.node, seen)
                .await
                .map_err(|error| error.to_string())?
                .map(demi_agent_tools::ContextAnswer::from))
        })
    }
}
