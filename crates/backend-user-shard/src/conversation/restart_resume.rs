//! A root's turn a graceful shutdown interrupted, resumed once the backend is
//! back.
//!
//! The restart was Demi's doing, not the turn's, so the turn goes on by itself,
//! as the user's Resume would, unless the user's `resumeAfterRestart`
//! preference is off.
//!
//! # Marks
//!
//! Before the shard's close aborts the agent's turns, it marks each live root
//! whose turn is running, in the control database (`interrupted_turns`), so the
//! next start finds them without opening a conversation's database.
//!
//! The mark and the turn's `interrupted` record live in two databases, so the
//! mark comes first. A shutdown cut short, such as by a deployment that stops
//! waiting, still leaves a mark for each root it began to interrupt, and the
//! record is written when the conversation opens again.
//!
//! # Resuming
//!
//! - A marked root resumes when its last turn ended interrupted, whichever
//!   record says so. One whose turn ended otherwise before the abort reached it
//!   does not.
//! - A crash marks nothing: its turn waits for the user, so a turn that brought
//!   the backend down cannot bring it down again.
//! - The start takes each mark before it acts on it, so a turn is resumed once
//!   for each shutdown that interrupted it.
//! - Its children come back with the tree, as a restored tree brings them.
//!
//! # The `resuming` status
//!
//! The restore may wait many seconds, such as for the runner of the
//! conversation's device to come back and its running commands to be taken
//! up. Meanwhile the conversation's status is `resuming`, from the moment its
//! mark is taken with the preference on until its resume settles, so no page
//! shows as failed a turn Demi is about to run. The shard holds the set in
//! memory alone, since it lasts no longer than the process.

use demi_core_store::store::CoreStore;
use demi_web_api_protocol::ids::UserId;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use demi_agent_transcript::INTERRUPTED_CODE;
use demi_core_store::StorageError;
use demi_backend_host_access::{conversation_of, root_of};
use demi_backend_page_sync::Part;
use demi_shared_types::{ErrorBlock, SessionPhase};
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::join_all;

use crate::shard::{Shard, Shards};

/// What the resume tells the model.
pub const RESTART_REASON: &str = "Demi resumed this turn after the backend restarted.";

/// The conversations whose interrupted turn the shard is resuming.
#[derive(Default)]
pub(crate) struct RestartResumes(RefCell<HashSet<ConversationId>>);

impl RestartResumes {
    /// Whether the shard is resuming `conversation`'s turn.
    pub(crate) fn contains(&self, conversation: &ConversationId) -> bool {
        self.0.borrow().contains(conversation)
    }
}

/// A conversation in the shard's `RestartResumes` while it lives; each entry
/// and exit changes the conversation's summary.
struct Resuming<'a> {
    shard: &'a Shard,
    conversation: ConversationId,
    /// The resume started the turn: the root's entry stops saying it resumes
    /// in the step its phase changes, so the guard leaves it alone.
    started: bool,
}

impl<'a> Resuming<'a> {
    fn begin(shard: &'a Shard, conversation: &ConversationId) -> Self {
        shard.restart_resumes().0.borrow_mut().insert(conversation.clone());
        shard.agent().resuming_changed(&root_of(conversation));
        shard.mark(Part::Conversation(conversation.clone()));
        Self { shard, conversation: conversation.clone(), started: false }
    }
}

impl Drop for Resuming<'_> {
    fn drop(&mut self) {
        self.shard.restart_resumes().0.borrow_mut().remove(&self.conversation);
        if !self.started {
            self.shard.agent().resuming_changed(&root_of(&self.conversation));
        }
        self.shard.mark(Part::Conversation(self.conversation.clone()));
    }
}

impl Shard {
    /// Marks each live root whose turn runs, before the shutdown aborts it.
    /// A mark not written leaves that turn to the user's Resume.
    pub(crate) async fn mark_running_roots(&self) {
        let agent = self.agent();
        let running: Vec<(ConversationId, SessionPhase)> = agent
            .live_roots()
            .into_iter()
            .filter_map(|root| {
                let phase = agent.tree(&root)?.root().session().phase();
                (phase != SessionPhase::Idle).then(|| (conversation_of(&root), phase))
            })
            .collect();
        let store = self.store();
        for (conversation, phase) in running {
            match store.mark_interrupted_turn(conversation.clone()).await {
                Ok(_) => tracing::info!(
                    event = "recovery.candidate_chosen",
                    %conversation,
                    stage = "marked",
                    reason = "turn_running",
                    turn_state_at_shutdown = ?phase,
                    "a turn the shutdown interrupts was marked"
                ),
                Err(error) => tracing::error!(
                    event = "turn_mark.failed",
                    %conversation,
                    outcome = "waits_for_resume",
                    error = &error as &dyn std::error::Error,
                    "a turn the shutdown interrupts was not marked"
                ),
            }
        }
    }

