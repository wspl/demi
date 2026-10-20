//! The output of the request the root agent runs, as the pages see it
//! (`crates/agent-server/src/server/tree/reading.rs`).
//!
//! The tests cover:
//!
//! - A page that opens in the middle of a request finds the count so far and
//!   the time since the first token in its handshake, then the response's
//!   own count, then nothing once the turn ended.
//! - A retried attempt counts from zero, and the failed attempt's count
//!   stays meanwhile; a stopped request ends with what it streamed.
//! - While a request streams, the reading goes at most every 500 ms.
//! - The speed's clock starts at the first output that adds to the count.
//! - Each response of a run ends its request, and the next starts from zero.
//! - A call's arguments that come whole count.
//! - A response's own count goes out when it ends, before the turn's end.
//!
//! The provider is scripted, on Tokio's paused clock.

use std::time::Duration;

use demi_conversation_socket_protocol::{ClientFrame, RequestOutput, ServerFrame, TranscriptPatch};
use demi_shared_types::Block;
use demi_provider_common::{
    CallEvent, ErrorCode, ProviderEvent,
    testing::{ScriptedRuntime, Turn, event},
};
use futures_util::{StreamExt as _, stream};
use jiff::SignedDuration;
use serde_json::json;
use tokio::time::Instant;

use crate::support::{Fixture, Gate, send, session_of, streamed_then_held, until};

/// The readings among `frames`, in order.
fn readings(frames: &[ServerFrame]) -> Vec<Option<RequestOutput>> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::RequestOutput { output } => Some(output.clone()),
            _ => None,
        })
        .collect()
}

// A request held in the middle, a second page and the end: a few
// milliseconds on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_page_that_opens_in_the_middle_of_a_request_finds_its_count_and_then_the_responses_own() {
    let gate = Gate::new();
    let script = ScriptedRuntime::new([streamed_then_held(
        &gate,
        vec![
            // 12 characters and 2 CJK ones: 3 + 2 tokens.
            event::text("Hello 你好 world"),
            // A call's framing, and 15 characters of its arguments with
            // the 12 above: 50 + 7 - 3 tokens more.
            event::tool_call_start("call-1", "shell"),
            event::tool_call_input("call-1", r#"{"script":"ls"}"#),
        ],
        vec![event::response(10, 42)],
    )]);
    let (fixture, clock) = Fixture::with_clock(&script);
    let first = fixture.opened().await;
    first.send(send("m1", "List the files.")).await;
    let session = session_of(&fixture);
    until(|| session.request_output().is_some_and(|output| output.tokens == 59)).await;
    clock.advance(SignedDuration::from_secs(2));

    let mut second = fixture.client();
    second.open().await;
    let handshake = second.next_until(|frame| matches!(frame, ServerFrame::RequestOutput { .. })).await;
    let [Some(reading)] = &readings(&handshake)[..] else {
        panic!("{handshake:?}");
    };
    assert_eq!((reading.tokens, reading.streamed_ms, reading.ended), (59, Some(2_000), false));

    // The response's own count replaces the estimate, and the turn's end
    // leaves no reading.
    gate.open();
    let frames = second.until_idle().await;
    let ended = readings(&frames);
    let tail: Vec<_> = ended
        .iter()
        .map(|output| output.as_ref().map(|output| (output.request_id.as_str(), output.tokens, output.ended)))
        .collect();
    assert_eq!(tail[tail.len() - 2..], [Some((reading.request_id.as_str(), 42, true)), None], "{frames:?}");
    assert_eq!(session.request_output(), None);
    drop(first);
}

