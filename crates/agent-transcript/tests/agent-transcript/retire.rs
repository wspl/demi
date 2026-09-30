//! The retirement of a tool result's expired images and videos
//! (`runtime.md` § Media).

use demi_agent_store::testing::test_model;
use demi_agent_transcript::retire::{Retirement, retire};
use demi_core::{
    BlobRef, Block, BlockId, CompactionBoundaryBlock, GoneCause, MediaSource, ModelMediaKind,
    Timestamp, ToolCallBlock, ToolCallStatus, ToolMediaSource, ToolResultContentBlock, UserBlock,
    UserContentBlock,
};

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
            ToolResultContentBlock::Text {
                text: "exit 0".into(),
            },
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
    let gone = |kind, media_type: &str| ToolResultContentBlock::Gone {
        kind,
        media_type: media_type.into(),
        cause: GoneCause::Retired { at: now() },
    };
    call.output[1] = gone(ModelMediaKind::Image, "image/png");
    call.output[2] = gone(ModelMediaKind::Video, "video/mp4");
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
        (
            "30 days old at most, before an old boundary",
            vec![shot("a", 30), boundary(2)],
            false,
            vec![],
        ),
        (
            "before a boundary less than a day old, which an edit could remove",
            vec![shot("a", 31), boundary(0)],
            false,
            vec![],
        ),
        (
            "in the replayed window of a conversation in use",
            vec![shot("a", 31)],
            false,
            vec![],
        ),
        (
            "anywhere in a conversation idle for 30 days",
            vec![shot("a", 31), boundary(0), shot("b", 31), shot("c", 29)],
            true,
            vec![0, 2],
        ),
        (
            "a message's upload, however old and idle",
            vec![upload("u1", 90), boundary(40)],
            true,
            vec![],
        ),
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
            // A part that says each medium is gone takes its place; the
            // rest of the block stays as it was.
            assert_eq!(block, retired(call.id.as_str(), 31), "{situation}");
        }
    }
}
