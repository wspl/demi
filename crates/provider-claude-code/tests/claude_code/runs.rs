//! How a run starts, keeps and ends its CLI process
//! (`crates/provider-claude-code/src/run.rs`,
//! `crates/provider-claude-code/src/live.rs`).

use std::collections::BTreeMap;

use demi_host_interface::{ProcessEnd, Signal, SpawnEnv, SpawnRequest};
use demi_provider_claude_code::CliWork;
use demi_provider_common::credentials::{CredentialPool as _, MemoryCredentialPool};
use demi_provider_common::{
    ErrorCode, InferenceItem, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun,
    ProviderRuntime,
};
use demi_shared_types::{ThinkingConfig, ThinkingSummary, TokenUsage};
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

/// A turn in which the model called two tools and then answered, as Claude
/// Code 2.1.296 printed it, with the tools named so that the request offers
/// neither and the CLI answers them itself. Its result's `usage` sums the
/// turn's three calls; with telemetry on, its `iterations` lists the last
/// call, and with `DISABLE_TELEMETRY` it lists none. Either way the
/// response carries the last call's usage, from its `message_delta`.
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

/// A `message_delta` that states only the output, as the Messages API may,
/// leaves the input counts its call's `message_start` stated.
#[tokio::test(flavor = "local")]
async fn a_calls_input_counts_its_message_delta_leaves_out_are_its_message_starts() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, _cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.say(json!({ "type": "stream_event", "event": { "type": "message_start", "message": {
                "usage": { "input_tokens": 11, "output_tokens": 1, "cache_read_input_tokens": 5, "cache_creation_input_tokens": 3 },
            } } }));
            cli.text("hello");
            cli.say(json!({ "type": "stream_event", "event": { "type": "message_delta", "usage": { "output_tokens": 7 } } }));
            cli.say(json!({ "type": "result", "subtype": "success", "usage": { "input_tokens": 99, "output_tokens": 99 } }));
            cli
        }
    );
    let counted = TokenUsage {
        input_tokens: 11,
        output_tokens: 7,
        cache_read_tokens: 5,
        cache_write_tokens: 3,
    };
    assert_eq!(events, [text("hello"), ProviderEvent::Response(Some(counted))]);
}

/// A turn whose stream holds no `message_delta` gives a response without
/// usage: the result's own usage sums the run's calls and never stands in.
#[tokio::test(flavor = "local")]
async fn a_turn_whose_stream_reports_no_calls_usage_gives_a_response_without_usage() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, _cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.text("hello");
            cli.say(json!({
                "type": "result",
                "subtype": "success",
                "usage": { "input_tokens": 6, "output_tokens": 113, "cache_read_input_tokens": 24401 },
            }));
            cli
        }
    );
    assert_eq!(events, [text("hello"), ProviderEvent::Response(None)]);
}

/// The CLI shows the model's thinking as the vendor's summary whenever
/// thinking is on, whatever the selection's summary says; with thinking
/// turned off it is asked for no display (`crates/provider-claude-code/src/run.rs`).
// Six runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn the_cli_streams_summarized_thinking_whenever_thinking_is_on() {
    let effort = |summary| ThinkingConfig::Effort {
        effort: "high".into(),
        summary,
    };
    for (thinking, display) in [
        (Some(effort(None)), Some("summarized")),
        (Some(effort(Some(ThinkingSummary::Off))), Some("summarized")),
        (None, Some("summarized")),
        (
            Some(ThinkingConfig::Adaptive {
                effort: "high".into(),
            }),
            Some("summarized"),
        ),
        (
            Some(ThinkingConfig::Budget {
                budget_tokens: 4_000,
            }),
            Some("summarized"),
        ),
        (Some(ThinkingConfig::Disabled {}), None),
    ] {
        let provider = provider().await;
        let (placement, mut starts) = ScriptedPlacement::new();
        let mut runtime = runtime_of(&provider, &placement);
        let request = InferenceRequest {
            thinking: thinking.clone(),
            ..request_without_tools(vec![user("hi")])
        };
        let (_, cli) = tokio::join!(all_events(runtime.run(request)), async {
            let cli = starts.next().await;
            cli.text("hello");
            cli.result(1, 1);
            cli
        });
        let args = &cli.spawn.args;
        let asked = args
            .iter()
            .position(|arg| arg == "--thinking-display")
            .map(|flag| args[flag + 1].as_str());
        assert_eq!(asked, display, "{thinking:?}");
        runtime.close().await;
    }
}

