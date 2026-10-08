//! How a run starts, keeps and ends its CLI process (`claude-code.md` §
//! Requests over stream-json, § Process lifetime).

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use demi_host_interface::{Descriptor, ProcessEnd, Signal, SpawnEnv, SpawnRequest};
use demi_provider_claude_code::{ClaudeCodeConfig, ClaudeCodeProvider};
use demi_provider_common::credentials::MemoryCredentialPool;
use demi_provider_common::quota::MemorySnapshots;
use demi_provider_common::{
    ErrorCode, InferenceItem, InferenceRequest, MediaBytes, Medium, ProviderEvent, ProviderFailure,
    ProviderRun, UserPart,
};
use demi_shared_types::{B64Bytes, ThinkingConfig, TokenUsage};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::cli::*;

fn usage(input: u64, output: u64) -> ProviderEvent {
    ProviderEvent::Response(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    })
}

fn text(text: &str) -> ProviderEvent {
    ProviderEvent::TextDelta(text.into())
}

fn user_line(text: &str) -> Value {
    json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
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

#[tokio::test(flavor = "local")]
async fn a_new_process_starts_with_the_cli_contract_its_own_directory_and_the_token_on_a_descriptor()
{
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let request = InferenceRequest {
        thinking: Some(ThinkingConfig::Effort {
            effort: "high".into(),
            summary: None,
        }),
        ..request_without_tools(vec![user("hi")])
    };
    let (events, cli) = tokio::join!(all_events(runtime.run(request)), async {
        let mut cli = starts.next().await;
        assert_eq!(cli.read().await, user_line("hi"));
        cli.say(json!({ "type": "system", "subtype": "init", "tools": [] }));
        cli.text("hel");
        cli.text("lo");
        cli.say(json!({
            "type": "result",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 2,
                "cache_read_input_tokens": 7,
                "cache_creation_input_tokens": 3,
            },
        }));
        cli
    });
    let cached = ProviderEvent::Response(TokenUsage {
        input_tokens: 10,
        output_tokens: 2,
        cache_read_tokens: 7,
        cache_write_tokens: 3,
    });
    assert_eq!(events, [text("hel"), text("lo"), cached]);
    let args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--include-partial-messages",
        "--no-session-persistence",
        "--safe-mode",
        "--disable-slash-commands",
        "--tools",
        "",
        "--permission-mode",
        "bypassPermissions",
        "--allow-dangerously-skip-permissions",
        "--model",
        "claude-test",
        "--system-prompt",
        "system",
        "--effort",
        "high",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    // The access token is on descriptor 3, never in the environment, which
    // removes a token the machine's own environment holds; the process
    // claims to be no other client.
    let env = BTreeMap::from([
        ("CLAUDECODE".to_owned(), None),
        ("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), None),
        (
            "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR".to_owned(),
            Some("3".to_owned()),
        ),
        (
            "CLAUDE_CONFIG_DIR".to_owned(),
            Some(cli.config_dir().to_owned()),
        ),
        ("DISABLE_AUTOUPDATER".to_owned(), Some("1".to_owned())),
        ("DISABLE_AUTO_COMPACT".to_owned(), Some("1".to_owned())),
        (
            "MAX_MCP_OUTPUT_TOKENS".to_owned(),
            Some("1000000".to_owned()),
        ),
    ]);
    let expected = SpawnRequest {
        command: site().executable,
        args,
        cwd: Some(site().run_dir),
        env: SpawnEnv::Overlay(env),
        descriptors: vec![Descriptor {
            fd: 3,
            bytes: Bytes::from_static(TOKEN.as_bytes()),
        }],
        retained: true,
    };
    assert_eq!(cli.spawn, expected);
    // Kept for the next turn: never signalled, still running, with its
    // directory.
    assert!(cli.signals().is_empty());
    assert_eq!(cli.end(), None);
    assert!(!cli.config_removed());

    // Closing the runtime ends the process, and its directory goes.
    runtime.close().await;
    assert!(cli.end().is_some());
    assert!(cli.config_removed());
}

#[tokio::test(flavor = "local")]
async fn a_process_is_never_started_without_the_accounts_token_or_when_the_placement_fails() {
    // An account whose secret document is gone.
    let pool = MemoryCredentialPool::new();
    let signed_out = ClaudeCodeProvider::new(
        ClaudeCodeConfig::new("entry-1", "Claude", Some("cred-gone".into())),
        Arc::new(pool),
        Arc::new(MemorySnapshots::new()),
        models_dev_client("http://127.0.0.1:9/api.json"),
        reqwest::Client::new(),
        clock(),
    );
    let (placement, _starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&signed_out, &placement);
    let events = all_events(runtime.run(request(vec![user("hi")]))).await;
    assert_eq!(events.len(), 1);
    let refused = failure(&events[0]);
    assert_eq!(refused.code, Some(ErrorCode::AuthMissing));
    assert_eq!(refused.message, "No Claude Code account is signed in");
    assert_eq!(placement.starts(), 0);

    // A placement that cannot start the CLI says why, as a request that got
    // no answer, which the agent retries.
    let provider = provider().await;
    let mut runtime = runtime_of(&provider, &placement);
    placement.fail_next("Claude Code 2.1.3 could not be installed: failed to lookup address information");
    let events = all_events(runtime.run(request(vec![user("hi")]))).await;
    assert_eq!(events.len(), 1);
    let failed = failure(&events[0]);
    assert_eq!(
        failed.message,
        "Claude Code 2.1.3 could not be installed: failed to lookup address information"
    );
    assert_eq!(failed.code, Some(ErrorCode::Overloaded));
}

// 0.04 s. The run goes on its own thread and runtime: a
// run that spun on the line it kept would never yield to a guard in its own
// runtime, so the test waits for its answer from outside, and the deadline
// only guards against that spin.
#[test]
fn a_line_the_cli_prints_before_answering_initialize_is_read_by_the_run_after_the_answer() {
    let (answer, answered) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build_local(tokio::runtime::LocalOptions::default())
            .expect("a runtime for the run");
        let events = runtime.block_on(async {
            let provider = provider().await;
            let (placement, mut starts) = ScriptedPlacement::new();
            let mut runtime = runtime_of(&provider, &placement);
            let (events, ()) =
                tokio::join!(all_events(runtime.run(request(vec![user("hi")]))), async {
                    let mut cli = starts.next().await;
                    let initialize = cli.read().await;
                    // The CLI tells its commands before it answers.
                    cli.say(
                        json!({ "type": "system", "subtype": "commands_changed", "commands": [] }),
                    );
                    cli.say(json!({
                    "type": "control_response",
                    "response": { "subtype": "success", "request_id": initialize["request_id"] },
                }));
                    assert_eq!(cli.read().await, user_line("hi"));
                    cli.text("hello");
                    cli.result(1, 1);
                });
            events
        });
        // The test gave up waiting only when this run spun.
        let _ = answer.send(events);
    });
    let events = answered
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the run answered, instead of spinning on the line it kept");
    assert_eq!(events, [text("hello"), usage(1, 1)]);
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
        assert_eq!(cli.read().await, user_line("do work"));
        cli.text("one");
        // The last iteration is the last API call's usage, not the turn's
        // total.
        cli.say(json!({
            "type": "result",
            "usage": {
                "input_tokens": 30,
                "output_tokens": 300,
                "iterations": [
                    { "input_tokens": 10, "output_tokens": 100, "type": "message" },
                    { "input_tokens": 20, "output_tokens": 200, "type": "message" },
                ],
            },
        }));
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
        assert_eq!(cli.read().await, user_line("second question"));
        cli.text("two");
        cli.result(1, 1);
    });
    assert_eq!(events, [text("two"), usage(1, 1)]);
    assert!(cli.unread().is_empty());
    assert_eq!(placement.starts(), 1);
    assert!(cli.signals().is_empty());

    // An edit of the first message: the process is closed and a new one
    // receives the transcript as one message.
    let mut edited = second.clone();
    edited[0] = user("do other work");
    let (events, mut replayed) = tokio::join!(all_events(runtime.run(request(edited))), async {
        let mut replayed = starts.next().await;
        replayed.initialized().await;
        assert_eq!(
            replayed.read().await,
            user_line("User: do other work\n\nAssistant: one\n\nUser: second question")
        );
        replayed.result(1, 1);
        replayed
    });
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
            let mut other = starts.next().await;
            assert!(
                other
                    .spawn
                    .args
                    .windows(2)
                    .any(|pair| pair == ["--model", "claude-other"])
            );
            other.read().await;
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
            offered.read().await;
            offered.result(1, 1);
        }
    );
    assert_eq!(other.signals(), [Signal::Terminate]);
    assert_eq!(placement.starts(), 4);
    runtime.close().await;
}

