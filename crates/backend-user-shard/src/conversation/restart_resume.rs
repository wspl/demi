//! A root's turn a graceful shutdown interrupted, resumed once the backend
//! is back (`failures-and-recovery.md` § The unfinished turn): the restart
//! was Demi's doing, not the turn's, so the turn goes on by itself, as the
//! user's Resume would, unless the user's `resumeAfterRestart` preference
//! is off.
//!
//! The shard's close marks each live root whose turn its shutdown ended
//! with the `interrupted` record it wrote, in the control database
//! (`interrupted_turns`), so the next start finds them without opening a
//! conversation's database. A crash marks nothing: its record is written
//! when the conversation is next opened, and that turn waits for the user,
//! so a turn that brought the backend down cannot bring it down again.
//! The start takes each mark as it takes the turn up, so a turn is resumed
//! once for each shutdown that interrupted it; its children come back with
//! the tree, as a restored tree brings them.

use std::rc::Rc;

use demi_agent_server::Node;
use demi_agent_transcript::INTERRUPTED_CODE;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::interrupted_turns::InterruptedTurn;
use demi_backend_host_access::conversation_of;
use demi_shared_types::BlockId;
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::join_all;

use crate::conversation::ShardHosts;
use crate::shard::{Shard, Shards};

/// What the resume tells the model.
pub const RESTART_REASON: &str = "Demi resumed this turn after the backend restarted.";

/// The live roots as the shutdown finds them, each with the record of its
/// last turn when that turn was unfinished then.
pub(crate) struct LiveRoots(Vec<(ConversationId, Rc<Node<ShardHosts>>, Option<BlockId>)>);

impl Shard {
    /// The live roots, read before the agent shuts down.
    pub(crate) fn live_roots(&self) -> LiveRoots {
        let agent = self.agent();
        LiveRoots(
            agent
                .live_roots()
                .into_iter()
                .filter_map(|root| {
                    let node = agent.tree(&root)?.root().clone();
                    let before = node.session().failed_turn().map(|record| record.id);
                    Some((conversation_of(&root), node, before))
                })
                .collect(),
        )
    }

    /// Marks each of `roots` whose turn the shutdown just ended with an
    /// `interrupted` record of its own, one its last turn did not end with
    /// before. A mark not written leaves that turn to the user's Resume.
    pub(crate) async fn mark_interrupted_turns(&self, roots: LiveRoots) {
        let control = &self.services().control;
        for (conversation, node, before) in roots.0 {
            let Some(record) = node.session().failed_turn() else {
                continue;
            };
            if record.code.as_deref() != Some(INTERRUPTED_CODE) || before.as_ref() == Some(&record.id) {
                continue;
            }
            if let Err(error) = control.mark_interrupted_turn(conversation.clone(), record.id).await {
                tracing::error!(
                    %conversation,
                    error = &error as &dyn std::error::Error,
                    "a turn the shutdown interrupted was not marked; it waits for the user's Resume"
                );
            }
        }
    }

    /// Takes up the user's turns a shutdown interrupted, each in its own
    /// course, so that one slow to open holds up no other.
    async fn resume_interrupted_turns(self: Rc<Self>) {
        let services = self.services();
        let turns = match services.control.interrupted_turns(self.user().clone()).await {
            Ok(turns) => turns,
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
        join_all(turns.into_iter().map(|turn| self.resume_interrupted_turn(turn, on))).await;
    }

    /// Takes the mark `turn` and resumes its turn when `on`.
    async fn resume_interrupted_turn(&self, turn: InterruptedTurn, on: bool) {
        // Counted among the tree's openers, so the shard's close waits for
        // it before the agent shuts down; it opens nothing once the close
        // began, and the mark stays for the next start.
        let _opening = self.tree_openers().token();
        if self.is_closing() {
            return;
        }
        let InterruptedTurn { conversation, error } = turn.clone();
        match self.services().control.take_interrupted_turn(turn).await {
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
        match self.resume_unfinished_turn(&conversation, &error, Some(RESTART_REASON)).await {
            Ok(true) => tracing::info!(%conversation, "a turn the shutdown interrupted resumed"),
            Ok(false) => {
                tracing::info!(%conversation, "a turn the shutdown interrupted is no longer the conversation's to resume");
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