/// A turn of Claude Code 2.1.296 run with `--thinking-display summarized`
/// on Opus: the thinking block's `thinking_delta`s carry its summary, and
/// the whole message printed after them repeats it. The agent sees one
/// thinking block, its text streamed once, before the answer.
// One run of a recorded stream: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn summarized_thinking_streams_as_one_thinking_block_whose_text_comes_once() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, _cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            for line in include_str!("fixtures/summarized-thinking.jsonl").lines() {
                cli.say_text(&format!("{line}\n"));
            }
            cli
        }
    );
    let starts = events
        .iter()
        .filter(|event| matches!(event, ProviderEvent::ThinkingStart))
        .count();
    assert_eq!(starts, 1, "{events:?}");
    let first_text = events
        .iter()
        .position(|event| matches!(event, ProviderEvent::TextDelta(_)))
        .unwrap();
    let mut pieces = 0;
    let mut thinking = String::new();
    for (at, event) in events.iter().enumerate() {
        if let ProviderEvent::ThinkingDelta(piece) = event {
            assert!(at < first_text, "{events:?}");
            pieces += 1;
            thinking.push_str(piece);
        }
    }
    assert!(pieces > 1, "the summary streams in pieces: {events:?}");
    assert_eq!(
        thinking,
        "This is a simple question without any document request, so I can just answer it directly.\n\n"
    );
    let signatures = events
        .iter()
        .filter(|event| matches!(event, ProviderEvent::ThinkingSignature(_)))
        .count();
    assert_eq!(signatures, 1, "{events:?}");
    assert!(matches!(events.last(), Some(ProviderEvent::Response(Some(_)))), "{events:?}");
    runtime.close().await;
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

#[tokio::test(flavor = "local")]
async fn a_new_process_starts_with_the_cli_contract_in_its_accounts_directory_with_no_token() {
    let (provider, pool) = provider_with_pool().await;
    let account = pool.active().await.unwrap().unwrap();
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
        let cli = starts.next().await;
        cli.say(json!({ "type": "system", "subtype": "init", "tools": [] }));
        cli.text("hel");
        cli.text("lo");
        cli.say(json!({
            "type": "stream_event",
            "event": {
                "type": "message_delta",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 2,
                    "cache_read_input_tokens": 7,
                    "cache_creation_input_tokens": 3,
                },
            },
        }));
        cli.say(json!({ "type": "result", "usage": { "input_tokens": 10, "output_tokens": 2 } }));
        cli
    });
    let cached = ProviderEvent::Response(Some(TokenUsage {
        input_tokens: 10,
        output_tokens: 2,
        cache_read_tokens: 7,
        cache_write_tokens: 3,
    }));
    assert_eq!(events, [text("hel"), text("lo"), cached]);
    // The process resumes the session written for it, named by a UUID the
    // session's id gives, and mirrors it; safe mode keeps out what the
    // machine and the account would add, such as the account's claude.ai
    // connectors.
    let session = cli.session();
    let resumed = session[0]["sessionId"].as_str().unwrap().to_owned();
    assert!(uuid::Uuid::parse_str(&resumed).is_ok(), "{resumed}");
    let args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--include-partial-messages",
        "--replay-user-messages",
        "--resume",
        &resumed,
        "--session-mirror",
        "--system-prompt-snapshot",
        "off",
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
        "--thinking-display",
        "summarized",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    // The process is the account's: its directory is the CLI's
    // configuration directory, where the CLI keeps and renews the sign-in,
    // and nothing hands it a token, neither a descriptor nor the
    // environment, which removes any the machine's own holds. It claims to
    // be no other client, keeps its session in a directory of its own under
    // the account's and answers what the session ends with.
    assert_eq!(
        cli.work,
        CliWork::Account {
            account: account.clone(),
            handover: None
        }
    );
    assert_eq!(cli.config_dir(), format!("{ACCOUNTS}/{account}"));
    let env = BTreeMap::from([
        ("CLAUDECODE".to_owned(), None),
        ("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), None),
        ("CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR".to_owned(), None),
        (
            "CLAUDE_CONFIG_DIR".to_owned(),
            Some(cli.config_dir().to_owned()),
        ),
        (
            "CLAUDE_CODE_PROJECT_DIR_NAME".to_owned(),
            Some(cli.process_dir().to_owned()),
        ),
        (
            "CLAUDE_CODE_RESUME_INTERRUPTED_TURN".to_owned(),
            Some("1".to_owned()),
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
        descriptors: Vec::new(),
        retained: true,
    };
    assert_eq!(cli.spawn, expected);
    // The session is a file of the process's own directory.
    assert_eq!(cli.files[0].0, format!("{resumed}.jsonl"));
    // Kept for the next turn: never signalled, still running, with its
    // directory.
    assert!(cli.signals().is_empty());
    assert_eq!(cli.end(), None);
    assert!(!cli.config_removed());

    // Closing the runtime ends the process, and its own directory goes.
    runtime.close().await;
    assert!(cli.end().is_some());
    assert!(cli.config_removed());

    // Another process of the account starts in the same directory, with a
    // directory of its own under it.
    let (_, second) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("again")]))),
        async {
            let cli = starts.next().await;
            cli.result(1, 1);
            cli
        }
    );
    assert_eq!(second.config_dir(), format!("{ACCOUNTS}/{account}"));
    assert_ne!(second.process_dir(), cli.process_dir());
}

