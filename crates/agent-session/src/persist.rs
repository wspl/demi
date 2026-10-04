//! Saving a session's checkpoint (`runtime.md` § Saving): only what changed,
//! one save at a time, one second after the first unsaved change or at once
//! where the design says so. A save that started always finishes, and a
//! failed one leaves its rows to the next.

use std::{
    rc::{Rc, Weak},
    time::Duration,
};

use demi_agent_store::StoreError;
use demi_agent_transcript::DirtyRows;
use demi_shared_types::Block;
use tokio::sync::Notify;

use super::{SessionEvent, SessionShared};

/// Whether a save is due and which rows it writes.
#[derive(Debug, Default)]
pub(crate) struct PersistMarks {
    pub(crate) dirty: bool,
    pub(crate) rows: DirtyRows,
}

/// Writes the checkpoint when a save is due. The caller holds the session's
/// save order.
pub(crate) async fn write_if_dirty(s: &SessionShared) -> Result<(), StoreError> {
    let Some((update, rows)) = s.update(|core| core.prepare_checkpoint()) else {
        return Ok(());
    };
    let saved = s.store.save(update).await;
    if saved.is_err() {
        s.update(|core| core.restore_marks(rows));
    }
    saved
}

/// Saves at once, after the saves ahead in the session's order.
pub(crate) async fn flush(s: &SessionShared) -> Result<(), StoreError> {
    let _turn = s.persist_gate.acquire().await;
    write_if_dirty(s).await
}

/// Commits a history rewrite (`runtime.md` § Saving): in the session's save
/// order, the store commits every retained row, and only then does the
/// session adopt them and publish one `replace` patch. A failed save changes
/// nothing.
pub(crate) async fn commit_rewrite(
    s: &SessionShared,
    blocks: Vec<Block>,
) -> Result<(), StoreError> {
    let _turn = s.persist_gate.acquire().await;
    let update = s.read(|core| core.rewrite_update(&blocks));
    s.store.save(update).await?;
    s.update(|core| core.adopt_rewrite(blocks));
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
