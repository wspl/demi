//! What the provider reads of the CLI's lines
//! (`crates/provider-claude-code/src/output.rs`,
//! `crates/provider-common/src/wire/mod.rs`): the events of streamed and whole
//! messages, the lines and blocks it skips, and the failures of lines it
//! cannot read.

use demi_host_interface::Signal;
use demi_provider_common::{ErrorCode, Provider as _, ProviderEvent, ProviderFailure};
use demi_shared_types::TokenUsage;
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
            let cli = starts.next().await;
            for line in lines {
                cli.say(line);
            }
            cli
        }
    );
    (events, cli.signals())
}

/// The end of the turn's last call, whose usage the response carries.
fn call_end() -> Value {
    json!({ "type": "stream_event", "event": { "type": "message_delta", "usage": { "input_tokens": 1, "output_tokens": 1 } } })
}

fn done() -> Value {
    json!({ "type": "result", "usage": { "input_tokens": 3, "output_tokens": 3 } })
}

fn response() -> ProviderEvent {
    ProviderEvent::Response(Some(TokenUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }))
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
        call_end(),
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
        // A tool use streams its start and input pieces, which name its
        // block by index (`mcp.rs` shows them); one of a tool the request
        // does not offer shows neither, and a piece of a block never opened
        // is skipped.
        json!({ "type": "stream_event", "event": { "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "toolu_1", "name": "mcp__main__shell_exec", "input": {} } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"descr" } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": 3, "delta": { "type": "input_json_delta", "partial_json": "{" } } }),
        call_end(),
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
async fn a_call_of_a_tool_the_request_does_not_offer_is_the_clis_to_answer_and_the_run_reads_on() {
    // The model named a tool the request does not offer: the CLI answers
    // the call with an error itself and asks the model again in the same
    // run (`crates/provider-claude-code/src/mcp.rs`).
    let (events, signals) = answer(vec![
        json!({ "type": "stream_event", "event": { "type": "message_start", "message": { "content": [] } } }),
        json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": "toolu_1", "name": "demi agent send parent", "input": {} },
        ] } }),
        json!({ "type": "stream_event", "event": { "type": "message_stop" } }),
        json!({ "type": "user", "message": { "role": "user", "content": [{
            "type": "tool_result",
            "tool_use_id": "toolu_1",
            "content": "<tool_use_error>Error: No such tool available: demi agent send parent</tool_use_error>",
            "is_error": true,
        }] } }),
        json!({ "type": "stream_event", "event": { "type": "message_start", "message": { "content": [] } } }),
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": "Corrected." } } }),
        call_end(),
        json!({ "type": "stream_event", "event": { "type": "message_stop" } }),
        done(),
    ])
    .await;
    assert_eq!(events, [ProviderEvent::TextDelta("Corrected.".into()), response()]);
    assert!(signals.is_empty());
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

