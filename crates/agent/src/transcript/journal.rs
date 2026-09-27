//! The record of what changed in a transcript since it was last committed
//! (`runtime.md` § Patches and versions): each change as the patch a client
//! applies, with the block's value as it was when the change was made, and
//! the rows it changed, which the next save writes (§ Saving). A history
//! rewrite is not recorded here: it is saved before it is published, and the
//! transcript publishes its `replace` patch itself.

use std::collections::BTreeSet;

use demi_agent_protocol::TranscriptPatch;
use demi_core::{Block, BlockId};

/// The patches of one commit, which advances the revision by one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PatchBatch {
    pub(crate) revision: u64,
    pub(crate) patches: Vec<TranscriptPatch>,
    /// The blocks the patches added or changed, in the order they were
    /// changed: the session records their command-state boundaries by id,
    /// since a later patch of the batch can move a block's index.
    pub(crate) touched: Vec<BlockId>,
    /// The rows the patches moved or changed.
    pub(crate) rows: DirtyRows,
}

/// Which transcript rows the next save writes: every row from `floor` on,
/// because an insertion there moved them, and the rows below it that changed
/// in place.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DirtyRows {
    floor: Option<usize>,
    points: BTreeSet<usize>,
}

impl DirtyRows {
    /// Every row from `index` on moved: a block was inserted there.
    fn move_from(&mut self, index: usize) {
        let floor = self.floor.map_or(index, |floor| floor.min(index));
        self.floor = Some(floor);
        self.points.retain(|point| *point < floor);
    }

    /// The row at `index` changed in place.
    fn change(&mut self, index: usize) {
        if self.floor.is_none_or(|floor| index < floor) {
            self.points.insert(index);
        }
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

    /// Adds `other`'s rows: a later commit's, or those a failed save did not
    /// write.
    pub(crate) fn merge(&mut self, other: DirtyRows) {
        if let Some(floor) = other.floor {
            self.move_from(floor);
        }
        for index in other.points {
            self.change(index);
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Journal {
    patches: Vec<TranscriptPatch>,
    touched: Vec<BlockId>,
    rows: DirtyRows,
}

impl Journal {
    pub(super) fn add(&mut self, index: usize, block: &Block) {
        self.touch(block.id());
        self.rows.move_from(index);
        self.patches.push(TranscriptPatch::Add {
            index: patch_index(index),
            value: block.clone(),
        });
    }

    pub(super) fn replace_block(&mut self, index: usize, block: &Block) {
        self.touch(block.id());
        self.rows.change(index);
        self.patches.push(TranscriptPatch::ReplaceBlock {
            index: patch_index(index),
            value: block.clone(),
        });
    }

    /// Consecutive appends to one block merge into one patch.
    pub(super) fn append_text(&mut self, index: usize, id: &BlockId, delta: &str) {
        self.touch(id);
        self.rows.change(index);
        let index = patch_index(index);
        if let Some(TranscriptPatch::AppendText {
            index: last,
            delta: merged,
        }) = self.patches.last_mut()
            && *last == index
        {
            merged.push_str(delta);
            return;
        }
        self.patches.push(TranscriptPatch::AppendText {
            index,
            delta: delta.to_owned(),
        });
    }

    /// The changes recorded since the last call, as the batch of `revision`;
    /// none when nothing changed.
    pub(super) fn take(&mut self, revision: u64) -> Option<PatchBatch> {
        if self.patches.is_empty() {
            return None;
        }
        let Journal {
            patches,
            touched,
            rows,
        } = std::mem::take(self);
        Some(PatchBatch {
            revision,
            patches,
            touched,
            rows,
        })
    }

    fn touch(&mut self, id: &BlockId) {
        if self.touched.last() != Some(id) {
            self.touched.push(id.clone());
        }
    }
}

/// A block's index as a patch names it.
fn patch_index(index: usize) -> u32 {
    // A transcript lives in memory, which holds far fewer than 2^32 blocks.
    u32::try_from(index).expect("a transcript holds fewer than 2^32 blocks")
}
