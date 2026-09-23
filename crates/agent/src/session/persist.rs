//! Saving a session's checkpoint (`runtime.md` § Saving): only what changed,
//! one save at a time, one second after the first unsaved change or at once
//! where the design says so. A save that started always finishes, and a
//! failed one leaves its rows to the next.

use std::{
    collections::BTreeSet,
    rc::{Rc, Weak},
    time::Duration,
};

use demi_agent_protocol::TranscriptPatch;
use tokio::sync::Notify;

use super::{SessionEvent, SessionShared};
use crate::store::{CommitGuard, StoreError};

/// Which transcript rows the next save writes: every row from `floor` on,
/// because an insertion or removal there moved them, and the rows below it
/// that changed in place.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DirtyRows {
    floor: Option<usize>,
    points: BTreeSet<usize>,
}

impl DirtyRows {
    pub(crate) fn mark(&mut self, patches: &[TranscriptPatch]) {
        for patch in patches {
            match patch {
                TranscriptPatch::Add { index, .. } | TranscriptPatch::Remove { index } => {
                    self.lower_floor(row(*index));
                }
                TranscriptPatch::Replace { .. } => self.lower_floor(0),
                TranscriptPatch::ReplaceBlock { index, .. }
                | TranscriptPatch::AppendText { index, .. } => {
                    let index = row(*index);
                    if self.floor.is_none_or(|floor| index < floor) {
                        self.points.insert(index);
                    }
                }
            }
        }
    }

    fn lower_floor(&mut self, index: usize) {
        let floor = self.floor.map_or(index, |floor| floor.min(index));
        self.floor = Some(floor);
        self.points.retain(|point| *point < floor);
    }

    /// The rows to write out of `len`, ascending.
    pub(crate) fn indices(&self, len: usize) -> Vec<usize> {
        let floor = self.floor.unwrap_or(len).min(len);
        self.points
            .iter()
            .copied()
            .filter(|index| *index < floor)
            .chain(floor..len)
            .collect()
    }

    /// Puts back the rows a failed save did not write.
    pub(crate) fn merge(&mut self, other: DirtyRows) {
        if let Some(floor) = other.floor {
            self.lower_floor(floor);
        }
        let floor = self.floor;
        self.points.extend(
            other
                .points
                .into_iter()
                .filter(|index| floor.is_none_or(|floor| *index < floor)),
        );
    }
}

fn row(index: u32) -> usize {
    usize::try_from(index).expect("a patch index fits in usize")
}

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

/// Writes the checkpoint when a save is due. The caller holds the session's
/// save order.
pub(crate) async fn write_if_dirty(
    s: &SessionShared,
    guard: &CommitGuard,
) -> Result<(), StoreError> {
    let Some((update, taken)) = s.update(|core| core.prepare_checkpoint()) else {
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
    write_if_dirty(s, &CommitGuard::default()).await
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

#[cfg(test)]
mod tests {
    use demi_core::{Block, TextBlock, Timestamp};

    use super::*;
    use crate::testing::test_model;

    fn text_block() -> Block {
        Block::Text(TextBlock {
            id: "t".try_into().unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            text: String::new(),
            forkable: false,
        })
    }

    #[test]
    fn a_save_writes_the_rows_from_the_lowest_insertion_on_and_the_points_below_it() {
        let mut rows = DirtyRows::default();
        rows.mark(&[
            TranscriptPatch::AppendText {
                index: 1,
                delta: "x".into(),
            },
            TranscriptPatch::Add {
                index: 5,
                value: text_block(),
            },
            TranscriptPatch::ReplaceBlock {
                index: 6,
                value: text_block(),
            },
        ]);
        assert_eq!(rows.indices(7), vec![1, 5, 6]);
        // A failed save's rows come back beside the ones marked since.
        let mut later = DirtyRows::default();
        later.mark(&[TranscriptPatch::ReplaceBlock {
            index: 3,
            value: text_block(),
        }]);
        later.merge(rows);
        assert_eq!(later.indices(7), vec![1, 3, 5, 6]);
        let mut rewritten = DirtyRows::default();
        rewritten.mark(&[TranscriptPatch::Replace { value: Vec::new() }]);
        assert_eq!(rewritten.indices(2), vec![0, 1]);
    }
}