#[tokio::test(flavor = "local")]
async fn a_call_the_vendor_refused_fails_the_run_once_with_the_code_of_its_http_status() {
    // The CLI makes up an assistant message for the refused call, which
    // repeats what its result reports.
    let refused = |status: u16, words: &str| {
        let message = format!("API Error: {status} {words}");
        vec![
            json!({ "type": "assistant", "error": "unknown", "message": {
                "model": "<synthetic>", "content": [{ "type": "text", "text": message }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": true, "result": message,
                "api_error_status": status }),
        ]
    };
    let (events, signals) = answer(refused(400, "The request is refused.")).await;
    let failed = failure(&events);
    assert_eq!(failed.message, "API Error: 400 The request is refused.");
    assert_eq!(failed.code, None, "a refused request is not retried");
    let status = failed
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.http_status);
    assert_eq!(status, Some(400));
    assert!(signals.is_empty());
    // A 401 refreshes the token and runs the turn again (refresh.rs).
    for (status, words, code) in [
        (
            400,
            "prompt is too long: 250000 tokens",
            ErrorCode::ContextLengthExceeded,
        ),
        (429, "Too many requests", ErrorCode::RateLimit),
        (529, "Overloaded", ErrorCode::Overloaded),
    ] {
        let (events, _) = answer(refused(status, words)).await;
        assert_eq!(failure(&events).code, Some(code), "{status} {words}");
    }
}

/// The CLI's own sign of a used-up account, `api_error:
/// usage_limit_reached` on its `result` line, is a usage limit whatever the
/// status, which the agent does not retry, and it waits until the reset its
/// refused call names on the assistant line the CLI makes up for it, never
/// the account's quota. A 429 the CLI says only throttles is a rate limit,
/// though its words mention a usage limit and its quota event says
/// rejected. The lines are the CLI 2.1.296's against a scripted vendor
/// (investigation 285).
#[tokio::test(flavor = "local")]
async fn a_usage_limit_is_the_clis_cause_and_waits_until_the_reset_its_refusal_names() {
    let provider = provider().await;
    let run = |lines: Vec<Value>| {
        let provider = &provider;
        async move {
            let (placement, mut starts) = ScriptedPlacement::new();
            let mut runtime = runtime_of(provider, &placement);
            let (events, ()) = tokio::join!(
                all_events(runtime.run(request_without_tools(vec![user("hi")]))),
                async {
                    let cli = starts.next().await;
                    for line in lines {
                        cli.say(line);
                    }
                }
            );
            events
        }
    };
    // The refused call's own reset, 10:00, and NOW is 08:00.
    let info = json!({ "status": "rejected", "resetsAt": 1790244000, "rateLimitType": "five_hour",
        "isUsingOverage": false });
    let session = |words: &str, info: Value| {
        let notice = json!({ "type": "assistant", "error": "rate_limit", "is_api_error_message": true,
            "api_error": "usage_limit_reached", "api_error_params": { "rate_limit_info": info },
            "message": { "model": "<synthetic>", "content": [{ "type": "text", "text": words }] } });
        let result = json!({ "type": "result", "subtype": "success", "is_error": true,
            "api_error_status": 429, "api_error": "usage_limit_reached", "result": words });
        (notice, result)
    };

    // An older quota reading names another used-up window, resetting at
    // 12:00; the refusal's own reset decides.
    let stale = json!({ "type": "rate_limit_event", "rate_limit_info": {
        "status": "rejected", "rateLimitType": "seven_day", "utilization": 1.0, "resetsAt": 1790251200 } });
    let (notice, result) = session("You've hit your session limit · resets 10am (UTC)", info.clone());
    let events = run(vec![stale, notice.clone(), result.clone()]).await;
    let failed = failure(&events);
    assert_eq!(failed.code, Some(ErrorCode::UsageLimit));
    assert_eq!(failed.retry_after, Some(std::time::Duration::from_secs(2 * 3600)));
    let diagnostics = failed.diagnostics.as_deref().unwrap();
    assert_eq!(diagnostics.upstream, Some(format!("{notice}\n{result}")));
    let facts = provider.read_failure(diagnostics, NOW.parse().unwrap());
    assert_eq!(facts.retry_at, Some("2026-09-24T10:00:00Z".parse().unwrap()));

    // Either the window or the usage credits lift the limit: the earlier
    // reset of those the refusal names, the credits' at 09:00 here, or the
    // credits' alone.
    let (notice, result) = session(
        "You're out of extra usage · resets 9am (UTC)",
        json!({ "status": "rejected", "resetsAt": 1790244000, "rateLimitType": "overage",
            "overageResetsAt": 1790240400, "isUsingOverage": false }),
    );
    let failed = failure(&run(vec![notice, result]).await).clone();
    assert_eq!(failed.retry_after, Some(std::time::Duration::from_secs(3600)));
    let (notice, result) = session(
        "You're out of extra usage · resets 9am (UTC)",
        json!({ "status": "rejected", "rateLimitType": "overage", "overageResetsAt": 1790240400,
            "isUsingOverage": false }),
    );
    let failed = failure(&run(vec![notice, result]).await).clone();
    assert_eq!(failed.retry_after, Some(std::time::Duration::from_secs(3600)));

    // Whatever the status says.
    let (notice, mut result) = session("You've hit your weekly limit", info);
    result.as_object_mut().unwrap().remove("api_error_status");
    let events = run(vec![notice, result]).await;
    assert_eq!(failure(&events).code, Some(ErrorCode::UsageLimit));

    // A member whose use its admin turned off is a usage limit too, and
    // names no reset: it waits for the user.
    let (notice, result) = session(
        "Your usage allocation has been disabled by your admin · ask your admin for a higher limit",
        json!({ "status": "rejected", "overageStatus": "rejected",
            "overageDisabledReason": "member_level_disabled", "isUsingOverage": false }),
    );
    let events = run(vec![notice, result]).await;
    let failed = failure(&events);
    assert_eq!(failed.code, Some(ErrorCode::UsageLimit));
    assert_eq!(failed.retry_after, None);
    let facts = provider.read_failure(failed.diagnostics.as_deref().unwrap(), NOW.parse().unwrap());
    assert_eq!(facts.retry_at, None);

    // A bare 429 only throttles, and is retried after the standard waits.
    let words = "API Error: Server is temporarily limiting requests (not your usage limit) · Synthetic refusal";
    let events = run(vec![
        json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "rejected", "isUsingOverage": false } }),
        json!({ "type": "assistant", "error": "rate_limit", "is_api_error_message": true,
            "message": { "model": "<synthetic>", "content": [{ "type": "text", "text": words }] } }),
        json!({ "type": "result", "subtype": "success", "is_error": true, "api_error_status": 429,
            "result": words }),
    ])
    .await;
    let failed = failure(&events);
    assert_eq!(failed.code, Some(ErrorCode::RateLimit));
    assert_eq!(failed.retry_after, None);
}
