//! How a run starts, keeps and ends its CLI process
//! (`crates/provider-claude-code/src/run.rs`,
//! `crates/provider-claude-code/src/live.rs`).

use demi_host_interface::{ProcessEnd, Signal};
use demi_provider_common::{
    ErrorCode, InferenceItem, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun,
    ProviderRuntime,
};
use demi_shared_types::TokenUsage;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::cli::*;

fn usage(input: u64, output: u64) -> ProviderEvent {
    ProviderEvent::Response(Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }))
}

/// A turn in which the model called two tools and then answered, as Claude Code
/// 2.1.296 printed it.
///
/// The tools are named so that the request offers neither and the CLI answers
/// them itself. Its result's `usage` sums the turn's three calls. With
/// telemetry on, its `iterations` lists the last call, and with
/// `DISABLE_TELEMETRY` it lists none. Either way the response carries the last
/// call's usage, from its `message_delta`.
// Two runs of a recorded stream: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_turn_of_several_calls_reports_its_last_calls_usage_never_their_sum() {
    for (stream, last) in [
        (
            include_str!("fixtures/two-tool-calls.jsonl"),
            TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
                cache_read_tokens: 4_744,
                cache_write_tokens: 127,
            },
        ),
        (
            include_str!("fixtures/two-tool-calls-without-telemetry.jsonl"),
            TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
                cache_read_tokens: 23_349,
                cache_write_tokens: 13_950,
            },
        ),
    ] {
        let provider = provider().await;
        let (placement, mut starts) = ScriptedPlacement::new();
        let mut runtime = runtime_of(&provider, &placement);
        let (events, _cli) = tokio::join!(
            all_events(runtime.run(request_without_tools(vec![user("hi")]))),
            async {
                let cli = starts.next().await;
                for line in stream.lines() {
                    cli.say_text(&format!("{line}\n"));
                }
                cli
            }
        );
        assert_eq!(events.last(), Some(&ProviderEvent::Response(Some(last))), "{events:?}");
        assert!(!events.iter().any(|event| matches!(event, ProviderEvent::Error(_))));
    }
}

fn text(text: &str) -> ProviderEvent {
    ProviderEvent::TextDelta(text.into())
}

fn user_message(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

fn user_line(text: &str) -> Value {
    json!({ "type": "user", "message": user_message(text) })
}

/// A message of another provider's turn, in the CLI's format: the `index`
/// of its first entry names it.
fn assistant_message(index: usize, content: Value) -> Value {
    json!({
        "id": format!("msg_demi_{index}"),
        "type": "message",
        "role": "assistant",
        "model": "claude-test",
        "content": [content],
    })
}

fn failure(event: &ProviderEvent) -> &ProviderFailure {
    match event {
        ProviderEvent::Error(failure) => failure,
        other => panic!("expected a failure, got {other:?}"),
    }
}

/// The events of `run` to its first failure, read as the agent reads a run:
/// it drops the run at the failure.
async fn as_the_agent_reads(mut run: ProviderRun<'_>) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    while let Some(event) = next_event(&mut run).await {
        let failed = matches!(event, ProviderEvent::Error(_));
        events.push(event);
        if failed {
            break;
        }
    }
    events
}

/// What a kept process answered `first` with, and the transcript after it.
fn answered_first() -> Vec<InferenceItem> {
    vec![
        user("first"),
        InferenceItem::AssistantText {
            model_id: "claude-test".into(),
            text: "one".into(),
        },
    ]
}

/// A process kept after it answered `first`.
async fn kept_after_first(runtime: &mut Box<dyn ProviderRuntime>, starts: &mut Starts) -> Cli {
    let (events, cli) = tokio::join!(all_events(runtime.run(request(vec![user("first")]))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.text("one");
        cli.result(1, 1);
        cli
    });
    assert_eq!(events, [text("one"), usage(1, 1)]);
    cli
}

