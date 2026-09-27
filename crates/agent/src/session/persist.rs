//! Saving a session's checkpoint (`runtime.md` § Saving): only what changed,
//! one save at a time, one second after the first unsaved change or at once
//! where the design says so. A save that started always finishes, and a
//! failed one leaves its rows to the next.

use std::{
    rc::{Rc, Weak},
    time::Duration,
};

use demi_core::Block;
use tokio::sync::Notify;

use super::{SessionEvent, SessionShared};
use crate::{
    store::{CommandStateHistory, CommandVersion, CommitGuard, StoreError},
    transcript::DirtyRows,
};

/// Whether a save is due and which rows it writes.
#[derive(Debug, Default)]
pub(crate) struct PersistMarks {
    pub(crate) dirty: bool,
    pub(crate) rows: DirtyRows,
}

/// What a save took from the marks, for putting back when it fails.
pub(crate) struct TakenMarks {
    pub(crate) rows: DirtyRows,
    pub(crate) command_state: bool,
}

/// Writes the checkpoint when a save is due, or when it carries `pending`,
/// a command-storage version that becomes current once the save commits.
/// The caller holds the session's save order.
pub(crate) async fn write_if_dirty(
    s: &SessionShared,
    pending: Option<&CommandVersion>,
    guard: &CommitGuard,
) -> Result<(), StoreError> {
    let Some((update, taken)) = s.update(|core| core.prepare_checkpoint(pending)) else {
        return Ok(());
    };
    let saved = s.store.save(update, guard).await;
    if saved.is_err() {
        s.update(|core| core.restore_marks(taken));
    }
    saved
}

/// Saves at once, after the saves ahead in the session's order.
pub(crate) async fn flush(s: &SessionShared) -> Result<(), StoreError> {
    let _turn = s.persist_gate.acquire().await;
    write_if_dirty(s, None, &CommitGuard::default()).await
}

/// Commits a history rewrite (`runtime.md` § Saving): in the session's save
/// order, the store commits every retained row with the command state of the
/// cut, `revision` current, and only then does the session adopt them and
/// publish one `replace` patch. A failed save changes nothing.
pub(crate) async fn commit_rewrite(
    s: &SessionShared,
    blocks: Vec<Block>,
    revision: u64,
) -> Result<(), StoreError> {
    let _turn = s.persist_gate.acquire().await;
    let (update, commands) = s.read(|core| {
        let commands = core.commands.select(&blocks, revision, false);
        (core.rewrite_update(&blocks, commands.clone()), commands)
    });
    s.store.save(update, &CommitGuard::default()).await?;
    let commands =
        CommandStateHistory::restore(commands).expect("a cut of a valid command state is valid");
    s.update(|core| core.adopt_rewrite(blocks, commands));
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