#[tokio::test(flavor = "local")]
async fn a_process_is_never_started_for_an_account_that_is_gone_or_when_the_placement_fails() {
    // An account removed meanwhile.
    let gone = provider_of(&MemoryCredentialPool::new(), "cred-gone");
    let (placement, _starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&gone, &placement);
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

/// The incident: two subagents' receipts written to a kept process, which
/// the CLI answers in two turns, each with its `result`. Both answers are
/// responses of the run that wrote the receipts, whole and in order, and the
/// next run starts clean in the same process. (The run ended at the first
/// `result`; the next one skipped the start of the second answer, showed its
/// rest and then its whole line again, and the answer to its own message
/// printed after it ended.)
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn messages_answered_in_turns_of_their_own_are_responses_of_the_run_that_wrote_them() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut cli = kept_after_first(&mut runtime, &mut starts).await;

    let mut receipts = answered_first();
    receipts.push(user("receipt A"));
    receipts.push(user("receipt B"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(receipts.clone()))), async {
        assert_eq!(cli.take().await, user_line("receipt A"));
        cli.text("answer ");
        cli.text("to A");
        cli.result(2, 2);
        assert_eq!(cli.take().await, user_line("receipt B"));
        cli.text("answer ");
        cli.text("to B");
        cli.result(3, 3);
    });
    assert_eq!(
        events,
        [
            text("answer "),
            text("to A"),
            usage(2, 2),
            text("answer "),
            text("to B"),
            usage(3, 3),
        ]
    );

    let mut next = receipts.clone();
    next.push(InferenceItem::AssistantText {
        model_id: "claude-test".into(),
        text: "answer to B".into(),
    });
    next.push(user("receipt C"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(next))), async {
        assert_eq!(cli.take().await, user_line("receipt C"));
        cli.text("the table");
        cli.result(4, 4);
    });
    assert_eq!(events, [text("the table"), usage(4, 4)]);
    assert!(cli.unread().is_empty());
    assert_eq!(placement.starts(), 1);
    assert!(cli.signals().is_empty());
}

/// Two messages the CLI takes into one turn are both printed back before
/// that turn's one `result`, which ends the run. (Fails with a run that
/// waits for a `result` for each message it wrote.)
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn messages_taken_into_one_turn_end_the_run_at_its_one_result() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut cli = kept_after_first(&mut runtime, &mut starts).await;

    let mut receipts = answered_first();
    receipts.push(user("receipt A"));
    receipts.push(user("receipt B"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(receipts.clone()))), async {
        cli.take().await;
        cli.take().await;
        cli.text("both");
        cli.result(2, 2);
    });
    assert_eq!(events, [text("both"), usage(2, 2)]);

    let mut next = receipts.clone();
    next.push(InferenceItem::AssistantText {
        model_id: "claude-test".into(),
        text: "both".into(),
    });
    next.push(user("next"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(next))), async {
        cli.take().await;
        cli.text("next answer");
        cli.result(3, 3);
    });
    assert_eq!(events, [text("next answer"), usage(3, 3)]);
    assert_eq!(placement.starts(), 1);
}

/// What the CLI prints for a message it answers itself, a command it does
/// not offer: its own whole answer and a `result`, with no message printed
/// back and no model call (Claude Code 2.1.296 and 2.1.301 with
/// `--disable-slash-commands`).
fn answer_locally(cli: &Cli, command: &str) {
    cli.say(json!({
        "type": "assistant",
        "message": { "model": "<synthetic>", "content": [
            { "type": "text", "text": format!("{command} isn't available in this environment.") },
        ] },
    }));
    cli.say(json!({ "type": "result", "subtype": "success", "is_error": false, "num_turns": 0 }));
}

