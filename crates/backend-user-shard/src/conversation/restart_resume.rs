//! A root's turn a graceful shutdown interrupted, resumed once the backend
//! is back: the restart was Demi's doing, not the turn's, so the turn goes
//! on by itself, as the user's Resume would, unless the user's
//! `resumeAfterRestart` preference is off.
//!
//! Before the shard's close aborts the agent's turns, it marks each live
//! root whose turn is running, in the control database
//! (`interrupted_turns`), so the next start finds them without opening a
//! conversation's database. The mark and the turn's `interrupted` record
//! live in two databases, so the mark comes first: a shutdown cut short,
//! such as by a deployment that stops waiting, still leaves a mark for
//! each root it began to interrupt, and the record is written when the
//! conversation opens again. A marked root resumes when its last turn ended
//! interrupted, whichever record says so; one whose turn ended otherwise
//! before the abort reached it does not. A crash marks nothing: its turn
//! waits for the user, so a turn that brought the backend down cannot bring
//! it down again. The start takes each mark before it acts on it, so a turn
//! is resumed once for each shutdown that interrupted it; its children come
//! back with the tree, as a restored tree brings them.

use std::rc::Rc;

use demi_agent_transcript::INTERRUPTED_CODE;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_host_access::conversation_of;
use demi_shared_types::{ErrorBlock, SessionPhase};
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::join_all;

use crate::shard::{Shard, Shards};

/// What the resume tells the model.
pub const RESTART_REASON: &str = "Demi resumed this turn after the backend restarted.";

impl Shard {
    /// Marks each live root whose turn runs, before the shutdown aborts it.
    /// A mark not written leaves that turn to the user's Resume.
    pub(crate) async fn mark_running_roots(&self) {
        let agent = self.agent();
        let running: Vec<ConversationId> = agent
            .live_roots()
            .into_iter()
            .filter(|root| {
                agent
                    .tree(root)
                    .is_some_and(|tree| tree.root().session().phase() != SessionPhase::Idle)
            })
            .map(|root| conversation_of(&root))
            .collect();
        let control = &self.services().control;
        for conversation in running {
            if let Err(error) = control.mark_interrupted_turn(conversation.clone()).await {
                tracing::error!(
                    %conversation,
                    error = &error as &dyn std::error::Error,
                    "a turn the shutdown interrupts was not marked; it waits for the user's Resume"
                );
            }
        }
    }

    /// Takes up the user's turns a shutdown interrupted, each in its own
    /// course, so that one slow to open holds up no other.
    async fn resume_interrupted_turns(self: Rc<Self>) {
        let services = self.services();
        let marked = match services.control.interrupted_turns(self.user().clone()).await {
            Ok(marked) => marked,
            Err(error) => {
                // The marks stay for the next start.
                tracing::warn!(user = %self.user(), "the turns the shutdown interrupted were not read: {error}");
                return;
            }
        };
        let on = match services.control.preferences(self.user().clone()).await {
            Ok(preferences) => preferences.resume_after_restart != Some(false),
            Err(error) => {
                tracing::warn!(user = %self.user(), "the preferences were not read for the turns the shutdown interrupted: {error}");
                return;
            }
        };
        join_all(marked.into_iter().map(|conversation| self.resume_interrupted_turn(conversation, on))).await;
    }

    /// Takes the conversation's mark and, when `on`, resumes its root's turn
    /// if that turn ended interrupted.
    async fn resume_interrupted_turn(&self, conversation: ConversationId, on: bool) {
        // Counted among the tree's openers, so the shard's close waits for
        // it before the agent shuts down; it opens nothing once the close
        // began, and the mark stays for the next start.
        let _opening = self.tree_openers().token();
        if self.is_closing() {
            return;
        }
        match self.services().control.take_interrupted_turn(conversation.clone()).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::warn!(%conversation, "a turn the shutdown interrupted was not taken up: {error}");
                return;
            }
        }
        if !on {
            tracing::info!(%conversation, "a turn the shutdown interrupted waits for the user, as their preference says");
            return;
        }
        let interrupted = |record: &ErrorBlock| record.code.as_deref() == Some(INTERRUPTED_CODE);
        match self
            .resume_unfinished_turn(&conversation, interrupted, Some(RESTART_REASON))
            .await
        {
            Ok(true) => tracing::info!(%conversation, "a turn the shutdown interrupted resumed"),
            Ok(false) => {
                tracing::info!(%conversation, "the marked turn did not end interrupted, or is no longer the conversation's to resume");
            }
            Err(why) => {
                tracing::warn!(%conversation, "a turn the shutdown interrupted was not resumed: {why}");
            }
        }
    }
}

/// Takes up at start the turns the last shutdown interrupted: each owner's
/// shard resumes its own in the background, so the start waits for none.
pub async fn resume_after_restart(control: &ControlService, shards: &Shards) -> Result<(), StorageError> {
    for owner in control.interrupted_turn_owners().await? {
        let handed = shards.of(&owner).adopt(Shard::resume_interrupted_turns).await;
        if handed.is_err() {
            // The backend is shutting down; the next start takes up what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}