/// Three receipts written at once: the CLI answers the first in a turn of its
/// own, then merges the other two into one turn. The run ends at the merged
/// turn's `result`, which names the last receipt. (A run that counted the
/// messages the CLI printed back waited for good: it printed the merged turn
/// back once.)
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_batch_merged_after_an_earlier_turn_ends_the_run_at_the_result_that_names_its_last_message() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut cli = kept_after_first(&mut runtime, &mut starts).await;

    let mut receipts = answered_first();
    receipts.push(user("receipt A"));
    receipts.push(user("receipt B"));
    receipts.push(user("receipt C"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(receipts))), async {
        assert_eq!(cli.take().await, user_line("receipt A"));
        cli.text("A is in");
        cli.result(2, 2);
        assert_eq!(cli.take().await, user_line("receipt B"));
        assert_eq!(cli.take().await, user_line("receipt C"));
        cli.text("B and C are in");
        cli.result(3, 3);
    });
    assert_eq!(events, [text("A is in"), usage(2, 2), text("B and C are in"), usage(3, 3)]);
    assert!(cli.unread().is_empty());
    assert_eq!(placement.starts(), 1);
}

#[tokio::test(flavor = "local")]
async fn a_kept_process_receives_only_what_the_transcript_gained_until_an_edit_another_model_or_new_tools_restart_it()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("do work")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.text("one");
        cli.result(20, 200);
        cli
    });
    assert_eq!(events, [text("one"), usage(20, 200)]);

    // The next turn: only the new user message is written.
    let mut second = first.clone();
    second.push(InferenceItem::AssistantText {
        model_id: "claude-test".into(),
        text: "one".into(),
    });
    second.push(user("second question"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second.clone()))), async {
        assert_eq!(cli.take().await, user_line("second question"));
        cli.text("two");
        cli.result(1, 1);
    });
    assert_eq!(events, [text("two"), usage(1, 1)]);
    assert!(cli.unread().is_empty());
    assert_eq!(placement.starts(), 1);
    assert!(cli.signals().is_empty());

    // An edit of the first message: the process is closed and a new one
    // resumes the edited transcript.
    let mut edited = second.clone();
    edited[0] = user("do other work");
    let (events, mut replayed) = tokio::join!(all_events(runtime.run(request(edited))), async {
        let mut replayed = starts.next().await;
        replayed.initialized().await;
        replayed.result(1, 1);
        replayed
    });
    assert_eq!(
        replayed.messages(),
        [
            user_message("do other work"),
            assistant_message(1, json!({ "type": "text", "text": "one" })),
            user_message("second question"),
        ]
    );
    assert!(replayed.unread().is_empty());
    assert_eq!(events, [usage(1, 1)]);
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert_eq!(cli.end(), Some(ProcessEnd::Signalled("SIGTERM".into())));

    // Another model needs another process too.
    let other_model = |request: InferenceRequest| InferenceRequest {
        model_id: "claude-other".into(),
        ..request
    };
    let (_, other) = tokio::join!(
        all_events(runtime.run(other_model(request_without_tools(second.clone())))),
        async {
            let other = starts.next().await;
            assert!(
                other
                    .spawn
                    .args
                    .windows(2)
                    .any(|pair| pair == ["--model", "claude-other"])
            );
            other.result(1, 1);
            other
        }
    );
    assert_eq!(replayed.signals(), [Signal::Terminate]);

    // A process started without tools has no MCP server to offer them in.
    let (_, ()) = tokio::join!(
        all_events(runtime.run(other_model(request(second.clone())))),
        async {
            let mut offered = starts.next().await;
            offered.initialized().await;
            offered.result(1, 1);
        }
    );
    assert_eq!(other.signals(), [Signal::Terminate]);
    assert_eq!(placement.starts(), 4);
    runtime.close().await;
}

