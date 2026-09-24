//! What the provider reads of the CLI's lines (`claude-code.md` § Reading;
//! `providers.md` § Reading vendor input): the events of streamed and whole
//! messages, the lines and blocks it skips, and the failures of lines it
//! cannot read.

use demi_core::TokenUsage;
use demi_provider::{ErrorCode, ProviderEvent, ProviderFailure};
use demi_shell::Signal;
use serde_json::{Value, json};

use crate::cli::*;

/// The events of a run in which the CLI answers with `lines`, and the
/// signals its process received.
async fn answer(lines: Vec<Value>) -> (Vec<ProviderEvent>, Vec<Signal>) {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            for line in lines {
                cli.say(line);
            }
            cli
        }
    );
    (events, cli.signals())
}

fn done() -> Value {
    json!({ "type": "result", "usage": { "input_tokens": 1, "output_tokens": 1 } })
}

fn response() -> ProviderEvent {
    ProviderEvent::Response(TokenUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    })
}

fn failure(events: &[ProviderEvent]) -> &ProviderFailure {
    match events {
        [ProviderEvent::Error(failure)] => failure,
        other => panic!("expected one failure, got {other:?}"),
    }
}

#[tokio::test(flavor = "local")]
async fn a_whole_message_gives_its_text_and_reasoning_until_the_process_streams() {
    let (events, _) = answer(vec![
        json!({ "type": "assistant", "message": { "content": [
            { "type": "thinking", "thinking": "considering", "signature": "sig-1" },
            { "type": "redacted_thinking", "data": "opaque" },
            { "type": "server_tool_use", "id": "srv" },
            { "type": "text", "text": "" },
            { "type": "text", "text": "hello" },
        ] } }),
        done(),
    ])
    .await;
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("considering".into()),
            ProviderEvent::ThinkingSignature("sig-1".into()),
            ProviderEvent::RedactedThinking("opaque".into()),
            ProviderEvent::TextDelta("hello".into()),
            response(),
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn streamed_pieces_become_events_and_the_whole_message_after_them_repeats_nothing() {
    let (events, _) = answer(vec![
        json!({ "type": "stream_event", "event": { "type": "message_start", "message": { "content": [] } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_start", "content_block": { "type": "thinking" } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "hmm" } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "thinking_delta" } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "signature_delta", "signature": "sig" } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_start", "content_block": { "type": "text", "text": "" } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": "hi" } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "input_json_delta", "partial_json": "{" } } }),
        json!({ "type": "stream_event", "event": { "type": "message_delta", "usage": { "output_tokens": 3 } } }),
        json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "hi" }] } }),
        json!({ "type": "stream_event", "event": { "type": "message_stop" } }),
        json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "allowed" } }),
        done(),
    ])
    .await;
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("hmm".into()),
            ProviderEvent::ThinkingSignature("sig".into()),
            ProviderEvent::TextDelta("hi".into()),
            response(),
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_line_demi_cannot_read_fails_the_run_with_the_line_as_its_record_and_closes_the_process()
{
    // A text delta whose text is not text.
    let malformed = json!({
        "type": "stream_event",
        "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": { "value": "hi" } } },
    });
    let (events, signals) = answer(vec![malformed.clone()]).await;
    let failed = failure(&events);
    assert!(failed.message.contains("text"), "{}", failed.message);
    assert_eq!(failed.code, None);
    let record = failed
        .diagnostics
        .as_ref()
        .unwrap()
        .upstream
        .as_deref()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(record).unwrap(), malformed);
    assert_eq!(signals, [Signal::Terminate]);

    // A line without a type names no message Demi can route.
    let (events, _) = answer(vec![json!({ "result": "ok" })]).await;
    assert!(
        failure(&events).message.contains("type"),
        "{}",
        failure(&events).message
    );

    // A tool use without an id or a name.
    let (events, signals) = answer(vec![json!({
        "type": "assistant",
        "message": { "content": [{ "type": "tool_use", "name": "mcp__main__shell_exec", "input": {} }] },
    })])
    .await;
    assert_eq!(
        failure(&events).message,
        "Invalid tool_use block from Claude Code"
    );
    assert_eq!(signals, [Signal::Terminate]);
}

#[tokio::test(flavor = "local")]
async fn the_clis_own_failures_are_worded_as_it_worded_them_and_classified_by_their_words() {
    let (events, signals) = answer(vec![
        json!({ "type": "error", "message": "rate limited, try later" }),
    ])
    .await;
    let failed = failure(&events);
    assert_eq!(failed.message, "rate limited, try later");
    assert_eq!(failed.code, Some(ErrorCode::RateLimit));
    assert_eq!(signals, [Signal::Terminate]);

    let (events, _) = answer(vec![
        json!({ "type": "error", "message": "boom", "code": "auth_expired" }),
    ])
    .await;
    assert_eq!(failure(&events).code, Some(ErrorCode::AuthExpired));

    let (events, _) = answer(vec![
        json!({ "type": "error", "message": { "detail": "boom" } }),
    ])
    .await;
    assert_eq!(failure(&events).message, "Claude Code error");
    assert_eq!(failure(&events).code, None);

    let (events, signals) = answer(vec![json!({
        "type": "result",
        "is_error": true,
        "result": "authentication failed",
        "errors": [3],
    })])
    .await;
    assert_eq!(failure(&events).message, "authentication failed");
    assert_eq!(failure(&events).code, Some(ErrorCode::AuthExpired));
    assert!(signals.is_empty());

    let (events, _) = answer(vec![
        json!({ "type": "result", "is_error": true, "result": { "text": 1 } }),
    ])
    .await;
    assert_eq!(failure(&events).message, "Claude Code returned an error");
}
