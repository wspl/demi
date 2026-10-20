//! Saving a session's checkpoint: only what changed, one save at a time.
//!
//! A save runs one second after the first unsaved change, or at once where the
//! session must not lose it. A save that started always finishes, and a failed
//! one leaves its rows to the next.
//!
//! Saves, history rewrites and edit commits run in the order they were
//! requested. A running turn never holds that order while it waits for a
//! provider, a tool or a context source, so its scheduled saves commit
//! meanwhile.
//!
//! A save is due when the transcript, the edit receipts, the queue, the waiting
//! agent messages or the model selection changed. The phase alone never makes
//! one due.
//!
//! A save is made at once:
//!
//! - before tools are dispatched;
//! - after a `context` block;
//! - when an action ends;
//! - when an agent message is admitted or written;
//! - on a history rewrite or an edit commit;
//! - on dispose.
//!
//! The save at the end of an action records the phase idle, so a checkpoint
//! that says an action was running is one the process died in.
//!
//! Failed saves:
//!
//! - A scheduled save that fails is reported as an `error` event and its rows
//!   go with the next change.
//! - The failed save at an action's end fails the action.
//!
//! A history rewrite is saved before it is published. A save stores no blob:
//! media are references whose blobs were stored when the medium entered.

use std::{
    rc::{Rc, Weak},
    time::Duration,
};

use demi_agent_store::StoreError;
use demi_agent_transcript::DirtyRows;
use demi_shared_types::{Block, SessionPhase};
use tokio::sync::Notify;

use super::{SessionEvent, SessionShared};

/// Whether a save is due and which rows it writes.
#[derive(Debug)]
pub(crate) struct PersistMarks {
    pub(crate) dirty: bool,
    pub(crate) rows: DirtyRows,
    /// The phase the last save the session prepared records.
    pub(crate) recorded: SessionPhase,
}

impl Default for PersistMarks {
    fn default() -> Self {
        Self {
            dirty: false,
            rows: DirtyRows::default(),
            recorded: SessionPhase::Idle,
        }
    }
}

/// Writes the checkpoint when a save is due. The caller holds the session's
/// save order. Once every row is saved, the blocks before the last
/// compaction boundary are let go.
pub(crate) async fn write_if_dirty(s: &SessionShared) -> Result<(), StoreError> {
    let Some((update, rows)) = s.update(|core| core.prepare_checkpoint()) else {
        s.update(|core| core.release_settled());
        return Ok(());
    };
    let saved = s.store.save(update).await;
    match &saved {
        Ok(()) => s.update(|core| core.release_settled()),
        Err(_) => s.update(|core| core.restore_marks(rows)),
    }
    saved
}

/// Saves at once, after the saves ahead in the session's order.
pub(crate) async fn flush(s: &SessionShared) -> Result<(), StoreError> {
    let _turn = s.persist_gate.acquire().await;
    write_if_dirty(s).await
}

/// Commits a history rewrite of the session's own, a retry, an unwind or a
/// dropped attempt.
///
/// In the session's save order, the store commits the new branch, which stands
/// in for what it replaces and leaves it stored, and only then does the session
/// adopt it and publish one `replace` patch. A failed save changes nothing.
pub(crate) async fn commit_rewrite(
    s: &SessionShared,
    blocks: Vec<Block>,
) -> Result<(), StoreError> {
    let _turn = s.persist_gate.acquire().await;
    commit_rewrite_held(s, blocks).await
}

/// [`commit_rewrite`] of `blocks`, the path from the window's start on,
/// for a caller that holds the session's save order.
pub(crate) async fn commit_rewrite_held(
    s: &SessionShared,
    blocks: Vec<Block>,
) -> Result<(), StoreError> {
    let update = s.read(|core| core.rewrite_update(&blocks, false));
    s.store.save(update).await?;
    s.update(|core| core.adopt_rewrite(blocks, false));
    Ok(())
}

/// The persister: one second after the first unsaved change, it saves. A
/// failed save is reported, and its rows wait for the next change.
pub(crate) async fn run(session: Weak<SessionShared>, wake: Rc<Notify>, interval: Duration) {
    loop {
        wake.notified().await;
        tokio::time::sleep(interval).await;
        let Some(s) = session.upgrade() else {
            return;
        };
        if let Err(error) = flush(&s).await {
            s.emit(SessionEvent::Error {
                report: error.into(),
            });
        }
    }
}