#[tokio::test(flavor = "local")]
async fn a_new_process_receives_the_transcript_as_one_user_message_that_names_each_speaker() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let screenshot = UserPart::Image(Medium::Bytes(MediaBytes {
        data: B64Bytes::from(Bytes::from_static(b"png")),
        media_type: "image/png".into(),
    }));
    let items = vec![
        InferenceItem::UserMessage {
            content: vec![UserPart::Text("previous work".into()), screenshot],
        },
        InferenceItem::AssistantThinking {
            model_id: "claude-test".into(),
            text: "thinking".into(),
            signature: Some("signed".into()),
            kept_past_summary: false,
        },
        tool_use("tool-1", "pwd"),
        tool_result("tool-1", "/tmp"),
        user("continue"),
    ];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(items))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        // The image stays in its place; the reasoning is left out; the tool
        // call and its result are the model's own words.
        let transcript = json!({ "type": "user", "message": { "role": "user", "content": [
            { "type": "text", "text": "User: previous work" },
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "cG5n" } },
            { "type": "text", "text": concat!(
                "Assistant: [Earlier in this conversation I called the tool shell_exec with input: {\"script\":\"pwd\"}.",
                "\n\nIt returned from shell_exec: /tmp]\n\nUser: continue",
            ) },
        ] } });
        assert_eq!(cli.read().await, transcript);
        cli.result(3, 1);
        cli
    });
    assert_eq!(events, [usage(3, 1)]);
    assert!(cli.unread().is_empty());
}

