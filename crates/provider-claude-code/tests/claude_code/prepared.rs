//! Processes prepared ahead of their request
//! (`crates/provider-claude-code/src/prepared.rs`).
//!
//! - The request that was known but for its input takes the prepared process as
//!   a kept one.
//! - Any other request closes it and starts its own.
//! - A prepared process goes after ten minutes unused.
//! - Its machine holds two at most.
//! - A preparation that fails or has no answered history to resume leaves the
//!   request as it would have been.
//!
//! Each test costs a few milliseconds; the one that waits ten minutes does so
//! on a paused clock.

use std::time::Duration;

use demi_host_interface::ProcessEnd;
use demi_provider_common::{Ahead, InferenceItem, InferenceRequest, ProviderEvent};
use demi_shared_types::{ThinkingConfig, TokenUsage};
use serde_json::{Value, json};

use crate::cli::*;

fn answer(text: &str) -> InferenceItem {
    InferenceItem::AssistantText {
        model_id: "claude-test".into(),
        text: text.into(),
    }
}

/// A history whose last turn was answered.
fn answered() -> Vec<InferenceItem> {
    vec![user("do work"), answer("done")]
}

/// The request the user's next message makes of it.
fn next() -> Vec<InferenceItem> {
    let mut items = answered();
    items.push(user("next question"));
    items
}

fn user_line(text: &str) -> Value {
    json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
}

fn usage(input: u64, output: u64) -> ProviderEvent {
    ProviderEvent::Response(Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }))
}

/// The messages of the session a process resumes, by role.
fn roles(cli: &Cli) -> Vec<String> {
    cli.messages()
        .iter()
        .map(|message| message["role"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test(flavor = "local")]
async fn the_request_a_process_was_prepared_for_takes_it_and_starts_none() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    runtime.prepare(request(answered()), Ahead::Opened).await;
    let mut cli = starts.next().await;
    // It resumes the answered history, declares its tools and waits; the
    // CLI's MCP handshake is answered meanwhile.
    assert_eq!(roles(&cli), ["user", "assistant"]);
    cli.initialized().await;
    cli.handshake().await;
    assert_eq!(placement.prepared_count(), 1);

    let (events, ()) = tokio::join!(all_events(runtime.run(request(next()))), async {
        assert_eq!(cli.take().await, user_line("next question"));
        cli.say(json!({
            "type": "stream_event",
            "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": "two" } },
        }));
        cli.result(1, 1);
    });
    assert_eq!(events, [ProviderEvent::TextDelta("two".into()), usage(1, 1)]);
    assert_eq!(placement.starts(), 1);
    assert!(cli.signals().is_empty());
    // Used, it is the runtime's kept process, no longer a prepared one.
    assert_eq!(placement.prepared_count(), 0);
}

#[tokio::test(flavor = "local")]
async fn a_request_with_another_model_effort_system_prompt_or_history_closes_the_prepared_process_and_starts_its_own()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let other_model = |request: InferenceRequest| InferenceRequest {
        model_id: "claude-other".into(),
        ..request
    };
    let other_effort = |request: InferenceRequest| InferenceRequest {
        thinking: Some(ThinkingConfig::Effort {
            effort: "high".into(),
            summary: None,
        }),
        ..request
    };
    let other_prompt = |request: InferenceRequest| InferenceRequest {
        system_prompt: "another system".into(),
        ..request
    };
    let edited = |request: InferenceRequest| {
        let mut items = request.items.to_vec();
        items[0] = user("do other work");
        InferenceRequest {
            items: items.into(),
            ..request
        }
    };
    let changes: [(&str, &dyn Fn(InferenceRequest) -> InferenceRequest); 4] = [
        ("model", &other_model),
        ("effort", &other_effort),
        ("system prompt", &other_prompt),
        ("history", &edited),
    ];
    for (changed, change) in changes {
        let mut runtime = runtime_of(&provider, &placement);
        runtime.prepare(request(answered()), Ahead::Opened).await;
        let mut prepared = starts.next().await;
        prepared.initialized().await;
        let (events, own) = tokio::join!(all_events(runtime.run(change(request(next())))), async {
            let mut own = starts.next().await;
            own.initialized().await;
            own.result(1, 1);
            own
        });
        assert_eq!(events, [usage(1, 1)], "{changed}");
        assert_eq!(roles(&own).last().map(String::as_str), Some("user"), "{changed}");
        // Closed, or, when it was still starting, its start given up.
        assert!(prepared.end().is_some(), "{changed}");
        assert_eq!(placement.prepared_count(), 0, "{changed}");
    }
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_prepared_process_unused_for_ten_minutes_is_closed() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    runtime.prepare(request(answered()), Ahead::Opened).await;
    let mut cli = starts.next().await;
    cli.initialized().await;
    tokio::time::advance(Duration::from_secs(10 * 60 - 1)).await;
    assert_eq!(cli.end(), None);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(cli.ended().await, ProcessEnd::Signalled("SIGTERM".into()));
    assert_eq!(placement.prepared_count(), 0);

    // The next request starts its own.
    let (events, _own) = tokio::join!(all_events(runtime.run(request(next()))), async {
        let mut own = starts.next().await;
        own.initialized().await;
        own.result(1, 1);
        own
    });
    assert_eq!(events, [usage(1, 1)]);
    assert_eq!(placement.starts(), 2);
}

#[tokio::test(flavor = "local")]
async fn a_machine_holds_two_prepared_processes_and_a_third_closes_the_oldest() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtimes = Vec::new();
    let mut clis = Vec::new();
    for session in ["session-1", "session-2", "session-3"] {
        let mut runtime = runtime_of(&provider, &placement);
        let prepared = InferenceRequest {
            session_id: session.into(),
            affinity: None,
            ..request(answered())
        };
        runtime.prepare(prepared, Ahead::Opened).await;
        let mut cli = starts.next().await;
        cli.initialized().await;
        runtimes.push(runtime);
        clis.push(cli);
    }
    assert!(matches!(clis[0].ended().await, ProcessEnd::Signalled(_)));
    assert_eq!(clis[1].end(), None);
    assert_eq!(clis[2].end(), None);
    assert_eq!(placement.prepared_count(), 2);
}

#[tokio::test(flavor = "local")]
async fn a_preparation_that_fails_leaves_the_request_to_start_its_own_process() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    placement.fail_next("Cloud capacity is currently full; retry later");
    runtime.prepare(request(answered()), Ahead::Opened).await;
    let (events, own) = tokio::join!(all_events(runtime.run(request(next()))), async {
        let mut own = starts.next().await;
        own.initialized().await;
        own.result(1, 1);
        own
    });
    assert_eq!(events, [usage(1, 1)]);
    // The request's own process resumes its input.
    assert_eq!(roles(&own), ["user", "assistant", "user"]);
    assert_eq!(placement.prepared_count(), 0);
}

#[tokio::test(flavor = "local")]
async fn a_history_that_ends_in_input_not_yet_answered_prepares_nothing() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    // A turn that failed left its message unanswered.
    let unanswered = vec![user("do work")];
    runtime
        .prepare(request(unanswered.clone()), Ahead::Opened)
        .await;
    assert_eq!(placement.prepared_count(), 0);
    let (events, _own) = tokio::join!(all_events(runtime.run(request(unanswered))), async {
        let mut own = starts.next().await;
        own.initialized().await;
        own.result(1, 1);
        own
    });
    assert_eq!(events, [usage(1, 1)]);
    assert_eq!(placement.starts(), 1);
}
