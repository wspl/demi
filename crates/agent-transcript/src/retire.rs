//! Retired tool media (`runtime.md` § Retired tool media): a tool result's
//! image or video that is more than 30 days old is gone from the result,
//! and in its place a part says what it was and when it was removed, once
//! no request can send it while a vendor may still keep that request in its
//! cache. A message's media come from the user's uploads and are never
//! retired. The backend applies the rule to stored conversations
//! (`storage.md` § Retiring tool media).

use demi_shared_types::{
    Block, GoneCause, ModelMediaKind, Timestamp, ToolMediaSource, ToolResultContentBlock,
};
use jiff::SignedDuration;

use super::replay_start;

/// How long a tool result's image or video stays in its block.
pub const KEPT: SignedDuration = SignedDuration::from_hours(30 * 24);

/// How long a vendor keeps a request in its cache, at most: OpenAI keeps an
/// entry up to 24 hours, the others less.
pub const CACHE_LIFETIME: SignedDuration = SignedDuration::from_hours(24);

/// When the rule is applied, and to what kind of conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retirement {
    pub now: Timestamp,
    /// Whether the conversation has been idle for [`KEPT`]: then no request
    /// of it has been sent for that long, and every expired tool medium
    /// goes, wherever it lies.
    pub idle: bool,
}

/// The blocks of one node's transcript that the rule changes, by index,
/// each with its expired tool media retired: gone, retired now. A
/// `tool_call` block more than [`KEPT`] old gives up its media when the
/// conversation is idle, or when it lies before the node's last
/// `compaction_boundary` and that boundary is more than [`CACHE_LIFETIME`]
/// old: replay starts at the boundary, so no request has sent the medium
/// since the summary request.
pub fn retire(blocks: &[Block], retirement: Retirement) -> Vec<(usize, Block)> {
    let start = replay_start(blocks);
    let summarized = match blocks.get(start) {
        Some(Block::CompactionBoundary(boundary)) => older_than(boundary.created_at, CACHE_LIFETIME, retirement.now),
        _ => false,
    };
    let mut changed = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let Block::ToolCall(call) = block else {
            continue;
        };
        let expired = older_than(call.created_at, KEPT, retirement.now);
        if !expired || !(retirement.idle || (summarized && index < start)) {
            continue;
        }
        let mut retired = call.clone();
        let mut any = false;
        for part in &mut retired.output {
            let (kind, media_type) = match part {
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Ref { media_type, .. },
                } => (ModelMediaKind::Image, media_type.clone()),
                ToolResultContentBlock::Video {
                    source: ToolMediaSource::Ref { media_type, .. },
                } => (ModelMediaKind::Video, media_type.clone()),
                ToolResultContentBlock::Text { .. } | ToolResultContentBlock::Gone { .. } => {
                    continue;
                }
            };
            *part = ToolResultContentBlock::Gone {
                kind,
                media_type,
                cause: GoneCause::Retired { at: retirement.now },
            };
            any = true;
        }
        if any {
            changed.push((index, Block::ToolCall(retired)));
        }
    }
    changed
}

/// Whether `time` lies more than `age` before `now`.
fn older_than(time: Timestamp, age: SignedDuration, now: Timestamp) -> bool {
    now.to_jiff().duration_since(time.to_jiff()) > age
}
