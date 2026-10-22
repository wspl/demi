//! What the provider reads of the CLI's lines
//! (`crates/provider-claude-code/src/output.rs`,
//! `crates/provider-common/src/wire/mod.rs`).
//!
//! The tests cover the events of streamed and whole messages, the lines and
//! blocks it skips, and the failures of lines it cannot read.

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

/// The CLI's own sign of a used-up account, `api_error: usage_limit_reached` on
/// its `result` line, is a usage limit whatever the status.
///
/// - The agent does not retry it.
/// - It waits until the reset its refused call names on the assistant line the
///   CLI makes up for it, never the account's quota.
/// - A 429 the CLI says only throttles is a rate limit, though its words
///   mention a usage limit and its quota event says rejected.
///
/// The lines are the CLI 2.1.296's against a scripted vendor (investigation
/// 285).
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
