//! Message editing (`message-editing.md`): an accepted edit replaces a user
//! message and everything after it with a new user turn, commits the
//! rewritten history with its receipt in one save, and then infers on a fresh
//! runtime. A Fork (`conversation-fork.md`) copies a prefix into a new root's
//! first checkpoint.

use std::rc::Rc;

use demi_agent_protocol::{ClientContent, EditRequest, TranscriptVersion};
use demi_command_service::protocol::canonical_digest;
use demi_core::{Block, BlockId, OperationId, UserContentBlock};
use demi_provider::ProviderRuntime;
use tokio::sync::watch;

use super::{
    SessionShared, TurnError, cancel::TurnCancel, compaction, core::SessionCore, persist, turn,
};
use crate::{
    store::{
        BoundaryEdge, Checkpoint, CheckpointState, CheckpointUpdate, CommandStateHistory,
        CommitGuard, EditReceipt,
    },
    transcript::{CutError, TranscriptLog, before_user, through_assistant},
};

/// One part of an edit's content as the session receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditContent {
    Content(UserContentBlock),
    /// The attachment record the edited message holds at this path, which
    /// the session puts in its place (`message-editing.md` § Files the edit
    /// keeps).
    KeptAttachment(String),
}

/// An edit as the session receives it: the request with its content
/// resolved, and the digest of the request as the browser sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditSubmission {
    pub(crate) operation_id: OperationId,
    pub(crate) target: BlockId,
    pub(crate) version: TranscriptVersion,
    pub(crate) content: Vec<EditContent>,
    pub(crate) digest: String,
}

/// Why an edit was rejected. Nothing of the history changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EditError {
    #[error("The edit operation ID was used for a different request")]
    Conflict,
    #[error("Message editing requires a settled session with no pending work")]
    Busy,
    #[error("The conversation changed; reopen the message to edit it")]
    Stale,
    #[error(transparent)]
    Target(#[from] CutError),
    #[error("The edited message holds no attachment at {0}")]
    UnknownAttachment(String),
    #[error("The edit was stopped before it was accepted")]
    Stopped,
    #[error("The agent session is closed")]
    Closed,
    /// Preparing or saving the edit failed, or the node refused it.
    #[error("{0}")]
    Failed(String),
}

/// What the session knows of an edit's operation before the edit runs.
pub(crate) enum EditCheck {
    /// The operation was accepted before: its receipt.
    Accepted(EditReceipt),
    /// The operation is being prepared: its acceptance, to share.
    InFlight(Acceptance),
    /// The operation is new and its snapshot current.
    Proceed,
}

/// The acceptance of an edit in flight, which every caller of the same
/// request shares.
pub(crate) type Acceptance = watch::Receiver<Option<Result<EditReceipt, EditError>>>;

/// An edit the session admitted and has not yet accepted or rejected.
pub(super) struct EditInFlight {
    pub(super) operation_id: OperationId,
    pub(super) digest: String,
    /// Taken by the action when it starts.
    pub(super) submission: Option<EditSubmission>,
    /// Set once the save committed: the edit is history now.
    pub(super) accepted: bool,
    acceptance: watch::Sender<Option<Result<EditReceipt, EditError>>>,
}

impl EditInFlight {
    pub(super) fn new(submission: EditSubmission) -> Self {
        Self {
            operation_id: submission.operation_id.clone(),
            digest: submission.digest.clone(),
            submission: Some(submission),
            accepted: false,
            acceptance: watch::Sender::new(None),
        }
    }

    pub(super) fn acceptance(&self) -> Acceptance {
        self.acceptance.subscribe()
    }

    pub(super) fn resolve(&self, result: Result<EditReceipt, EditError>) {
        self.acceptance.send_replace(Some(result));
    }
}

/// Waits for an edit's acceptance.
pub(crate) async fn accepted(mut acceptance: Acceptance) -> Result<EditReceipt, EditError> {
    match acceptance.wait_for(Option::is_some).await {
        Ok(result) => result.clone().expect("the acceptance was set"),
        // The session went away before it decided.
        Err(_) => Err(EditError::Closed),
    }
}

/// The SHA-256 of an edit request's RFC 8785 canonical JSON, as the browser
/// sent it, before its uploads are resolved (`message-editing.md` § Commit
/// and idempotency).
pub(crate) fn edit_digest(request: &EditRequest<ClientContent>) -> String {
    canonical_digest(request).expect("an edit request serializes")
}

/// The edit action (`message-editing.md` § Behavior and ownership):
/// prepares the replacement turn beside the accepted history, commits it
/// with its receipt and the command state before the target in one save,
/// adopts it, closes the runtimes it discarded, and runs the replacement's
/// turn on a runtime fork. Until the save commits, nothing of the history
/// changes.
pub(super) async fn run(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    // Held until the edit is accepted or rejected: the replacement's turn
    // may start and close children again.
    let reservation = cancel
        .guard(s.runtime.reserve_edit())
        .await?
        .map_err(TurnError::refused)?;
    cancel.check()?;
    // Earlier throttled checkpoints settle before the edit's save.
    persist::flush(s).await?;
    let submission = s
        .update(|core| {
            core.editing
                .as_mut()
                .and_then(|edit| edit.submission.take())
        })
        .expect("an edit action has its submission");
    let preamble = cancel.guard(s.runtime.preamble()).await?;
    let Candidate {
        blocks,
        commands,
        mut runtime,
        update,
        receipt,
        model,
    } = s
        .read(|core| Candidate::prepare(core, s, &submission, preamble))
        .map_err(|error| TurnError::refused(error.to_string()))?;
    let saved = {
        let _turn = s.persist_gate.acquire().await;
        match cancel.check() {
            // A save that started always finishes: its outcome decides.
            Ok(()) => s
                .store
                .save(update, &CommitGuard::default())
                .await
                .map_err(TurnError::from),
            Err(stopped) => Err(stopped),
        }
    };
    if let Err(error) = saved {
        // The history is as it was, and the candidate's runtime served
        // nothing.
        runtime.close().await;
        return Err(error);
    }
    // Nothing awaits between the commit and the adoption.
    let discarded = s.update(|core| core.adopt_edit(blocks, commands, runtime, model, receipt));
    // The callers waiting for the acceptance answer before the replacement's
    // turn writes anything, so an `edit_result` precedes its turn's frames.
    tokio::task::yield_now().await;
    for mut runtime in discarded {
        runtime.close().await;
    }
    drop(reservation);
    cancel.check()?;
    compaction::preflight(s, cancel).await?;
    turn::run(s, cancel).await
}