    /// Takes up the user's turns a shutdown interrupted, each in its own
    /// course, so that one slow to open holds up no other.
    async fn resume_interrupted_turns(self: Rc<Self>) {
        let started = std::time::Instant::now();
        let marked = match self.store().interrupted_turns().await {
            Ok(marked) => marked,
            Err(error) => {
                // The marks stay for the next start.
                tracing::warn!(event = "interrupted_turns.read_failed", user = %self.user(), step = "marks", %error, "the turns the shutdown interrupted were not read");
                return;
            }
        };
        let on = match self.store().preferences().await {
            Ok(preferences) => preferences.resume_after_restart != Some(false),
            Err(error) => {
                tracing::warn!(event = "interrupted_turns.read_failed", user = %self.user(), step = "preferences", %error, "the preferences were not read for the turns the shutdown interrupted");
                return;
            }
        };
        let candidates = marked.len();
        let settled = join_all(marked.into_iter().map(|conversation| self.resume_interrupted_turn(conversation, on))).await;
        let count = |wanted: &str| settled.iter().filter(|outcome| **outcome == wanted).count();
        tracing::info!(
            event = "recovery.ended",
            user = %self.user(),
            candidates,
            resumed = count("resumed"),
            held_by_preference = count("held"),
            skipped = count("skipped"),
            failed = count("failed"),
            ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "the turns a shutdown interrupted were taken up"
        );
    }

    /// Takes the conversation's mark and, when `on`, resumes its root's turn
    /// if that turn ended interrupted. Answers how it settled: `resumed`,
    /// `held` for the user's preference, `skipped` or `failed`.
    async fn resume_interrupted_turn(&self, conversation: ConversationId, on: bool) -> &'static str {
        // Counted among the tree's openers, so the shard's close waits for
        // it before the agent shuts down; it opens nothing once the close
        // began, and the mark stays for the next start.
        let _opening = self.tree_openers().token();
        if self.is_closing() {
            return "skipped";
        }
        match self.store().take_interrupted_turn(conversation.clone()).await {
            Ok(true) => {}
            Ok(false) => return "skipped",
            Err(error) => {
                tracing::warn!(event = "interrupted_turn.take_up_failed", %conversation, %error, "a turn the shutdown interrupted was not taken up");
                return "failed";
            }
        }
        tracing::info!(
            event = "recovery.candidate_chosen",
            %conversation,
            stage = "taken",
            reason = "mark_present",
            "a turn the shutdown interrupted was taken up"
        );
        if !on {
            tracing::info!(event = "interrupted_turn.settled", %conversation, outcome = "held", reason = "user_preference", "a turn the shutdown interrupted waits for the user");
            return "held";
        }
        let mut resuming = Resuming::begin(self, &conversation);
        let interrupted = |record: &ErrorBlock| record.code.as_deref() == Some(INTERRUPTED_CODE);
        match self
            .resume_unfinished_turn(&conversation, interrupted, Some(RESTART_REASON))
            .await
        {
            Ok(true) => {
                resuming.started = true;
                tracing::info!(event = "interrupted_turn.settled", %conversation, outcome = "resumed", "a turn the shutdown interrupted resumed");
                "resumed"
            }
            Ok(false) => {
                tracing::info!(event = "interrupted_turn.settled", %conversation, outcome = "skipped", reason = "not_interrupted_or_not_ours", "a marked turn was not the conversation's to resume");
                "skipped"
            }
            Err(why) => {
                tracing::warn!(event = "interrupted_turn.settled", %conversation, outcome = "failed", %why, "a turn the shutdown interrupted was not resumed");
                "failed"
            }
        }
    }
}

/// Takes up at start the turns the last shutdown interrupted: each owner's
/// shard resumes its own in the background, so the start waits for none.
pub async fn resume_after_restart(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        if store.interrupted_turns().await?.is_empty() {
            continue;
        }
        let handed = shards.of(owner).adopt(Shard::resume_interrupted_turns).await;
        if handed.is_err() {
            // The backend is shutting down; the next start takes up what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}
