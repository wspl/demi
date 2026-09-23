//! How a transcript travels (`runtime.md` § Patches and versions): as
//! patches, each naming the index of the block it touches, and a version.

use demi_core::{Block, MAX_SAFE_INTEGER};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One change to a transcript. A batch of patches advances the revision by
/// one; a rewrite of history is one `replace`. `agent-client`'s one patch
/// applier applies them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TranscriptPatch {
    /// Insert a block at the index.
    Add {
        #[garde(skip)]
        index: u32,
        #[garde(dive)]
        value: Block,
    },
    /// Remove the block at the index.
    Remove {
        #[garde(skip)]
        index: u32,
    },
    /// Replace the block at the index.
    ReplaceBlock {
        #[garde(skip)]
        index: u32,
        #[garde(dive)]
        value: Block,
    },
    /// Append text to the text or thinking block at the index. Consecutive
    /// appends to one block merge into one.
    AppendText {
        #[garde(skip)]
        index: u32,
        #[garde(skip)]
        delta: String,
    },
    /// Replace every block.
    Replace {
        #[garde(dive)]
        value: Vec<Block>,
    },
}

/// A transcript's version. The epoch is new each time the session's
/// transcript is built, at creation and at every restore, so a version taken
/// before a backend restart never matches after it; the revision counts the
/// patch batches since.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptVersion {
    #[garde(length(utf16, min = 1))]
    pub epoch: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
}