// A fixed bug: a subagent's request failed for good when its Cloud's runner
// reconnected under it.
#[tokio::test(flavor = "local")]
async fn a_run_whose_machine_went_away_got_no_answer_and_is_retried() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.exit(ProcessEnd::Lost("replaced by a new connection of the device's runner".into()));
        }
    );
    let lost = failure(&events[0]);
    assert_eq!(
        lost.message,
        "Claude Code's machine went away: replaced by a new connection of the device's runner"
    );
    assert_eq!(lost.code, Some(ErrorCode::Overloaded));
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_cancelled_run_closes_its_process_and_ends_without_an_event() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let cancel = CancellationToken::new();
    let cancelled = InferenceRequest {
        cancel: cancel.clone(),
        ..request_without_tools(vec![user("hi")])
    };
    let mut run = runtime.run(cancelled);
    let (first, cli) = tokio::join!(next_event(&mut run), async {
        let cli = starts.next().await;
        cli.text("partial");
        cli
    });
    assert_eq!(first, Some(text("partial")));
    cancel.cancel();
    assert_eq!(next_event(&mut run).await, None);
    drop(run);
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert!(!cli.dropped());

    // A process that ignores SIGTERM is killed after five seconds.
    let cancel = CancellationToken::new();
    let cancelled = InferenceRequest {
        cancel: cancel.clone(),
        ..request_without_tools(vec![user("hi")])
    };
    let started = tokio::time::Instant::now();
    let (events, cli) = tokio::join!(all_events(runtime.run(cancelled)), async {
        let cli = starts.next().await;
        cli.ignore_terminate();
        cancel.cancel();
        cli
    });
    assert_eq!(events, []);
    assert_eq!(cli.signals(), [Signal::Terminate, Signal::Kill]);
    assert_eq!(cli.end(), Some(ProcessEnd::Signalled("SIGKILL".into())));
    assert!(started.elapsed() >= std::time::Duration::from_secs(5));
}

// A few milliseconds: three turns of the scripted CLI.
#[tokio::test(flavor = "local")]
async fn an_action_holds_the_machine_from_its_first_request_to_its_end() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut kept: Option<Cli> = None;
    let mut history = Vec::new();
    for message in ["one", "two"] {
        history.push(user(message));
        let (_, cli) = tokio::join!(
            all_events(runtime.run(request_without_tools(history.clone()))),
            async {
                let cli = match kept.take() {
                    Some(mut cli) => {
                        cli.take().await;
                        cli
                    }
                    None => starts.next().await,
                };
                cli.result(1, 1);
                cli
            }
        );
        kept = Some(cli);
        // Between the action's requests, as while its tools run, the
        // machine stays held, once for the whole action.
        assert_eq!(placement.leases(), 1);
    }
    // The action ended, completed, failed or stopped: the machine is let go,
    // though the runtime keeps its process for the next one.
    runtime.idle();
    assert_eq!(placement.leases(), 0);
    let cli = kept.unwrap();
    assert!(cli.signals().is_empty());
    // The next action holds it again, until the runtime closes.
    let (_, cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("one"), user("two"), user("three")]))),
        async {
            let mut cli = cli;
            cli.take().await;
            cli.result(1, 1);
            cli
        }
    );
    assert_eq!(placement.leases(), 1);
    runtime.close().await;
    assert_eq!(placement.leases(), 0);
    assert!(cli.end().is_some());
}

#[tokio::test(flavor = "local")]
async fn the_clis_error_ends_the_turn_and_keeps_the_process_while_a_broken_line_closes_it() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let result = json!({
        "type": "result",
        "is_error": true,
        "result": "context window exceeded",
        "errors": ["input is too long"],
        "usage": { "input_tokens": 200000, "output_tokens": 0 },
    });
    let (events, mut cli) = tokio::join!(
        as_the_agent_reads(runtime.run(request_without_tools(vec![user("huge")]))),
        async {
            let cli = starts.next().await;
            cli.say(result.clone());
            cli
        }
    );
    // The failure is the run's last event; its usage is not a response.
    assert_eq!(events.len(), 1);
    let failed = failure(&events[0]);
    assert_eq!(failed.code, Some(ErrorCode::ContextLengthExceeded));
    let record = failed
        .diagnostics
        .as_ref()
        .unwrap()
        .upstream
        .as_deref()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(record).unwrap(), result);
    assert!(cli.signals().is_empty());

    // The next request goes on in the same process.
    let items = vec![user("huge"), user("smaller")];
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(items))),
        async {
            assert_eq!(cli.take().await, user_line("smaller"));
            cli.say_text("{\"type\": \"stream_event\", \"event\": {\"type\": \"message_delta\", \"usage\": {\"input_tokens\": \"many\"}}}\n");
        }
    );
    let broken = failure(&events[0]);
    assert!(
        broken
            .message
            .starts_with("Claude Code sent a line Demi cannot read"),
        "{}",
        broken.message
    );
    assert_eq!(broken.code, None);
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert_eq!(placement.starts(), 1);
}
