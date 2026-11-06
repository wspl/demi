//! The push notifications of the user's conversations.
//!
//! Three things notify:
//!
//! - a turn of the root that the user asked for and that ends, with an
//!   answer or an error, once the conversation no longer runs;
//! - a request that asks the user;
//! - what the agent sends with `demi notify send`.
//!
//! The core decides what notifies; its starter decides where each goes and
//! delivers it, and says whether a page already showed it.
//!
//! A turn notifies only when the user asked for it: one a schedule's message
//! or another conversation's started notifies nothing
//! (`demi_agent_session::AgentSession::take_turn_end_asked_by_user`), and its
//! agent tells what is worth telling with `demi notify send`. A schedule's
//! run queued behind the user's turn never hides it (§ Telling turns apart).
//!
//! A turn the user stopped notifies nothing, since the user was there.
//!
//! # Telling turns apart
//!
//! A turn's end is told apart from the turns that ended before by the turn that
//! had ended last when the tree started to work, which the core keeps while it
//! works. The session keeps the latest end of a turn the user asked for, judged
//! as the turn's inputs and end arrived, and the core takes it at each rest:
//! the one it took at the rest after work is the one notified of, so a turn
//! that starts after it never stands in for it, and a compaction or a retry
//! since changes nothing.
//!
//! - A turn that starts and ends between two status changes still notifies.
//! - Work that ends no turn, such as a compaction or a retry the user stopped,
//!   notifies nothing.

use std::cell::RefCell;
use std::collections::HashMap;

use demi_agent_session::{EndedTurn, TurnOutcome};
use demi_core_host_access::root_of;
use demi_shared_types::BlockId;
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::permissions::PermissionRequestId;
use demi_web_api_protocol::push::{ANSWER_START_CHARS, PushAsk, PushMessage};

use crate::Core;

/// What the core keeps for the notifications of its conversations.
#[derive(Default)]
pub(crate) struct Notifications {
    /// Each conversation whose tree works, with the block that ended its
    /// latest ended turn when the work started.
    working: RefCell<HashMap<ConversationId, Option<BlockId>>>,
}

impl Core {
    /// Follows the conversation's status: as its tree starts to run, notes
    /// the turn that had ended last; as it rests, notifies of the latest
    /// turn the user asked for that ended since, unless the user stopped
    /// the last one. Called on each change of the tree's status.
    pub(crate) fn follow_turn_end(&self, conversation: &ConversationId) {
        if self.is_closing() {
            return;
        }
        let tree = self.agent().tree(&root_of(conversation));
        let mut working = self.notifications.working.borrow_mut();
        match tree {
            Some(tree) if tree.is_running() => {
                working
                    .entry(conversation.clone())
                    .or_insert_with(|| tree.root().session().ended_turn());
            }
            // A tree no longer loaded did not end a turn as it closed.
            None => {
                working.remove(conversation);
            }
            Some(tree) => {
                let session = tree.root().session();
                // Taken at every rest, whatever follows, so a turn is
                // notified of at most once, and only at the rest after it.
                let asked = session.take_turn_end_asked_by_user();
                let Some(before) = working.remove(conversation) else {
                    return;
                };
                let Some(last) = session.ended_turn_result() else {
                    return;
                };
                // A turn the user stopped last says the user was there.
                if before.as_ref() == Some(&last.id) || last.outcome == TurnOutcome::Stopped {
                    return;
                }
                let Some(ended) = asked else {
                    return;
                };
                let this = self.this();
                let conversation = conversation.clone();
                self.tasks()
                    .spawn_local(async move { this.notify_turn_end(conversation, ended).await });
            }
        }
    }

    /// Notifies of `ended`, a turn the user asked for that the
    /// conversation's root ended with an answer or an error.
    async fn notify_turn_end(&self, conversation: ConversationId, ended: EndedTurn) {
        let Some(title) = self.conversation_title(&conversation).await else {
            return;
        };
        let turn = ended.id.as_str().to_owned();
        let message = match ended.outcome {
            TurnOutcome::Answered => PushMessage::TurnFinished {
                conversation_id: conversation,
                title,
                turn,
                answer_start: ended
                    .answer
                    .map(|answer| answer.chars().take(ANSWER_START_CHARS).collect()),
            },
            TurnOutcome::Failed => PushMessage::TurnFailed {
                conversation_id: conversation,
                title,
                turn,
            },
            TurnOutcome::Stopped => return,
        };
        self.notify(message).await;
    }

    /// Sends `message` to the user through the starter; a failure is logged.
    async fn notify(&self, message: PushMessage) {
        if let Err(error) = self.starter().notify(message).await {
            tracing::warn!(event = "notification.failed", user = %self.user(), error = &error as &dyn std::error::Error, "a notification was not sent");
        }
    }

    /// Notifies that the conversation asks the user what `ask` says.
    pub(crate) fn notify_asked(&self, conversation: &ConversationId, request: PermissionRequestId, ask: PushAsk) {
        let this = self.this();
        let conversation = conversation.clone();
        self.tasks().spawn_local(async move {
            let Some(title) = this.conversation_title(&conversation).await else {
                return;
            };
            let message = PushMessage::Permission {
                conversation_id: conversation,
                title,
                request,
                ask,
            };
            this.notify(message).await;
        });
    }

    /// The conversation's title, null while it has none; none when the
    /// conversation is gone or cannot be read.
    async fn conversation_title(&self, conversation: &ConversationId) -> Option<Option<String>> {
        match self.store().conversation(conversation.clone()).await {
            Ok(record) => record.map(|record| record.title),
            Err(error) => {
                tracing::warn!(event = "notification.conversation_read_failed", %conversation, %error, "a conversation was not read for its notification");
                None
            }
        }
    }
}
