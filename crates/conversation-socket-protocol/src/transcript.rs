//! How a transcript travels (`runtime.md` § Patches and versions): as
//! patches, each naming the index of the block it touches, and a version.

use std::ops::Range;

use demi_shared_types::{Block, BlockId, MAX_SAFE_INTEGER, ToolResultContentBlock, ToolView};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One change to a transcript. A batch of patches advances the revision by
/// one; a rewrite of history is a `truncate` at the first block it changes
/// and an `add` for each block from there. A block travels without its
/// entries. `conversation-client`'s one patch applier applies them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TranscriptPatch {
    /// Insert a block at the index.
    Add {
        #[garde(skip)]
        index: u32,
        #[serde(with = "demi_shared_types::client_block")]
        #[schemars(with = "Block")]
        #[garde(dive)]
        value: Block,
    },
    /// Replace the block at the index.
    ReplaceBlock {
        #[garde(skip)]
        index: u32,
        #[serde(with = "demi_shared_types::client_block")]
        #[schemars(with = "Block")]
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
    /// Remove every block from the index `length` on.
    Truncate {
        #[garde(skip)]
        length: u32,
    },
}

/// A transcript's version. The epoch is new each time the session's
/// transcript is built, at creation and at every restore, so a version taken
/// before a backend restart never matches after it; the revision counts the
/// patch batches since.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptVersion {
    #[garde(length(chars, min = 1))]
    pub epoch: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
}

/// How many bytes of JSON a page of a transcript holds past its first
/// request (`web-api.md` § Pages).
pub const PAGE_BYTES: usize = 64 * 1024;

/// How long a call's input a light form keeps whole: a short script, such
/// as one that only looks at a command, says on the folded row what the
/// call did (`runtime.md` § Work groups).
pub const LIGHT_INPUT_BYTES: usize = 512;

/// `block` in its light form (`web-api.md` § Light form): without what only
/// its open row shows, nor the entries of the vendor's record, which never
/// leave the backend. A tool call longer than [`LIGHT_INPUT_BYTES`] keeps of
/// its input only its `description`, so its row is titled as the call whole.
pub fn light(block: &Block) -> Block {
    let mut light = block.clone();
    if let Some(entries) = light.entries_mut() {
        entries.clear();
    }
    match &mut light {
        Block::ToolCall(call) => {
            if call.input.len() > LIGHT_INPUT_BYTES {
                let description = serde_json::from_str::<serde_json::Value>(&call.input)
                    .ok()
                    .and_then(|input| input.get("description")?.as_str().map(str::to_owned));
                call.input = match description {
                    Some(description) => serde_json::json!({ "description": description }).to_string(),
                    None => "{}".to_owned(),
                };
            }
            call.output
                .retain(|part| !matches!(part, ToolResultContentBlock::Text { .. }));
            if let Some(ToolView::Shell(view)) = &mut call.view {
                view.chunks.clear();
            }
        }
        Block::Thinking(thinking) => {
            thinking.text.clear();
            thinking.signature = None;
        }
        Block::RedactedThinking(redacted) => redacted.data.clear(),
        Block::AgentMessage(message) => message.message.content.clear(),
        Block::Wakeup(wakeup) => {
            for report in &mut wakeup.reports {
                report.output.clear();
            }
        }
        Block::Context(context) => {
            context.text.clear();
            context.instructions.clear();
        }
        Block::CompactionBoundary(boundary) => boundary.summary.clear(),
        Block::User(_)
        | Block::Steer(_)
        | Block::Resume(_)
        | Block::Abort(_)
        | Block::Text(_)
        | Block::Response(_)
        | Block::Error(_)
        | Block::CompactionMarker(_) => {}
    }
    light
}

/// The size of `block`'s light form as a page sends it.
pub fn light_bytes(block: &Block) -> usize {
    serde_json::to_vec(&light(block)).map_or(0, |json| json.len())
}