#[tokio::test(flavor = "local")]
async fn a_process_that_ends_is_reported_by_its_status_and_the_next_request_starts_another() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);

    // Ended with success and no result: nothing to report.
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.exit(ProcessEnd::Exited(0));
        }
    );
    assert_eq!(events, []);

    // A nonzero exit is the tail of standard error, else its code.
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.stderr("the configuration home cannot be written\n");
            cli.exit(ProcessEnd::Exited(1));
        }
    );
    assert_eq!(
        failure(&events[0]).message,
        "the configuration home cannot be written"
    );
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.exit(ProcessEnd::Exited(2));
        }
    );
    assert_eq!(
        failure(&events[0]).message,
        "Claude Code exited with code 2"
    );

    // Only the last 64 KiB of standard error are kept.
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.stderr(&"x".repeat(100 * 1024));
            cli.stderr("end");
            cli.exit(ProcessEnd::Exited(1));
        }
    );
    let message = &failure(&events[0]).message;
    assert_eq!(message.len(), 64 * 1024);
    assert!(message.ends_with("xend"));
    assert_eq!(placement.starts(), 4);
}

#[tokio::test(flavor = "local")]
async fn a_process_that_ended_while_kept_is_replaced_by_one_that_replays_the_transcript() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (_, cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.result(1, 1);
            cli
        }
    );
    // The Cloud stopped with the process in it.
    cli.exit(ProcessEnd::Lost("runner disconnected".into()));
    let items = vec![user("hi"), user("again")];
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(items))),
        async {
            let mut replayed = starts.next().await;
            assert_eq!(
                replayed.read().await,
                json!({ "type": "user", "message": { "role": "user", "content": [
                { "type": "text", "text": "hi" },
                { "type": "text", "text": "again" },
            ] } })
            );
            replayed.result(2, 2);
        }
    );
    assert_eq!(events, [usage(2, 2)]);
    assert_eq!(placement.starts(), 2);
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
            let mut cli = starts.next().await;
            cli.read().await;
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
        let mut cli = starts.next().await;
        cli.read().await;
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
        let mut cli = starts.next().await;
        cli.ignore_terminate();
        cli.read().await;
        cancel.cancel();
        cli
    });
    assert_eq!(events, []);
    assert_eq!(cli.signals(), [Signal::Terminate, Signal::Kill]);
    assert_eq!(cli.end(), Some(ProcessEnd::Signalled("SIGKILL".into())));
    assert!(started.elapsed() >= std::time::Duration::from_secs(5));
}

#[tokio::test(flavor = "local")]
async fn dropping_a_run_or_a_runtime_kills_the_process_without_waiting() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    {
        let mut run = runtime.run(request_without_tools(vec![user("hi")]));
        let (first, mut cli) = tokio::join!(next_event(&mut run), async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.text("first");
            cli
        });
        assert_eq!(first, Some(text("first")));
        drop(run);
        assert!(cli.dropped());
        assert!(cli.unread().is_empty());
    }
    let (_, cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.result(1, 1);
            cli
        }
    );
    // A fresh runtime, as another session gets, starts a process of its own
    // and leaves this one's alone.
    let mut fresh = runtime.fresh();
    let (_, other) = tokio::join!(
        all_events(fresh.run(request_without_tools(vec![user("elsewhere")]))),
        async {
            let mut other = starts.next().await;
            assert_eq!(other.read().await, user_line("elsewhere"));
            other.result(1, 1);
            other
        }
    );
    assert!(!cli.dropped() && cli.signals().is_empty());
    drop(fresh);
    assert!(other.dropped() && !cli.dropped());
    drop(runtime);
    assert!(cli.dropped());
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
            let mut cli = starts.next().await;
            cli.read().await;
            cli.say(result.clone());
            cli
        }
    );
    // The failure is the run's last event; its usage is not a response.
    assert_eq!(events.len(), 1);
    let failed = failure(&events[0]);
    assert_eq!(failed.message, "context window exceeded\ninput is too long");
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
            assert_eq!(cli.read().await, user_line("smaller"));
            cli.say_text("{\"type\": \"result\", \"usage\": {\"input_tokens\": \"many\"}}\n");
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

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_retry_with_nothing_new_replays_the_transcript_in_a_new_process() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let items = vec![user("hi")];
    let (_, cli) = tokio::join!(
        as_the_agent_reads(runtime.run(request_without_tools(items.clone()))),
        async {
            let mut cli = starts.next().await;
            cli.read().await;
            cli.say(json!({ "type": "result", "is_error": true, "result": "overloaded" }));
            cli
        }
    );

    // The agent retries the same transcript: the kept process already holds
    // it and would wait for input that never comes, so a new one replays it.
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(items))),
        async {
            let mut replayed = starts.next().await;
            assert_eq!(replayed.read().await, user_line("hi"));
            replayed.result(1, 1);
        }
    );
    assert_eq!(events, [usage(1, 1)]);
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert_eq!(placement.starts(), 2);
}