// A failed attempt, its backoff and a held retry: a few milliseconds on
// the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_retried_request_counts_from_zero() {
    let gate = Gate::new();
    let script = ScriptedRuntime::new([
        // Reasoning is unwound, so the attempt is retried.
        Turn::Events(vec![
            ProviderEvent::ThinkingStart,
            event::thinking("Considering the files"),
            event::error("overloaded", Some(ErrorCode::Overloaded)),
        ]),
        streamed_then_held(&gate, vec![event::text("ab")], vec![event::response(1, 1)]),
    ]);
    let fixture = Fixture::new(&script);
    let mut page = fixture.opened().await;
    page.send(send("m1", "Look.")).await;
    let failed = page.next_until(|frame| matches!(frame, ServerFrame::RetryScheduled { .. })).await;
    let attempt = readings(&failed)
        .into_iter()
        .flatten()
        .map(|output| output.request_id)
        .next()
        .expect("the attempt started");
    // The failed attempt's count, its 21 characters of reasoning, stays
    // during the wait.
    let last = readings(&failed).into_iter().flatten().last().expect("the attempt ended");
    assert_eq!((last.request_id == attempt, last.tokens, last.ended), (true, 6, true), "{failed:?}");
    let frames = page
        .next_until(|frame| {
            matches!(frame, ServerFrame::RequestOutput { output: Some(output) } if output.request_id != attempt && output.tokens > 0)
        })
        .await;
    let counts: Vec<u64> = readings(&frames)
        .into_iter()
        .flatten()
        .filter(|output| output.request_id != attempt)
        .map(|output| output.tokens)
        .collect();
    assert_eq!(counts, [0, 1], "{frames:?}");
    gate.open();
    page.until_idle().await;
}

// Twenty deltas 100 ms apart: two seconds of paused time, a few
// milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn the_reading_of_a_streaming_request_goes_at_most_every_half_second() {
    let script = ScriptedRuntime::new([Turn::Stream(Box::new(|_| {
        stream::iter(0..21)
            .then(|delta| async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if delta < 20 { event::text("four") } else { event::response(1, 20) }
            })
            .boxed_local()
    }))]);
    let fixture = Fixture::new(&script);
    let mut page = fixture.opened().await;
    page.send(send("m1", "Count.")).await;
    let mut streaming = Vec::new();
    loop {
        let frame = page.next().await.expect("the page stays attached");
        if let ServerFrame::RequestOutput { output: Some(output) } = &frame {
            if output.ended {
                break;
            }
            streaming.push(Instant::now());
        }
    }
    assert!(streaming.len() >= 3, "{streaming:?}");
    for pair in streaming.windows(2) {
        assert!(pair[1] - pair[0] >= Duration::from_millis(500), "{streaming:?}");
    }
    page.until_idle().await;
}

/// The ended readings among `frames`, each request's last, in order.
fn ends(frames: &[ServerFrame]) -> Vec<(String, u64)> {
    readings(frames)
        .into_iter()
        .flatten()
        .filter(|output| output.ended)
        .map(|output| (output.request_id, output.tokens))
        .collect()
}

// A request stopped before its first frame of counting: a few milliseconds
// on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_request_stopped_in_its_first_half_second_ends_with_what_it_streamed() {
    let gate = Gate::new();
    let script = ScriptedRuntime::new([streamed_then_held(&gate, vec![event::text("Hello there")], vec![event::response(1, 1)])]);
    let fixture = Fixture::new(&script);
    let mut page = fixture.opened().await;
    page.send(send("m1", "Greet.")).await;
    let session = session_of(&fixture);
    until(|| session.request_output().is_some_and(|output| output.tokens == 3)).await;
    page.send(ClientFrame::Abort {}).await;
    let frames = page.until_idle().await;
    let shown = readings(&frames);
    let [.., Some(last), None] = &shown[..] else {
        panic!("{frames:?}");
    };
    assert_eq!((last.tokens, last.ended), (3, true), "{frames:?}");
    drop(gate);
}

