//! How a transcript travels: as patches, each naming the index of the
//! block it touches, and a version.

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
    /// Insert a block at the index, with its place among the versions there
    /// when it has others.
    Add {
        #[garde(skip)]
        index: u32,
        #[serde(with = "demi_shared_types::client_block")]
        #[schemars(with = "Block")]
        #[garde(dive)]
        value: Block,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(dive)]
        versions: Option<Versions>,
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

/// Where a block stands among the versions at its place on the path: the
/// `index`th of `count`, from 1, in the order they were made. Only a block
/// whose place holds two versions or more has one: an edited message, or
/// the first block of a regenerated answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Versions {
    #[garde(range(min = 1))]
    pub index: u32,
    #[garde(range(min = 2))]
    pub count: u32,
}

/// The versions of one block of a reset or a page, by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockVersions {
    #[garde(skip)]
    pub block_id: BlockId,
    #[garde(range(min = 1))]
    pub index: u32,
    #[garde(range(min = 2))]
    pub count: u32,
}

impl BlockVersions {
    pub fn new(block_id: BlockId, versions: Versions) -> Self {
        Self {
            block_id,
            index: versions.index,
            count: versions.count,
        }
    }

    /// The versions of `blocks`, whose places `versions` holds them by,
    /// from the position `start` on.
    pub fn of(blocks: &[Block], versions: &std::collections::BTreeMap<usize, Versions>, start: usize) -> Vec<Self> {
        versions
            .range(start..)
            .filter_map(|(position, held)| Some(Self::new(blocks.get(*position)?.id().clone(), *held)))
            .collect()
    }
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
/// request.
pub const PAGE_BYTES: usize = 64 * 1024;

/// How long a call's input a light form keeps whole: a short script, such
/// as one that only looks at a command, says on the folded row what the
/// call did.
pub const LIGHT_INPUT_BYTES: usize = 512;

/// Whole requests' `blocks` in their light form: each without what only its open
/// row shows, and of the lists of changed files a shell call carries, only those a
/// page shows: the run's on the latest call of each run, and the request's on the
/// latest call of each request.
pub fn light<'a>(blocks: impl IntoIterator<Item = &'a Block>) -> Vec<Block> {
    let mut light: Vec<Block> = blocks.into_iter().map(light_block).collect();
    // From the end, the first call met with a run's or a request's list is
    // the latest that carries it.
    let mut runs_listed: Vec<BlockId> = Vec::new();
    let mut request_listed = false;
    for block in light.iter_mut().rev() {
        if matches!(block, Block::User(_)) {
            request_listed = false;
            continue;
        }
        let run = block.run().cloned();
        let Block::ToolCall(call) = block else {
            continue;
        };
        let Some(ToolView::Shell(view)) = &mut call.view else {
            continue;
        };
        if view.run_files.is_some() {
            match run {
                Some(run) if !runs_listed.contains(&run) => runs_listed.push(run),
                _ => view.run_files = None,
            }
        }
        if view.request_files.is_some() {
            if request_listed {
                view.request_files = None;
            }
            request_listed = true;
        }
    }
    light
}

/// `block` in its light form: without what only its open row shows, nor
/// the entries of the vendor's record, which never leave the backend. A
/// tool call longer than [`LIGHT_INPUT_BYTES`] keeps of its input only its
/// `description`, so its row is titled as the call whole.
fn light_block(block: &Block) -> Block {
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
            // Without the white space around it, as the page tests a thinking for
            // text.
            thinking.text_length = Some(thinking.text.trim().chars().count() as u64);
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
        | Block::AccountSwitch(_)
        | Block::CompactionMarker(_) => {}
    }
    light
}

/// The size of whole requests' `blocks` in their light form as a page
/// sends them.
pub fn light_bytes<'a>(blocks: impl IntoIterator<Item = &'a Block>) -> usize {
    light(blocks)
        .iter()
        .map(|block| serde_json::to_vec(block).map_or(0, |json| json.len()))
        .sum()
}

/// The fewest bytes `block` takes in a page's light form, whichever of its
/// lists of changed files the page keeps on it ([`light`]): a reader that
/// adds these up over a request's blocks knows the request does not fit a
/// room once they pass it, without reading its later blocks.
pub fn least_light_bytes(block: &Block) -> usize {
    let mut light = light_block(block);
    if let Block::ToolCall(call) = &mut light
        && let Some(ToolView::Shell(view)) = &mut call.view
    {
        view.run_files = None;
        view.request_files = None;
    }
    serde_json::to_vec(&light).map_or(0, |json| json.len())
}

/// The requests of a transcript of `length` blocks whose `user` blocks are
/// at `users`, in order: each from a `user` block to the next, the blocks
/// before the first belonging to the first request.
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

/// Where a page is taken from.
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
/// stays within [`PAGE_BYTES`]; at least one request. `size` gives a
/// request's bytes, or none when they are more than the room it is given,
/// what the page has left, so a reader can stop reading a request that
/// cannot fit; the first request has unbounded room. `Around` adds the
/// request after, then the one before, in turn. An index past the
/// requests' ends answers an empty page there.
pub fn page(
    requests: &[Range<usize>],
    at: PageAt,
    mut size: impl FnMut(Range<usize>, usize) -> Option<usize>,
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
    // The first request is kept whatever its size.
    let mut bytes = size(requests[first].clone(), usize::MAX).unwrap_or(usize::MAX);
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
        let fits = PAGE_BYTES
            .checked_sub(bytes)
            .and_then(|room| size(requests[candidate].clone(), room).filter(|&more| more <= room));
        let Some(more) = fits else {
            break;
        };
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
/// blocks for.
pub fn latest_page_start(blocks: &[Block]) -> usize {
    latest_page_start_within(blocks, true)
}

/// The start of the latest page among `blocks`, the transcript's from
/// its start when `whole`, else a window of it that a session holds,
/// which starts inside the request before its first `user` block: the
/// window's blocks before that block, the end of that request, count as a
/// request of their own, and the page reaches no further back than the
/// window, so that no block before it is read to size the page. For
/// example, a window from a boundary at 23,000 whose first message is at
/// 23,600 starts its latest page at 23,600 when that request fills the
/// page, and never at the earlier request's start.
pub fn latest_page_start_within(blocks: &[Block], whole: bool) -> usize {
    let users: Vec<usize> = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| matches!(block, Block::User(_)).then_some(index))
        .collect();
    let mut requests = requests(blocks.len(), &users);
    if !whole
        && let Some(&first) = users.first()
        && first > 0
        && let Some(leading) = requests.first_mut()
    {
        // The first request starts at its own `user` block, after the
        // window's leading part.
        leading.start = first;
        requests.insert(0, 0..first);
    }
    page(&requests, PageAt::Latest, |range, _| {
        Some(light_bytes(&blocks[range]))
    })
    .start
}

/// Where a reset of `blocks` starts for a page that holds them from `from`,
/// whose block there it holds as `edge`: at `from` while that block is
/// still `edge`, and otherwise, or when the page names none or only one of
/// them, at the start of the latest page.
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
