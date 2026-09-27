//! Retired tool media (`runtime.md` § Retired tool media): a tool result's
//! image or video that is more than 30 days old gives way, in its place in
//! the result, to one line of text that says what it was and when it was
//! removed, once no request can send it while a vendor may still keep that
//! request in its cache. A message's media come from the user's uploads and
//! are never retired. The backend applies the rule to stored conversations
//! (`storage.md` § Retiring tool media).

use demi_core::{Block, Timestamp, ToolMediaSource, ToolResultContentBlock};
use jiff::SignedDuration;
use jiff::tz::TimeZone;

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
/// each with its expired tool media retired. A `tool_call` block more than
/// [`KEPT`] old gives up its media when the conversation is idle, or when
/// it lies before the node's last `compaction_boundary` and that boundary
/// is more than [`CACHE_LIFETIME`] old: replay starts at the boundary, so
/// no request has sent the medium since the summary request.
pub fn retire(blocks: &[Block], retirement: Retirement) -> Vec<(usize, Block)> {
    let start = replay_start(blocks);
    let summarized = match blocks.get(start) {
        Some(Block::CompactionBoundary(boundary)) => older_than(boundary.created_at, CACHE_LIFETIME, retirement.now),
        _ => false,
    };
    let text = |kind: &str, media_type: &str| ToolResultContentBlock::Text {
        text: format!(
            "[{kind}:{media_type}, removed on {}: a tool result's images and videos are kept for 30 days]",
            retirement.now.to_jiff().to_zoned(TimeZone::UTC).date()
        ),
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
            let replacement = match part {
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Ref { media_type, .. },
                } => text("image", media_type),
                ToolResultContentBlock::Video {
                    source: ToolMediaSource::Ref { media_type, .. },
                } => text("video", media_type),
                _ => continue,
            };
            *part = replacement;
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

#[cfg(test)]
mod tests {
    use demi_core::{
        BlobRef, BlockId, CompactionBoundaryBlock, MediaSource, ToolCallBlock, ToolCallStatus, UserBlock,
        UserContentBlock,
    };

    use super::*;
    use crate::testing::test_model;

    const DAY: i64 = 24 * 60 * 60 * 1000;

    /// 2026-10-01T12:00:00Z.
    fn now() -> Timestamp {
        "2026-10-01T12:00:00Z".parse().unwrap()
    }

    fn days_ago(days: i64) -> Timestamp {
        Timestamp::from_millisecond(now().as_millisecond() - days * DAY).unwrap()
    }

    fn blob(byte: u8) -> BlobRef {
        BlobRef::of(&[byte])
    }

    fn id(value: &str) -> BlockId {
        BlockId::try_from(value).unwrap()
    }

    /// A shell call whose result holds a line of text, an image and a video.
    fn shot(name: &str, age_days: i64) -> Block {
        Block::ToolCall(ToolCallBlock {
            id: id(name),
            created_at: days_ago(age_days),
            model: test_model(),
            tool_use_id: format!("toolu_{name}"),
            tool_name: "shell_exec".into(),
            input: "{}".into(),
            status: ToolCallStatus::Completed,
            output: vec![
                ToolResultContentBlock::Text { text: "exit 0".into() },
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Ref {
                        r#ref: blob(1),
                        media_type: "image/png".into(),
                    },
                },
                ToolResultContentBlock::Video {
                    source: ToolMediaSource::Ref {
                        r#ref: blob(2),
                        media_type: "video/mp4".into(),
                    },
                },
            ],
            view: None,
        })
    }

    /// A message with an uploaded image.
    fn upload(name: &str, age_days: i64) -> Block {
        Block::User(UserBlock {
            id: id(name),
            turn_id: name.try_into().unwrap(),
            created_at: days_ago(age_days),
            model: test_model(),
            content: vec![UserContentBlock::Image {
                source: MediaSource::Ref {
                    r#ref: blob(3),
                    media_type: "image/png".into(),
                },
            }],
            preamble: None,
        })
    }

    fn boundary(age_days: i64) -> Block {
        Block::CompactionBoundary(CompactionBoundaryBlock {
            id: id("boundary"),
            created_at: days_ago(age_days),
            model: test_model(),
            summary: "Earlier work.".into(),
            summary_tokens: 3,
        })
    }

    fn retired(name: &str, age_days: i64) -> Block {
        let Block::ToolCall(mut call) = shot(name, age_days) else {
            unreachable!("a shot is a tool call")
        };
        call.output[1] = ToolResultContentBlock::Text {
            text: "[image:image/png, removed on 2026-10-01: a tool result's images and videos are kept for 30 days]"
                .into(),
        };
        call.output[2] = ToolResultContentBlock::Text {
            text: "[video:video/mp4, removed on 2026-10-01: a tool result's images and videos are kept for 30 days]"
                .into(),
        };
        Block::ToolCall(call)
    }

    /// The indexes the rule changes in each situation.
    #[test]
    fn a_tool_medium_goes_after_30_days_once_no_cache_can_hold_a_request_that_sent_it() {
        let cases: [(&str, Vec<Block>, bool, Vec<usize>); 7] = [
            (
                "31 days old, before a boundary two days old",
                vec![upload("u1", 40), shot("a", 31), boundary(2), shot("b", 31)],
                false,
                vec![1],
            ),
            ("30 days old at most, before an old boundary", vec![shot("a", 30), boundary(2)], false, vec![]),
            (
                "before a boundary less than a day old, which an edit could remove",
                vec![shot("a", 31), boundary(0)],
                false,
                vec![],
            ),
            ("in the replayed window of a conversation in use", vec![shot("a", 31)], false, vec![]),
            (
                "anywhere in a conversation idle for 30 days",
                vec![shot("a", 31), boundary(0), shot("b", 31), shot("c", 29)],
                true,
                vec![0, 2],
            ),
            ("a message's upload, however old and idle", vec![upload("u1", 90), boundary(40)], true, vec![]),
            (
                "only the last boundary counts",
                vec![shot("a", 31), boundary(5), shot("b", 31), boundary(0)],
                false,
                vec![],
            ),
        ];
        for (situation, blocks, idle, expected) in cases {
            let changed = retire(&blocks, Retirement { now: now(), idle });
            let indexes: Vec<usize> = changed.iter().map(|(index, _)| *index).collect();
            assert_eq!(indexes, expected, "{situation}");
            for (index, block) in changed {
                let Block::ToolCall(call) = &blocks[index] else {
                    panic!("{situation}: only a tool call changes");
                };
                // The text takes each medium's place; the rest of the block
                // stays as it was.
                assert_eq!(block, retired(call.id.as_str(), 31), "{situation}");
            }
        }
    }
}
