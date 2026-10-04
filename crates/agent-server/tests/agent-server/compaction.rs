//! Compaction at the socket: the usage the page shows, the `compact` frame
//! the user may send only from half the window, and a failed compaction,
//! which is a failure record like any failed request.

use demi_conversation_socket_protocol::{ClientFrame, ClientFrameKind, ServerFrame};
use demi_provider_common::testing::{ScriptedRuntime, Turn, event};
use demi_shared_types::{Block, ContextUsage, SessionPhase};

use crate::support::{
    Fixture, is_context_usage, is_idle, is_pending_steers, kinds, open, send, session_of,
};

/// The test model's window, which `model_of` gives every scripted model.
const WINDOW: u64 = 100_000;

fn usage(tokens: u64) -> ServerFrame {
    ServerFrame::ContextUsage {
        usage: ContextUsage {
            tokens,
            window: Some(WINDOW),
            compact_from: Some(WINDOW / 2),
        },
    }
}

fn last_usage(frames: &[ServerFrame]) -> Option<&ServerFrame> {
    frames.iter().rfind(|frame| is_context_usage(frame))
}

/// A turn that answers `text` and reports `input_tokens` of usage.
fn answer(text: &str, input_tokens: u64) -> Turn {
    Turn::Events(vec![event::text(text), event::response(input_tokens, 0)])
}

#[tokio::test(flavor = "local")]
async fn the_page_is_told_the_usage_and_compacts_only_from_half_the_window() {
    let script = ScriptedRuntime::new([
        answer("a", 30_000),
        answer("b", 60_000),
        answer("the summary", 10),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.client();

    client.send(open()).await;
    client.next_until(is_pending_steers).await;
    // The usage follows the handshake.
    let opened = client.next_until(is_context_usage).await;
    assert_eq!(opened.last(), Some(&usage(0)));

    client.send(send("m1", "first")).await;
    let turn = client.next_until(is_idle).await;
    assert_eq!(last_usage(&turn), Some(&usage(30_000)));
    client.send(ClientFrame::Compact {}).await;
    assert_eq!(
        client.next().await,
        Some(ServerFrame::Rejected {
            command: ClientFrameKind::Compact,
            reason: "Compaction is available from 50% context usage (now 30%)".into(),
        })
    );

    client.send(send("m2", "second")).await;
    let turn = client.next_until(is_idle).await;
    assert_eq!(last_usage(&turn), Some(&usage(60_000)));
    client.send(ClientFrame::Compact {}).await;
    let compaction = client.next_until(is_idle).await;
    assert!(compaction.contains(&ServerFrame::Phase {
        phase: SessionPhase::Compacting
    }));
    let blocks = session_of(&fixture).transcript().blocks;
    assert_eq!(kinds(&blocks).last().map(String::as_str), Some("compaction_marker"));
    // The summary replaced what the response measured: the usage is the
    // estimate of the blocks from the boundary on, far below the half.
    let Some(ServerFrame::ContextUsage { usage }) = last_usage(&compaction) else {
        panic!("the compaction reported no usage: {compaction:?}");
    };
    assert!(usage.tokens < WINDOW / 2, "{usage:?}");
}

#[tokio::test(flavor = "local")]
async fn a_failed_compaction_is_a_failure_record_that_survives_a_reload_and_its_turn_resumes() {
    let script = ScriptedRuntime::new([
        // The answer reaches 80% of the window, so the turn compacts after it,
        // and the summary request fails for good.
        answer("a", 85_000),
        Turn::Events(vec![event::error("The summary request failed", None)]),
        // Resume compacts first, then finishes the turn.
        answer("the summary", 10),
        answer("done", 10),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;

    client.send(send("m1", "first")).await;
    client.next_until(is_idle).await;

    client.send(ClientFrame::Close {}).await;
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;
    let mut reopened = fixture.client();
    reopened.send(open()).await;
    let handshake = reopened.next_until(is_pending_steers).await;
    let Some(ServerFrame::TranscriptReset { blocks, .. }) = handshake.get(1) else {
        panic!("no transcript in the handshake: {handshake:?}");
    };
    let Some(Block::Error(error)) = blocks.last() else {
        panic!("the transcript does not end with the failure: {blocks:?}");
    };
    assert_eq!(error.message, "The summary request failed");
    // The pass ran inside the turn, which the failure ended.
    assert!(!error.outside_turn);

    reopened.send(ClientFrame::Resume {}).await;
    reopened.next_until(is_idle).await;
    let blocks = session_of(&fixture).transcript().blocks;
    let answer = blocks.iter().rev().find_map(|block| match block {
        Block::Text(text) => Some(text.text.as_str()),
        _ => None,
    });
    assert_eq!(answer, Some("done"), "{:?}", kinds(&blocks));
    assert_eq!(script.remaining(), 0);
}
