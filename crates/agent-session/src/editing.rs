//! Message editing: an accepted edit continues the conversation from a new
//! version of a user message, and Regenerate from a new version of its answer.
//!
//! Either commits the new branch with its receipt in one save, leaving the old
//! one stored, and then infers on a fresh runtime. A switch of versions makes
//! another branch current. A Fork copies a prefix into a new root's first
//! checkpoint.
//!
//! The session owns the operation, and the transcript the suffix replacement
//! and the patch that replicates it.
//!
//! The replacement infers on a fork of the provider runtime, so that no state
//! the provider kept about the removed history reaches it. It takes a new turn
//! ID, the selected model and a freshly prepared preamble. The retained blocks
//! keep their IDs, timestamps, content and media.
//!
//! Only a `user` block is a target: an explicit submission has its own block
//! type, so the session and the page read editability from the type and no
//! second rule decides it.
//!
//! A request is checked before its uploads are resolved, so that a repeated one
//! writes no file. The checks run in this order:
//!
//! 1. its operation's receipt;
//! 2. an edit with its operation ID still being prepared;
//! 3. its snapshot.
//!
//! The digest covers the request as the page sent it, because resolving an
//! upload writes the file under a name not yet taken, so the same request
//! resolved twice would differ while its retry must match. A snapshot taken
//! before a restart is always stale, since every restore starts a new epoch.

use std::rc::Rc;

use demi_agent_store::{
    AgentTreeStore, Checkpoint, CheckpointState, CheckpointUpdate, EditReceipt, Kind, NodeRecord, SessionStore, StoreError,
    StoredPrefix,
};
use demi_agent_transcript::{CutError, Earlier, ForkEnd, TranscriptLog, before_user, fork_prefix, side_end};
use demi_command_protocol::canonical_digest;
use demi_conversation_socket_protocol::{
    AgentCode, EditRequest, MediaRef, RegenerateRequest, Refusal, TranscriptVersion,
};
use demi_provider_common::ProviderRuntime;
use demi_shared_types::{
    BlobRef, Block, BlockId, DocumentSource, MediaSource, NodeId, OperationId, UserBlock, UserContentBlock,
};
use tokio::sync::watch;

use super::{
    ErrorReport,
    SessionShared, TurnError, cancel::TurnCancel, compaction, core::SessionCore, persist, turn, window,
};

/// One part of an edit's content as the session receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditContent {
    Content(UserContentBlock),
    /// The attachment record the edited message holds under this number,
    /// such as `a7`, which the session puts in its place.
    KeptAttachment(String),
    /// The native media block of this kind the edited message holds by the
    /// blob it names, which the session puts in its place.
    KeptMedia(MediaRef),
}

/// An edit as the session receives it: the request with its content
/// resolved, and the digest of the request as the web app sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSubmission {
    pub operation_id: OperationId,
    pub target: BlockId,
    pub version: TranscriptVersion,
    /// The new version's content; none for Regenerate, which keeps the
    /// message as it is and makes a new version of its answer.
    pub content: Option<Vec<EditContent>>,
    pub digest: String,
}