// One run, held after an empty reasoning block while the clock moves: a few
// milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn the_speed_counts_from_the_first_output_that_adds_to_the_count() {
    let gate = Gate::new();
    let script = ScriptedRuntime::new([streamed_then_held(
        &gate,
        vec![ProviderEvent::ThinkingStart],
        vec![event::thinking("abcd"), event::text("Done."), event::response(1, 5)],
    )]);
    let (fixture, clock) = Fixture::with_clock(&script);
    let mut page = fixture.opened().await;
    page.send(send("m1", "Think.")).await;
    let session = session_of(&fixture);
    until(|| session.transcript().blocks.iter().any(|block| matches!(block, Block::Thinking(_)))).await;
    clock.advance(SignedDuration::from_secs(5));
    gate.open();
    let frames = page.until_idle().await;
    let ended: Vec<_> = readings(&frames).into_iter().flatten().filter(|output| output.ended).collect();
    assert_eq!(ended.iter().map(|output| output.streamed_ms).collect::<Vec<_>>(), [Some(0)], "{frames:?}");
}

// One run of two responses in the shape of a Claude Code run that answers
// two messages in turns of their own (`crates/provider-claude-code/src/run.rs`):
// the first call started with the run, the second starts with its own
// `Call(Started)`, and each ends with its response, the usage of its last
// call. A few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn each_response_of_a_run_ends_its_request_and_the_next_counts_from_zero() {
    let script = ScriptedRuntime::new([Turn::Events(vec![
        event::text("abcd"),
        event::response(1, 7),
        ProviderEvent::Call(CallEvent::Started),
        event::text("efghijkl"),
        event::response(2, 9),
    ])]);
    let fixture = Fixture::new(&script);
    let mut page = fixture.opened().await;
    page.send(send("m1", "Answer twice.")).await;
    let frames = page.until_idle().await;
    let ended = ends(&frames);
    assert_eq!(ended.iter().map(|(_, tokens)| *tokens).collect::<Vec<_>>(), [7, 9], "{frames:?}");
    assert_ne!(ended[0].0, ended[1].0);
    // The second request's first reading counts its own text only.
    let second: Vec<u64> = readings(&frames)
        .into_iter()
        .flatten()
        .filter(|output| output.request_id == ended[1].0)
        .map(|output| output.tokens)
        .collect();
    assert_eq!(second.first(), Some(&2), "{frames:?}");
}

// A call opened, then given whole: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_call_whose_arguments_come_whole_counts_them() {
    let gate = Gate::new();
    let script = ScriptedRuntime::new([
        streamed_then_held(
            &gate,
            vec![
                event::tool_call_start("call-1", "no_such_tool"),
                event::tool_call("call-1", "no_such_tool", json!({ "path": "notes.md" })),
            ],
            vec![event::response(1, 1)],
        ),
        Turn::Events(vec![event::text("No such tool."), event::response(1, 1)]),
    ]);
    let fixture = Fixture::new(&script);
    let mut page = fixture.opened().await;
    page.send(send("m1", "Look.")).await;
    let session = session_of(&fixture);
    until(|| session.transcript().blocks.iter().any(|block| matches!(block, Block::ToolCall(_)))).await;
    // The framing and the 19 characters of `{"path":"notes.md"}`.
    assert_eq!(session.request_output().map(|output| output.tokens), Some(55));
    gate.open();
    page.until_idle().await;
}

// A turn's last response: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_responses_own_count_goes_out_when_it_ends_before_the_turns_end() {
    let script = ScriptedRuntime::new([Turn::Events(vec![event::text("Done here."), event::response(3, 4)])]);
    let fixture = Fixture::new(&script);
    let mut page = fixture.opened().await;
    page.send(send("m1", "Finish.")).await;
    let frames = page.until_idle().await;
    let ended = frames
        .iter()
        .position(|frame| matches!(frame, ServerFrame::RequestOutput { output: Some(output) } if output.ended && output.tokens == 4))
        .expect("the response's own count went out");
    let response = frames
        .iter()
        .position(|frame| {
            matches!(frame, ServerFrame::TranscriptPatch { patches, .. }
                if patches.iter().any(|patch| matches!(patch, TranscriptPatch::Add { value: Block::Response(_), .. })))
        })
        .expect("the response's block went out");
    assert!(ended < response, "{frames:?}");
}
