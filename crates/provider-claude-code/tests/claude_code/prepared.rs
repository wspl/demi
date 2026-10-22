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

use demi_provider_common::{Ahead, InferenceItem, InferenceRequest, ProviderEvent};
use demi_shared_types::{ThinkingConfig, TokenUsage};

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
