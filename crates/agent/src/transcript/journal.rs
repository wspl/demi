//! The record of what changed in a transcript since it was last committed
//! (`runtime.md` § Patches and versions): each change as the patch a client
//! applies, with the block's value as it was when the change was made.

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
}

#[derive(Debug, Default)]
pub(super) struct Journal {
    patches: Vec<TranscriptPatch>,
    touched: Vec<BlockId>,
}

impl Journal {
    pub(super) fn add(&mut self, index: usize, block: &Block) {
        self.touch(block.id());
        self.patches.push(TranscriptPatch::Add {
            index: patch_index(index),
            value: block.clone(),
        });
    }

    pub(super) fn replace_block(&mut self, index: usize, block: &Block) {
        self.touch(block.id());
        self.patches.push(TranscriptPatch::ReplaceBlock {
            index: patch_index(index),
            value: block.clone(),
        });
    }

    /// Consecutive appends to one block merge into one patch.
    pub(super) fn append_text(&mut self, index: usize, id: &BlockId, delta: &str) {
        self.touch(id);
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

    pub(super) fn take(&mut self) -> Option<(Vec<TranscriptPatch>, Vec<BlockId>)> {
        if self.patches.is_empty() {
            return None;
        }
        Some((
            std::mem::take(&mut self.patches),
            std::mem::take(&mut self.touched),
        ))
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