/// A message the CLI answers itself, never printing it back, ends the run
/// at the `result` of that answer. (The run waited for its message to be
/// printed back, for good.)
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_message_the_cli_answers_itself_ends_the_run_at_its_result() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut cli = kept_after_first(&mut runtime, &mut starts).await;

    let mut items = answered_first();
    items.push(user("/cost"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items))), async {
        assert_eq!(cli.read().await, user_line("/cost"));
        answer_locally(&cli, "/cost");
    });
    assert!(
        matches!(events.as_slice(), [.., ProviderEvent::Response(None)]),
        "{events:?}"
    );
    assert!(events.contains(&text("/cost isn't available in this environment.")), "{events:?}");
    assert!(cli.signals().is_empty());
}

/// A message the CLI answers itself, written with one it takes into a turn
/// of the model's, gives both answers in the run, which ends at the second.
/// (The run ended at the first `result`, or waited for good.)
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_message_the_cli_answers_itself_and_one_for_the_model_give_both_answers_and_end() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut cli = kept_after_first(&mut runtime, &mut starts).await;

    let mut items = answered_first();
    items.push(user("/cost"));
    items.push(user("hello"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items))), async {
        assert_eq!(cli.read().await, user_line("/cost"));
        answer_locally(&cli, "/cost");
        assert_eq!(cli.take().await, user_line("hello"));
        cli.message_start();
        cli.text("hi");
        cli.result(2, 2);
    });
    let texts: Vec<&ProviderEvent> = events
        .iter()
        .filter(|event| matches!(event, ProviderEvent::TextDelta(_) | ProviderEvent::Response(_)))
        .collect();
    assert_eq!(
        texts,
        [
            &text("/cost isn't available in this environment."),
            &ProviderEvent::Response(None),
            &text("hi"),
            &usage(2, 2),
        ]
    );
    assert_eq!(placement.starts(), 1);
    assert!(cli.signals().is_empty());
}

/// A turn that fails while a message the run wrote is not printed back yet
/// fails the run and closes the process, which would otherwise answer that
/// message after the run ended. (The process was kept.)
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_turn_that_fails_before_every_message_was_taken_fails_the_run_and_closes_the_process() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut cli = kept_after_first(&mut runtime, &mut starts).await;

    let mut items = answered_first();
    items.push(user("receipt A"));
    items.push(user("receipt B"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items))), async {
        cli.take().await;
        cli.message_start();
        cli.say(json!({ "type": "result", "is_error": true, "result": "overloaded" }));
    });
    assert!(
        matches!(events.as_slice(), [.., ProviderEvent::Error(_)]),
        "{events:?}"
    );
    assert_eq!(cli.signals(), [Signal::Terminate]);
}

/// A kept process that printed output after the `result` that ended its
/// last run went on without a request: the next run closes it, and a new
/// process resumes the transcript, so none of that output is shown. (The
/// run skipped it and wrote to the process that printed it.)
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn output_waiting_before_a_run_writes_closes_the_process() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, cli) = tokio::join!(all_events(runtime.run(request(vec![user("first")]))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.text("one");
        cli.result(1, 1);
        // A turn nobody asked for, printed with the first: an assistant
        // message that answers no message the run wrote.
        cli.say(json!({
            "type": "assistant",
            "message": {
                "id": "msg_stray", "type": "message", "role": "assistant", "model": "claude-test",
                "content": [{ "type": "text", "text": "stray" }], "stop_reason": "end_turn",
            },
        }));
        cli.result(9, 9);
        cli
    });
    assert_eq!(events, [text("one"), usage(1, 1)]);

    let mut second = answered_first();
    second.push(user("second"));
    let (events, mut resumed) = tokio::join!(all_events(runtime.run(request(second))), async {
        let mut resumed = starts.next().await;
        resumed.initialized().await;
        resumed.text("two");
        resumed.result(2, 2);
        resumed
    });
    assert_eq!(events, [text("two"), usage(2, 2)]);
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert_eq!(placement.starts(), 2);
    assert!(resumed.unread().is_empty());
}

/// A kept process prints status lines between requests, such as the
/// `command_lifecycle` line when a command ends; they answer no message, so
/// the next run reads past them and writes to the same process. (Every one
/// closed the process and rebuilt the session.)
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_status_line_between_requests_keeps_the_process() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(vec![user("first")]))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.text("one");
        cli.result(1, 1);
        cli.say(json!({
            "type": "system", "subtype": "command_lifecycle", "status": "completed",
            "session_id": "43a99fd4-6db6-4418-9922-b8f39b7a7f5c",
        }));
        cli
    });
    assert_eq!(events, [text("one"), usage(1, 1)]);

    let mut second = answered_first();
    second.push(user("second"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second))), async {
        cli.take().await;
        cli.text("two");
        cli.result(2, 2);
    });
    assert_eq!(events, [text("two"), usage(2, 2)]);
    assert_eq!(placement.starts(), 1);
    assert!(cli.signals().is_empty());
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