/// An edit's replacement history, prepared beside the accepted one.
struct Candidate {
    blocks: Vec<Block>,
    commands: CommandStateHistory,
    runtime: Box<dyn ProviderRuntime>,
    update: CheckpointUpdate,
    receipt: EditReceipt,
    model: demi_core::ModelSelection,
}

impl Candidate {
    fn prepare(
        core: &SessionCore,
        s: &SessionShared,
        submission: &EditSubmission,
        preamble: Option<String>,
    ) -> Result<Self, EditError> {
        let blocks = core.transcript.blocks();
        let prefix = before_user(blocks, &submission.target)?.to_vec();
        let Some(Block::User(target)) = blocks.get(prefix.len()) else {
            unreachable!("the prefix ends before the user block");
        };
        let content = submission
            .content
            .iter()
            .map(|part| match part {
                EditContent::Content(block) => Ok(block.clone()),
                EditContent::KeptAttachment(path) => target
                    .content
                    .iter()
                    .find(|kept| matches!(kept, UserContentBlock::Attachment(record) if &record.path == path))
                    .cloned()
                    .ok_or_else(|| EditError::UnknownAttachment(path.clone())),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let revision = core
            .commands
            .boundary(&submission.target, BoundaryEdge::BeforeUser)
            .ok_or_else(|| {
                EditError::Failed(format!(
                    "No command-state boundary before {}",
                    submission.target
                ))
            })?;
        let model = core.latest_selection().clone();
        let turn = core.turn();
        let mut candidate = TranscriptLog::new(prefix, s.ids.clone(), core.clock());
        let user = candidate.push_user(turn.clone(), &model, content, preamble);
        let blocks = candidate.blocks().to_vec();
        let mut commands =
            CommandStateHistory::restore(core.commands.select(&blocks, revision, false))
                .expect("a cut of a valid command state is valid");
        commands.capture(user.clone(), BoundaryEdge::BeforeUser, revision);
        commands.capture(user, BoundaryEdge::AfterBlock, revision);
        let runtime = core
            .switch
            .as_ref()
            .and_then(|switch| switch.runtime.as_ref())
            .or(core.provider.as_ref())
            .expect("an idle session's runtime is in its slot")
            .fresh();
        let receipt = EditReceipt {
            operation_id: submission.operation_id.clone(),
            digest: submission.digest.clone(),
            turn_id: turn,
        };
        let mut edits = core.edits.clone();
        edits.push(receipt.clone());
        let update = CheckpointUpdate {
            state: CheckpointState {
                phase: demi_core::SessionPhase::Running,
                queue: Vec::new(),
                edits,
                model: model.clone(),
                ..core.checkpoint_state()
            },
            command_state: Some(commands.snapshot(None)),
            changed_blocks: blocks.iter().cloned().enumerate().collect(),
            block_count: blocks.len(),
        };
        Ok(Self {
            blocks,
            commands,
            runtime,
            update,
            receipt,
            model,
        })
    }
}

/// Why a Fork cannot start where it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForkError {
    #[error(transparent)]
    Target(#[from] CutError),
    #[error("No command-state boundary after the Fork target")]
    NoBoundary,
    #[error("The Fork source must be a root session")]
    NotRoot,
    #[error("No matching Fork source checkpoint")]
    NoCheckpoint,
    #[error("A Fork must start idle, without queued actions or edit receipts, of this harness")]
    InvalidSeed,
    #[error("{0}")]
    Store(String),
}

/// A Fork's seed (`conversation-fork.md` § The fork seed): a root checkpoint
/// of the history through the completed text `target`, with the command
/// state bound to that text's completion and the versions its boundaries
/// refer to, idle, with nothing waiting.
pub(crate) fn fork_seed(
    blocks: &[Block],
    commands: &CommandStateHistory,
    state: CheckpointState,
    target: &BlockId,
) -> Result<Checkpoint, ForkError> {
    let prefix = through_assistant(blocks, target)?.to_vec();
    let revision = commands
        .boundary(target, BoundaryEdge::AfterAssistant)
        .ok_or(ForkError::NoBoundary)?;
    Ok(Checkpoint {
        state: CheckpointState {
            phase: demi_core::SessionPhase::Idle,
            queue: Vec::new(),
            agent_inputs: Vec::new(),
            wakeups: Vec::new(),
            edits: Vec::new(),
            ..state
        },
        command_state: commands.select(&prefix, revision, true),
        transcript: prefix,
    })
}