/// Why an edit was rejected. Nothing of the history changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("The edit operation ID was used for a different request")]
    Conflict,
    #[error("Message editing requires a settled session with no pending work")]
    Busy,
    #[error("The conversation changed; reopen the message to edit it")]
    Stale,
    #[error(transparent)]
    Target(#[from] CutError),
    #[error("The edited message holds no attachment {0}")]
    UnknownAttachment(String),
    #[error("The edited message holds no {kind} {blob}")]
    UnknownMedia { kind: &'static str, blob: BlobRef },
    #[error("The edit was stopped before it was accepted")]
    Stopped,
    #[error("The agent session is closed")]
    Closed,
    /// Preparing or saving the edit failed, or the node refused it.
    #[error("{}", .0.message)]
    Failed(Box<ErrorReport>),
}

impl EditError {
    /// The refusal as the page words it.
    pub fn refusal(&self) -> Refusal {
        let code = match self {
            Self::Conflict => AgentCode::EditConflict,
            Self::Busy => AgentCode::EditBusy,
            Self::Stale => AgentCode::EditStale,
            Self::Target(_) => AgentCode::EditTarget,
            Self::UnknownAttachment(_) | Self::UnknownMedia { .. } => AgentCode::EditFileMissing,
            Self::Stopped => AgentCode::EditStopped,
            Self::Closed => AgentCode::SessionClosed,
            Self::Failed(report) => {
                return Refusal {
                    code: report
                        .code
                        .clone()
                        .unwrap_or_else(|| AgentCode::FrameFailed.to_string()),
                    params: Default::default(),
                    reason: report.message.clone(),
                };
            }
        };
        Refusal::new(code, self.to_string())
    }
}

/// What the session knows of an edit's operation before the edit runs.
pub enum EditCheck {
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
pub async fn accepted(mut acceptance: Acceptance) -> Result<EditReceipt, EditError> {
    match acceptance.wait_for(Option::is_some).await {
        Ok(result) => result.clone().expect("the acceptance was set"),
        // The session went away before it decided.
        Err(_) => Err(EditError::Closed),
    }
}

/// The SHA-256 of an edit request's RFC 8785 canonical JSON, as the web app
/// sent it, before its uploads are resolved.
pub fn edit_digest(request: &EditRequest) -> String {
    canonical_digest(request).expect("an edit request serializes")
}

/// The digest of a Regenerate request, as an edit's is made.
pub fn regenerate_digest(request: &RegenerateRequest) -> String {
    canonical_digest(request).expect("a regenerate request serializes")
}

/// The edit action.
///
/// It prepares the replacement turn beside the accepted history, commits it
/// with its receipt in one save, adopts it, closes the runtimes it discarded,
/// and runs the replacement's turn on a runtime fork. Until the save commits,
/// nothing of the history changes.
pub(super) async fn run(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    // Held until the edit is accepted or rejected: the replacement's turn
    // may start and close children again.
    let reservation = cancel
        .guard(s.runtime.reserve_edit())
        .await?
        .map_err(|message| TurnError::refused(AgentCode::FrameFailed, message))?;
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
    // Regenerate keeps the message, with the preamble it was sent with.
    let preamble = match submission.content {
        Some(_) => cancel.guard(s.runtime.preamble()).await?,
        None => None,
    };
    // The save order is held from here to the adoption: a message before
    // the window is edited with the blocks from it on, which are read from
    // the store and stay as they are meanwhile.
    let order = s.persist_gate.acquire().await;
    if let Some(position) = window::position(s, &submission.target).await? {
        window::reach(s, position).await?;
    }
    let Candidate {
        blocks,
        mut runtime,
        update,
        receipt,
        model,
    } = s
        .read(|core| Candidate::prepare(core, s, &submission, preamble))
        .map_err(|error| {
            let refusal = error.refusal();
            TurnError::Failed(Box::new(ErrorReport {
                message: refusal.reason,
                code: Some(refusal.code),
                diagnostics: None,
            }))
        })?;
    let saved = match cancel.check() {
        // A save that started always finishes: its outcome decides.
        Ok(()) => s.store.save(update).await.map_err(TurnError::from),
        Err(stopped) => Err(stopped),
    };
    if let Err(error) = saved {
        drop(order);
        // The history is as it was, and the candidate's runtime served
        // nothing.
        runtime.close().await;
        return Err(error);
    }
    // Nothing awaits between the commit and the adoption.
    let discarded = s.update(|core| core.adopt_edit(blocks, runtime, model, receipt));
    drop(order);
    // The callers waiting for the acceptance answer before the replacement's
    // turn writes anything, so an `edit_result` precedes its turn's frames.
    tokio::task::yield_now().await;
    for mut runtime in discarded {
        runtime.close().await;
    }
    drop(reservation);
    cancel.check()?;
    compaction::preflight(s, cancel).await?;
    turn::run_replacement(s, cancel).await
}

/// An edit's replacement history, prepared beside the accepted one.
struct Candidate {
    blocks: Vec<Block>,
    runtime: Box<dyn ProviderRuntime>,
    update: CheckpointUpdate,
    receipt: EditReceipt,
    model: demi_shared_types::ModelSelection,
}

impl Candidate {
    fn prepare(
        core: &SessionCore,
        s: &SessionShared,
        submission: &EditSubmission,
        preamble: Option<String>,
    ) -> Result<Self, EditError> {
        // The window holds the message and every block after it.
        let blocks = core.transcript.blocks();
        let prefix = before_user(blocks, &submission.target)?.to_vec();
        let Some(Block::User(target)) = blocks.get(prefix.len()) else {
            unreachable!("the prefix ends before the user block");
        };
        let model = core.landing_selection().clone();
        let turn = core.turn();
        let blocks = match &submission.content {
            Some(content) => {
                let content = Self::content(target, content)?;
                let mut candidate = TranscriptLog::new(prefix, s.ids.clone(), core.clock());
                // A new version is the user's own, whoever sent the first, and keeps
                // the comments sent with the first.
                candidate.push_user(turn.clone(), &model, content, target.comments.clone(), preamble, None);
                candidate.blocks().to_vec()
            }
            // The message stays, and the new answer follows it.
            None => blocks[..=prefix.len()].to_vec(),
        };
        let runtime = core
            .switch
            .as_ref()
            .and_then(|switch| switch.runtime.as_deref())
            .or(core.provider.held())
            .expect("an idle session's runtime is in its slot")
            .fresh();
        let receipt = EditReceipt {
            operation_id: submission.operation_id.clone(),
            digest: submission.digest.clone(),
            turn_id: turn,
        };
        let mut edits = core.edits.clone();
        edits.push(receipt.clone());
        let update = core.rewrite_update(&blocks, true);
        let update = CheckpointUpdate {
            state: CheckpointState {
                phase: demi_shared_types::SessionPhase::Running,
                queue: Vec::new(),
                edits,
                model: model.clone(),
                ..update.state
            },
            ..update
        };
        Ok(Self {
            blocks,
            runtime,
            update,
            receipt,
            model,
        })
    }

    /// The new version's content: each reference to a file the edited
    /// message `target` holds in that block's place.
    fn content(target: &UserBlock, content: &[EditContent]) -> Result<Vec<UserContentBlock>, EditError> {
        content
            .iter()
            .map(|part| match part {
                EditContent::Content(block) => Ok(block.clone()),
                EditContent::KeptAttachment(id) => target
                    .content
                    .iter()
                    .find(|kept| matches!(kept, UserContentBlock::Attachment(record) if &record.id == id))
                    .cloned()
                    .ok_or_else(|| EditError::UnknownAttachment(id.clone())),
                EditContent::KeptMedia(media) => {
                    let (kind, blob) = named(media);
                    referenced_media(&target.content)
                        .find(|(its_kind, its_blob, _)| *its_kind == kind && *its_blob == blob)
                        .map(|(_, _, block)| block.clone())
                        .ok_or_else(|| EditError::UnknownMedia {
                            kind,
                            blob: blob.clone(),
                        })
                }
            })
            .collect()
    }
}

/// A switch of versions, admitted as an edit is.
///
/// In the session's save order, the store makes the latest leaf of the `to`th
/// version at the block `target`'s place current. The session adopts that path
/// from there as one rewrite batch, on fresh runtimes, so nothing a provider
/// kept of the path it left reaches the next request. Nothing else changes.
pub(super) async fn switch(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    target: &BlockId,
    to: u32,
) -> Result<(), TurnError> {
    let reservation = cancel
        .guard(s.runtime.reserve_edit())
        .await?
        .map_err(|message| TurnError::refused(AgentCode::FrameFailed, message))?;
    cancel.check()?;
    persist::flush(s).await?;
    let order = s.persist_gate.acquire().await;
    let position = window::position(s, target)
        .await?
        .ok_or_else(|| TurnError::refused(AgentCode::EditTarget, "The conversation holds no such block"))?;
    window::reach(s, position).await?;
    // The place whose version changed may be before the window, where a
    // compaction carried the block from: the switch's own transaction reads
    // the path before it, as it was, so nothing after its commit can fail.
    let start = s.read(|core| core.transcript.start());
    let mut switched = s.store.switch(position, to, start).await?;
    let before = switched
        .before
        .take()
        .map(|before| (before.start, before.rows, Earlier::of(before.earlier)));
    let discarded = s.update(|core| core.adopt_switch(switched, before));
    drop(order);
    for mut runtime in discarded {
        runtime.close().await;
    }
    drop(reservation);
    Ok(())
}

/// The kind of block a kept medium names, and the blob it names.
fn named(media: &MediaRef) -> (&'static str, &BlobRef) {
    match media {
        MediaRef::Image { r#ref, .. } => ("image", r#ref),
        MediaRef::Video { r#ref, .. } => ("video", r#ref),
        MediaRef::Document { r#ref, .. } => ("document", r#ref),
    }
}

/// Each native media block of `content` by its kind and the blob it
/// references.
fn referenced_media(
    content: &[UserContentBlock],
) -> impl Iterator<Item = (&'static str, &BlobRef, &UserContentBlock)> {
    content.iter().filter_map(|block| {
        let (kind, blob) = match block {
            UserContentBlock::Image {
                source: MediaSource::Ref { r#ref, .. },
            } => ("image", r#ref),
            UserContentBlock::Video {
                source: MediaSource::Ref { r#ref, .. },
            } => ("video", r#ref),
            UserContentBlock::Document {
                source: DocumentSource::Ref { r#ref, .. },
            } => ("document", r#ref),
            _ => return None,
        };
        Some((kind, blob, block))
    })
}

/// Why a Fork cannot start where it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForkError {
    #[error(transparent)]
    Target(#[from] CutError),
    #[error("The Fork source must be a root session")]
    NotRoot,
    #[error("No matching Fork source checkpoint")]
    NoCheckpoint,
    #[error("A Fork must start idle, without queued actions or edit receipts")]
    InvalidSeed,
    #[error("{0}")]
    Store(String),
}

/// A Fork's seed: the history through the completed text a [`ForkEnd`] names,
/// idle, with nothing waiting.
///
/// It seeds a new root, or a forked child or side question
/// (`crates/agent-server/src/server/tree/supervisor.rs`). The blocks the
/// source's store holds before its window are no part of it: `prefix` names
/// them as rows, which the destination's store copies in the database
/// ([`demi_agent_store::AgentTreeStore::create_node_after`]), and the
/// checkpoint holds the blocks after them, from its `start`.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkSeed {
    pub prefix: Option<StoredPrefix>,
    pub checkpoint: Checkpoint,
}

impl ForkSeed {
    /// Creates the node `record` in `store` with the seed as its first
    /// checkpoint, in one commit.
    ///
    /// The prefix's rows are copied from the tree of the conversation root
    /// `source`, or from `store`'s own with none, then the seed's blocks.
    pub async fn create(
        &self,
        store: &dyn AgentTreeStore,
        record: NodeRecord,
        source: Option<&NodeId>,
    ) -> Result<(), StoreError> {
        let checkpoint = &self.checkpoint;
        let initial = CheckpointUpdate {
            state: checkpoint.state.clone(),
            changed_blocks: (checkpoint.start..).zip(checkpoint.transcript.iter().cloned()).collect(),
            block_count: checkpoint.block_count(),
            moved_from: None,
            version: false,
        };
        match &self.prefix {
            Some(prefix) => store.create_node_after(record, source, prefix, initial).await,
            None => store.create_node(record, initial).await,
        }
    }
}

/// The seed of a source whose path holds `window` from `start` on, the blocks
/// before it in `store`, with the checkpoint state `state`.
///
/// What a Fork through `end` keeps is checked against the window, and against
/// the store for the blocks before it, which it names without reading them.
pub async fn fork_seed(
    store: &dyn SessionStore,
    start: usize,
    window: &[Block],
    state: CheckpointState,
    end: ForkEnd<'_>,
) -> Result<ForkSeed, ForkError> {
    let state = CheckpointState {
        phase: demi_shared_types::SessionPhase::Idle,
        queue: Vec::new(),
        agent_inputs: Vec::new(),
        reports: Vec::new(),
        next_turn_reports: Vec::new(),
        intervals: Vec::new(),
        edits: Vec::new(),
        ..state
    };
    let seed = |length: usize, transcript: Vec<Block>, prefix: Option<StoredPrefix>| ForkSeed {
        prefix,
        checkpoint: Checkpoint {
            start: length,
            versions: Default::default(),
            state: state.clone(),
            transcript,
        },
    };
    if start == 0 {
        return Ok(seed(0, fork_prefix(window, end)?.to_vec(), None));
    }
    let store_error = |error: StoreError| ForkError::Store(error.to_string());
    // No call before the window may still execute, whatever the Fork keeps
    // of the window.
    let unfinished_before = |before: usize| async move {
        Ok::<_, ForkError>(store.last_of(Kind::ExecutingCall, before).await.map_err(store_error)?.is_some())
    };
    let named = match end {
        ForkEnd::At(target) | ForkEnd::Side(target) => Some(target),
        ForkEnd::Newest => None,
    };
    let in_window = named.is_none_or(|target| window.iter().any(|block| block.id() == target));
    let (length, tail) = if in_window {
        match fork_prefix(window, end) {
            Ok(kept) => (start, kept.to_vec()),
            // No text of the window ends a Fork with no call before it
            // executing: the newest such text is before the window.
            Err(CutError::NothingCompleted) => {
                let at = store
                    .last_of(Kind::CompletedText, start)
                    .await
                    .map_err(store_error)?
                    .ok_or(CutError::NothingCompleted)?;
                (at + 1, Vec::new())
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        let target = named.expect("a Fork end the window lacks names a block");
        let at = store.position(target).await.map_err(store_error)?.ok_or(CutError::NotCompletedText)?;
        let read = store.blocks(at..at + 1).await.map_err(store_error)?;
        if !read.blocks.first().is_some_and(|block| Kind::CompletedText.holds(block)) {
            return Err(CutError::NotCompletedText.into());
        }
        match end {
            // The side question's history goes on through the window, which
            // starts with the compaction that summarized its text.
            ForkEnd::Side(_) => (start, window[..=side_end(window, 0)].to_vec()),
            _ => (at + 1, Vec::new()),
        }
    };
    // The rows named end at the window or before it, at the text.
    if unfinished_before(length).await? || tail.iter().any(|block| Kind::ExecutingCall.holds(block)) {
        return Err(CutError::UnfinishedToolCalls.into());
    }
    let prefix = store.prefix(length).await.map_err(store_error)?;
    Ok(seed(length, tail, prefix))
}