#[tokio::test(flavor = "local")]
async fn a_process_that_ends_is_reported_by_its_status_and_the_next_request_starts_another() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);

    // Ended with success but no result line: the reply never completed.
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.exit(ProcessEnd::Exited(0));
        }
    );
    assert_eq!(
        (failure(&events[0]).message.as_str(), &failure(&events[0]).code),
        ("The provider's stream ended before the reply was complete", &Some(demi_provider_common::ErrorCode::Overloaded))
    );

    // A nonzero exit is the tail of standard error, else its code.
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
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
            let cli = starts.next().await;
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
            let cli = starts.next().await;
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
async fn a_process_that_ended_while_kept_is_replaced_by_one_that_resumes_the_transcript() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (_, cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.result(1, 1);
            cli
        }
    );
    // The Cloud stopped with the process in it.
    cli.exit(ProcessEnd::Lost("runner disconnected".into()));
    let items = vec![user("hi"), user("again")];
    let (events, replayed) = tokio::join!(
        all_events(runtime.run(request_without_tools(items))),
        async {
            let replayed = starts.next().await;
            replayed.result(2, 2);
            replayed
        }
    );
    assert_eq!(events, [usage(2, 2)]);
    assert_eq!(replayed.messages(), [user_message("hi"), user_message("again")]);
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

// A fixed bug: the machine went away while the CLI wrote a long line, and
// the half of it that arrived failed the run as a line Demi cannot read.
#[tokio::test(flavor = "local")]
async fn output_cut_in_the_middle_of_a_line_by_a_machine_that_went_away_is_retried() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, ()) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.say_text(r#"{"type":"user","message":{"content":[{"type":"text","text":"iVBORw0KGgo"#);
            cli.exit(ProcessEnd::Lost("replaced by a new connection of the device's runner".into()));
        }
    );
    let lost = failure(&events[0]);
    assert_eq!(lost.code, Some(ErrorCode::Overloaded), "{}", lost.message);
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

#[tokio::test(flavor = "local")]
async fn dropping_a_run_or_a_runtime_closes_the_process_sigterm_first_without_waiting() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    {
        let mut run = runtime.run(request_without_tools(vec![user("hi")]));
        let (first, mut cli) = tokio::join!(next_event(&mut run), async {
            let cli = starts.next().await;
            cli.text("first");
            cli
        });
        assert_eq!(first, Some(text("first")));
        assert_eq!(placement.leases(), 1);
        drop(run);
        // The action, not the run, holds the machine.
        assert_eq!(placement.leases(), 1);
        // The turn dropped the run's stream: the process is asked to end,
        // never killed outright, since a kill while it renews the account's
        // sign-in stops the account for a minute.
        cli.ended().await;
        assert_eq!(cli.signals(), [Signal::Terminate]);
        assert!(!cli.dropped());
        assert!(cli.unread().is_empty());
    }
    let (_, cli) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
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
            let other = starts.next().await;
            other.result(1, 1);
            other
        }
    );
    assert!(cli.signals().is_empty());
    drop(fresh);
    other.ended().await;
    assert_eq!(other.signals(), [Signal::Terminate]);
    assert!(cli.signals().is_empty());
    drop(runtime);
    cli.ended().await;
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert!(!cli.dropped());
    assert_eq!(placement.leases(), 0);
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

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_retry_with_nothing_new_resumes_the_transcript_in_a_new_process() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let items = vec![user("hi")];
    let (_, cli) = tokio::join!(
        as_the_agent_reads(runtime.run(request_without_tools(items.clone()))),
        async {
            let cli = starts.next().await;
            cli.say(json!({ "type": "result", "is_error": true, "result": "overloaded" }));
            cli
        }
    );

    // The agent retries the same transcript: the kept process already holds
    // it and would wait for input that never comes, so a new one resumes it.
    let (events, replayed) = tokio::join!(
        all_events(runtime.run(request_without_tools(items))),
        async {
            let replayed = starts.next().await;
            replayed.result(1, 1);
            replayed
        }
    );
    assert_eq!(events, [usage(1, 1)]);
    assert_eq!(replayed.messages(), [user_message("hi")]);
    assert_eq!(cli.signals(), [Signal::Terminate]);
    assert_eq!(placement.starts(), 2);
}