/// The requests of a transcript of `length` blocks whose `user` blocks are
/// at `users`, in order: each from a `user` block to the next, the blocks
/// before the first belonging to the first request (`web-api.md` § Pages).
pub fn requests(length: usize, users: &[usize]) -> Vec<Range<usize>> {
    if length == 0 {
        return Vec::new();
    }
    // The first request starts at 0, its `user` block wherever it is.
    let starts: Vec<usize> = std::iter::once(0)
        .chain(users.iter().copied().skip(1).filter(|&index| index < length))
        .collect();
    let ends = starts.iter().skip(1).copied().chain(std::iter::once(length));
    starts.iter().copied().zip(ends).map(|(start, end)| start..end).collect()
}

/// Where a page is taken from (`web-api.md` § Pages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageAt {
    /// The latest page.
    Latest,
    /// The page that ends just before the block at this index.
    Before(usize),
    /// The page that starts just after the block at this index.
    After(usize),
    /// The page that holds the block at this index.
    Around(usize),
}

/// The blocks of the page `at` among `requests`, as a range of indices:
/// whole requests from the one `at` names, adding requests while the page
/// stays within [`PAGE_BYTES`], `size` giving a request's bytes; at least
/// one request. `Around` adds the request after, then the one before, in
/// turn. An index past the requests' ends answers an empty page there.
pub fn page(
    requests: &[Range<usize>],
    at: PageAt,
    mut size: impl FnMut(Range<usize>) -> usize,
) -> Range<usize> {
    let Some(last) = requests.last() else {
        return 0..0;
    };
    // The request that holds the block `index`.
    let holding = |index: usize| requests.partition_point(|request| request.end <= index);
    let (first, toward_start, toward_end) = match at {
        PageAt::Latest => (requests.len() - 1, true, false),
        PageAt::Before(0) => return 0..0,
        PageAt::Before(index) => (holding(index - 1).min(requests.len() - 1), true, false),
        PageAt::After(index) if index + 1 >= last.end => return last.end..last.end,
        PageAt::After(index) => (holding(index + 1), false, true),
        PageAt::Around(index) => (holding(index).min(requests.len() - 1), true, true),
    };
    let (mut low, mut high) = (first, first);
    let mut bytes = size(requests[first].clone());
    let (mut down, mut up) = (toward_start, toward_end);
    // Around a block the page grows after it first, then before.
    let mut next_up = toward_end;
    while down || up {
        let grow_up = up && (next_up || !down);
        next_up = !grow_up;
        let candidate = if grow_up {
            (high + 1 < requests.len()).then_some(high + 1)
        } else {
            low.checked_sub(1)
        };
        let Some(candidate) = candidate else {
            if grow_up {
                up = false;
            } else {
                down = false;
            }
            continue;
        };
        let more = size(requests[candidate].clone());
        if bytes + more > PAGE_BYTES {
            break;
        }
        bytes += more;
        if grow_up {
            high = candidate;
        } else {
            low = candidate;
        }
    }
    requests[low].start..requests[high].end
}

/// The start of a transcript's latest page, for a reset the page named no
/// blocks for (`runtime.md` § Where a reset starts).
pub fn latest_page_start(blocks: &[Block]) -> usize {
    let users: Vec<usize> = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| matches!(block, Block::User(_)).then_some(index))
        .collect();
    let requests = requests(blocks.len(), &users);
    page(&requests, PageAt::Latest, |range| {
        blocks[range].iter().map(light_bytes).sum()
    })
    .start
}

/// Where a reset of `blocks` starts for a page that holds them from `from`,
/// whose block there it holds as `edge` (`runtime.md` § Where a reset
/// starts): at `from` while that block is still `edge`, and otherwise, or
/// when the page names none or only one of them, at the start of the latest
/// page.
pub fn reset_start(blocks: &[Block], from: Option<u32>, edge: Option<&BlockId>) -> usize {
    let held = from.and_then(|from| usize::try_from(from).ok());
    match (held, edge) {
        (Some(from), Some(edge)) if blocks.get(from).is_some_and(|block| block.id() == edge) => from,
        _ => latest_page_start(blocks),
    }
}

/// An index of a transcript as a frame or a page names it.
pub fn index_u32(index: usize) -> u32 {
    // A transcript lives in memory, which holds far fewer than 2^32 blocks.
    u32::try_from(index).expect("a transcript holds fewer than 2^32 blocks")
}
