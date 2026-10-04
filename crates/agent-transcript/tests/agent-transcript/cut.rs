//! Where a transcript is cut for a resume (`runtime.md` § Resume) and for a
//! Fork (`conversation-fork.md` § Eligibility).

use demi_agent_store::testing::test_model;
use demi_agent_transcript::{CutError, resume_point, through_assistant};
use demi_shared_types::{
    AbortBlock, Block, BlockId, ErrorBlock, ResponseBlock, TextBlock, ThinkingBlock, Timestamp,
    TokenUsage, ToolCallBlock, ToolCallStatus, TurnId, UserBlock,
};

fn id(value: &str) -> BlockId {
    BlockId::try_from(value).unwrap()
}

fn user(value: &str) -> Block {
    Block::User(UserBlock {
        id: id(value),
        turn_id: TurnId::try_from(value).unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        content: Vec::new(),
        preamble: None,
    })
}

fn text(value: &str) -> Block {
    Block::Text(TextBlock {
        id: id("text"),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        text: value.into(),
        forkable: false,
    })
}

/// `text` once it is complete.
fn completed(value: &str) -> Block {
    let Block::Text(block) = text(value) else {
        unreachable!("text makes a text block")
    };
    Block::Text(TextBlock {
        forkable: true,
        ..block
    })
}

fn thinking() -> Block {
    Block::Thinking(ThinkingBlock {
        id: id("thinking"),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        text: "hmm".into(),
        signature: None,
    })
}

fn error() -> Block {
    Block::Error(ErrorBlock {
        id: id("error"),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        message: "failed".into(),
        code: None,
        diagnostics: None,
        outside_turn: false,
    })
}

fn response() -> Block {
    Block::Response(ResponseBlock {
        id: id("response"),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        usage: TokenUsage::default(),
    })
}

fn tool_call(status: ToolCallStatus) -> Block {
    Block::ToolCall(ToolCallBlock {
        id: id("call"),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        tool_use_id: "call-1".into(),
        tool_name: "note".into(),
        input: "{}".into(),
        status,
        output: Vec::new(),
        view: None,
    })
}

fn abort() -> Block {
    Block::Abort(AbortBlock {
        id: id("abort"),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        is_resumed: false,
    })
}

#[test]
fn the_resume_point_drops_only_what_nobody_can_have_acted_on() {
    let point = |blocks: &[Block]| {
        let point = resume_point(blocks);
        (point.cut, point.rerun.is_some())
    };
    // The failure alone: the turn reruns.
    assert_eq!(point(&[user("u1"), thinking(), error()]), (1, true));
    // Blank text is a leftover; emitted text, a response and a stop are
    // not.
    assert_eq!(point(&[user("u1"), text(" \n"), error()]), (1, true));
    assert_eq!(point(&[user("u1"), text("posted"), error()]), (2, false));
    assert_eq!(point(&[user("u1"), response(), thinking()]), (2, false));
    assert_eq!(point(&[user("u1"), abort()]), (2, false));
    // A tool call stops it whatever its status: one still executing
    // outlived its process, and its effect may have landed.
    for status in [ToolCallStatus::Completed, ToolCallStatus::Executing] {
        assert_eq!(
            point(&[user("u1"), tool_call(status), thinking(), error()]),
            (2, false),
            "{status}"
        );
    }
    // Only the latest turn is in play.
    assert_eq!(
        point(&[user("u1"), text("answer"), response(), user("u2"), error()]),
        (4, true)
    );
    assert_eq!(point(&[]), (0, false));
}

#[test]
fn a_fork_ends_only_at_completed_text_with_no_call_before_it_still_executing() {
    let target = id("text");
    let cut =
        |blocks: &[Block], target: &BlockId| through_assistant(blocks, target).map(<[Block]>::len);
    // Text still streaming is refused; once complete, it ends the Fork.
    assert_eq!(
        cut(&[user("u1"), text("answer")], &target),
        Err(CutError::NotCompletedText)
    );
    assert_eq!(
        cut(&[user("u1"), completed("answer"), response()], &target),
        Ok(2)
    );
    // Only text ends a Fork.
    assert_eq!(
        cut(&[user("u1"), completed("answer")], &id("u1")),
        Err(CutError::NotCompletedText)
    );
    // A call before it still executing refuses it; a completed one does not.
    let after_call = |status| [user("u1"), tool_call(status), completed("answer")];
    assert_eq!(
        cut(&after_call(ToolCallStatus::Executing), &target),
        Err(CutError::UnfinishedToolCalls)
    );
    assert_eq!(cut(&after_call(ToolCallStatus::Completed), &target), Ok(3));
}
