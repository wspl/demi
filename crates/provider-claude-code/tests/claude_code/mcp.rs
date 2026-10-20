//! The SDK MCP channel and the model's tool batches (`crates/provider-claude-code/src/mcp.rs`).

use bytes::Bytes;
use demi_host_interface::Signal;
use demi_provider_common::testing::property_order;
use demi_provider_common::{InferenceItem, MediaBytes, ProviderEvent, ResultPart, ToolCall};
use demi_shared_types::{B64Bytes, TokenUsage};
use serde_json::json;

use crate::cli::*;

fn call(id: &str, script: &str) -> ProviderEvent {
    ProviderEvent::ToolCall(ToolCall {
        tool_use_id: id.into(),
        tool_name: "shell_exec".into(),
        input: json!({ "script": script }),
    })
}

fn usage(input: u64, output: u64) -> ProviderEvent {
    ProviderEvent::Response(Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }))
}

#[tokio::test(flavor = "local")]
async fn the_server_lists_the_requests_tools_and_a_call_before_any_streaming_is_a_batch_of_one() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("hi")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        let initialize = cli.initialized().await;
        assert_eq!(
            initialize["request"],
            json!({ "subtype": "initialize", "sdkMcpServers": ["main"], "systemPrompt": "system" })
        );
        cli.handshake().await;
        cli.mcp(
            "list",
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        );
        let listed = cli.mcp_reply("list").await;
        let tool = shell_exec();
        assert_eq!(
            listed["result"]["tools"],
            json!([{
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            }])
        );
        assert_eq!(
            property_order(&listed["result"]["tools"][0]["inputSchema"]),
            ["description", "script", "intervalMs"]
        );
        cli.mcp(
            "ping",
            json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }),
        );
        assert_eq!(cli.mcp_reply("ping").await["result"], json!({}));
        cli.call("call-1", 3, "toolu_1", "pwd");
        cli
    });
    // A call before any streaming has no call's usage to end with.
    assert_eq!(events, [call("toolu_1", "pwd"), ProviderEvent::Response(None)]);
    // The call waits for its result.
    assert!(cli.unread().is_empty());

    let mut second = first.clone();
    second.push(tool_use("toolu_1", "pwd"));
    second.push(tool_result("toolu_1", "/tmp"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second))), async {
        let reply = cli.mcp_reply("call-1").await;
        assert_eq!(
            reply,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": { "content": [{ "type": "text", "text": "/tmp" }], "isError": false },
            })
        );
        cli.text("after the tool");
        cli.result(2, 4);
    });
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("after the tool".into()),
            usage(2, 4)
        ]
    );
    assert!(cli.signals().is_empty());
}

#[tokio::test(flavor = "local")]
async fn a_whole_batch_reaches_the_agent_before_any_answer_and_a_later_call_is_answered_from_its_stored_result()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("run both")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        cli.message_start();
        cli.text("running both");
        // Each call is reported as the model writes it, before the batch.
        cli.streamed_tool_use(1, "toolu_alpha", &["{\"script\": ", "\"printf alpha\"}"]);
        cli.tool_use("toolu_alpha", "printf alpha");
        // The CLI runs a tool as soon as its block is whole, before the
        // message ends; the second call stays blocked behind the first.
        cli.call("call-alpha", 2, "toolu_alpha", "printf alpha");
        cli.streamed_tool_use(2, "toolu_beta", &["{\"script\": \"printf beta\"}"]);
        cli.tool_use("toolu_beta", "printf beta");
        cli.message_stop();
        cli
    });
    let start = |id: &str| ProviderEvent::ToolCallStart {
        tool_use_id: id.into(),
        tool_name: "shell_exec".into(),
    };
    let input = |id: &str, piece: &str| ProviderEvent::ToolCallInput {
        tool_use_id: id.into(),
        partial_json: piece.into(),
    };
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("running both".into()),
            start("toolu_alpha"),
            input("toolu_alpha", "{\"script\": "),
            input("toolu_alpha", "\"printf alpha\"}"),
            start("toolu_beta"),
            input("toolu_beta", "{\"script\": \"printf beta\"}"),
            call("toolu_alpha", "printf alpha"),
            call("toolu_beta", "printf beta"),
            ProviderEvent::Response(None),
        ]
    );
    assert!(
        cli.unread().is_empty(),
        "no call is answered before the batch's results"
    );

    let mut second = first.clone();
    second.push(tool_use("toolu_alpha", "printf alpha"));
    second.push(tool_use("toolu_beta", "printf beta"));
    second.push(tool_result("toolu_alpha", "alpha done"));
    second.push(tool_result("toolu_beta", "beta done"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second))), async {
        let alpha = cli.mcp_reply("call-alpha").await;
        assert_eq!(
            alpha["result"]["content"],
            json!([{ "type": "text", "text": "alpha done" }])
        );
        // Only after the first answer does the CLI send the second call,
        // which is answered at once.
        // The CLI may reuse a JSON-RPC id once its request was answered; the
        // answer still goes back in the control request that carried it.
        cli.call("call-beta", 2, "toolu_beta", "printf beta");
        let beta = cli.mcp_reply("call-beta").await;
        assert_eq!(
            beta["result"]["content"],
            json!([{ "type": "text", "text": "beta done" }])
        );
        cli.result(3, 5);
    });
    assert_eq!(events, [usage(3, 5)]);
    assert_eq!(placement.starts(), 1);
}

/// A Claude Code turn of several tool calls is several runs, one per batch.
///
/// Each run that stops at a batch ends with the usage of the call that wrote
/// it, so the agent can compact at the call that crossed its threshold rather
/// than at the turn's `result`.
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn each_run_that_stops_at_a_batch_ends_with_the_usage_of_the_call_that_wrote_it() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    // The CLI's call `n` writes one tool use and reads `input` tokens.
    let write_call = |cli: &Cli, n: i64, input: u64| {
        let id = format!("toolu_{n}");
        cli.message_start();
        cli.streamed_tool_use(0, &id, &["{\"script\": \"pwd\"}"]);
        cli.tool_use(&id, "pwd");
        cli.call(&format!("call-{n}"), n, &id, "pwd");
        cli.message_delta(input, 10);
        cli.message_stop();
    };
    let mut items = vec![user("work")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(items.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        write_call(&cli, 1, 100_000);
        cli
    });
    assert_eq!(events.last(), Some(&usage(100_000, 10)), "{events:?}");

    items.push(tool_use("toolu_1", "pwd"));
    items.push(tool_result("toolu_1", "/tmp"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items.clone()))), async {
        cli.mcp_reply("call-1").await;
        write_call(&cli, 2, 270_000);
    });
    assert_eq!(events.last(), Some(&usage(270_000, 10)), "{events:?}");

    items.push(tool_use("toolu_2", "pwd"));
    items.push(tool_result("toolu_2", "/tmp"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items))), async {
        cli.mcp_reply("call-2").await;
        cli.text("done");
        cli.result(280_000, 5);
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("done".into()), usage(280_000, 5)]
    );
    assert_eq!(placement.starts(), 1);
}

/// The batch's run reported its call's usage; a `result` after it whose
/// stream brought no call's counts reports none, never the batch's again,
/// which would count that call twice in the usage ledger.
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_result_with_no_new_calls_counts_never_reports_the_batchs_usage_again() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut items = vec![user("work")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(items.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        cli.message_start();
        cli.streamed_tool_use(0, "toolu_1", &["{\"script\": \"pwd\"}"]);
        cli.tool_use("toolu_1", "pwd");
        cli.call("call-1", 1, "toolu_1", "pwd");
        cli.message_delta(100_000, 10);
        cli.message_stop();
        cli
    });
    assert_eq!(events.last(), Some(&usage(100_000, 10)), "{events:?}");

    items.push(tool_use("toolu_1", "pwd"));
    items.push(tool_result("toolu_1", "/tmp"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items))), async {
        cli.mcp_reply("call-1").await;
        cli.say(json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "usage": { "input_tokens": 100_000, "output_tokens": 10 },
        }));
    });
    assert_eq!(events, [ProviderEvent::Response(None)]);
}

#[tokio::test(flavor = "local")]
async fn a_call_of_a_tool_the_request_does_not_offer_is_left_out_of_the_batch_for_the_cli_to_answer()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("run both")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        cli.message_start();
        cli.streamed_tool_use(1, "toolu_alpha", &["{\"script\": \"pwd\"}"]);
        cli.tool_use("toolu_alpha", "pwd");
        cli.call("call-alpha", 2, "toolu_alpha", "pwd");
        // The model names a tool the request does not offer, which the CLI
        // answers itself with an error, never through the server.
        cli.say(json!({ "type": "stream_event", "event": {
            "type": "content_block_start",
            "index": 2,
            "content_block": { "type": "tool_use", "id": "toolu_ghost", "name": "demi agent send parent", "input": {} },
        } }));
        cli.say(json!({ "type": "stream_event", "event": {
            "type": "content_block_delta",
            "index": 2,
            "delta": { "type": "input_json_delta", "partial_json": "{}" },
        } }));
        cli.say(json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": "toolu_ghost", "name": "demi agent send parent", "input": {} },
        ] } }));
        cli.message_stop();
        cli
    });
    assert_eq!(
        events,
        [
            ProviderEvent::ToolCallStart {
                tool_use_id: "toolu_alpha".into(),
                tool_name: "shell_exec".into(),
            },
            ProviderEvent::ToolCallInput {
                tool_use_id: "toolu_alpha".into(),
                partial_json: "{\"script\": \"pwd\"}".into(),
            },
            call("toolu_alpha", "pwd"),
            ProviderEvent::Response(None),
        ]
    );

    // The batch is the offered call alone, so its one result answers it.
    let mut second = first.clone();
    second.push(tool_use("toolu_alpha", "pwd"));
    second.push(tool_result("toolu_alpha", "/tmp"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second))), async {
        let alpha = cli.mcp_reply("call-alpha").await;
        assert_eq!(
            alpha["result"]["content"],
            json!([{ "type": "text", "text": "/tmp" }])
        );
        cli.text("both answered");
        cli.result(3, 5);
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("both answered".into()), usage(3, 5)]
    );
    assert!(cli.signals().is_empty());
}

#[tokio::test(flavor = "local")]
async fn a_tool_result_goes_back_as_mcp_content_with_images_as_base64_and_errors_flagged() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("look")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        // A call that names no tool-use id gets a new unique one.
        cli.mcp(
            "call-1",
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "shell_exec", "arguments": { "script": "shot" } },
            }),
        );
        cli
    });
    let [ProviderEvent::ToolCall(held), ProviderEvent::Response(None)] = &events[..] else {
        panic!("expected one call, got {events:?}")
    };
    assert!(
        held.tool_use_id.starts_with("mcp-control-"),
        "{}",
        held.tool_use_id
    );
    assert_eq!(held.input, json!({ "script": "shot" }));

    let mut second = first.clone();
    second.push(InferenceItem::ToolUse {
        model_id: "claude-test".into(),
        tool_use_id: held.tool_use_id.clone(),
        tool_name: held.tool_name.clone(),
        input: held.input.clone(),
    });
    second.push(InferenceItem::ToolResult {
        tool_use_id: held.tool_use_id.clone(),
        output: vec![
            ResultPart::Text("captured".into()),
            ResultPart::Image(MediaBytes {
                data: B64Bytes::new(Bytes::from_static(&[1, 2, 3])),
                media_type: "image/png".into(),
            }),
        ],
        is_error: true,
    });
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second))), async {
        let reply = cli.mcp_reply("call-1").await;
        assert_eq!(
            reply["result"],
            json!({
                "content": [
                    { "type": "text", "text": "captured" },
                    { "type": "image", "data": "AQID", "mimeType": "image/png" },
                ],
                "isError": true,
            })
        );
        cli.result(1, 1);
    });
    assert_eq!(events, [usage(1, 1)]);
}

#[tokio::test(flavor = "local")]
async fn a_malformed_call_is_refused_by_the_server_and_an_unknown_request_by_demi() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, cli) = tokio::join!(all_events(runtime.run(request(vec![user("hi")]))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        cli.mcp(
            "call-1",
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "arguments": {} } }),
        );
        let refused = cli.mcp_reply("call-1").await;
        assert_eq!(refused["error"]["code"], -32601, "{refused}");
        cli.say(json!({
            "type": "control_request",
            "request_id": "hook-1",
            "request": { "subtype": "hook_callback", "callback_id": "x" },
        }));
        let answer = cli.read().await;
        assert_eq!(answer["response"]["subtype"], "error", "{answer}");
        assert_eq!(answer["response"]["request_id"], "hook-1");
        cli.text("went on");
        cli.result(1, 1);
        cli
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("went on".into()), usage(1, 1)]
    );
    assert!(cli.signals().is_empty());
}

#[tokio::test(flavor = "local")]
async fn a_batch_without_all_its_results_or_results_never_asked_for_fail_the_run() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("run both")];
    let batch = |cli: &Cli| {
        cli.message_start();
        cli.tool_use("toolu_alpha", "printf alpha");
        cli.tool_use("toolu_beta", "printf beta");
        cli.message_stop();
    };
    let (_, cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        batch(&cli);
        cli
    });
    // A result missing: the run fails and the process is closed.
    let mut partial = first.clone();
    partial.push(tool_result("toolu_alpha", "alpha done"));
    let events = all_events(runtime.run(request(partial))).await;
    let [ProviderEvent::Error(failure)] = &events[..] else {
        panic!("expected a failure, got {events:?}")
    };
    assert_eq!(
        failure.message,
        "Claude Code provider missing tool_result for SDK MCP tool_use toolu_beta"
    );
    assert_eq!(cli.signals(), [Signal::Terminate]);

    // Results the CLI never asked for before it exited.
    let (_, cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        batch(&cli);
        cli
    });
    let mut both = first.clone();
    both.push(tool_result("toolu_alpha", "alpha done"));
    both.push(tool_result("toolu_beta", "beta done"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(both))), async {
        cli.exit(demi_host_interface::ProcessEnd::Exited(0));
    });
    let [ProviderEvent::Error(failure)] = &events[..] else {
        panic!("expected a failure, got {events:?}")
    };
    assert_eq!(
        failure.message,
        "Claude Code exited before requesting SDK MCP tool result for toolu_alpha, toolu_beta"
    );
}

/// A steer written with a batch's results, which the CLI answers in a turn of
/// its own after the batch's turn ended, is answered in the run that wrote it.
///
/// That run ends at the second `result`, the one after the CLI printed the
/// steer back, and the next run starts clean in the same process. (The run
/// ended at the first `result`, and the steer's answer was skipped as output of
/// no request.)
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_steer_written_with_a_batchs_results_and_answered_in_a_turn_of_its_own_is_answered_in_its_run() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("run it")];
    let (_, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        cli.message_start();
        cli.tool_use("toolu_1", "pwd");
        cli.call("call-1", 2, "toolu_1", "pwd");
        cli.message_stop();
        cli
    });
    let mut second = first.clone();
    second.push(tool_use("toolu_1", "pwd"));
    second.push(tool_result("toolu_1", "/tmp"));
    second.push(InferenceItem::UserSteer {
        content: vec![demi_provider_common::UserPart::Text("also list it".into())],
    });
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second.clone()))), async {
        let steer = cli.read().await;
        assert_eq!(steer["type"], "user", "{steer}");
        cli.mcp_reply("call-1").await;
        cli.text("done");
        cli.result(1, 1);
        let mut replay = steer.clone();
        replay["isReplay"] = json!(true);
        cli.say(replay);
        cli.text("listed it");
        cli.result(2, 2);
    });
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("done".into()),
            usage(1, 1),
            ProviderEvent::TextDelta("listed it".into()),
            usage(2, 2),
        ]
    );

    let mut third = second.clone();
    third.push(user("next"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(third))), async {
        cli.take().await;
        cli.text("next answer");
        cli.result(3, 3);
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("next answer".into()), usage(3, 3)]
    );
    assert_eq!(placement.starts(), 1);
    assert!(cli.signals().is_empty());
}

fn stream_event(cli: &Cli, event: serde_json::Value) {
    cli.say(json!({ "type": "stream_event", "event": event }));
}

/// A whole assistant line of the message `id`, as the CLI prints each block
/// of a streamed message, or a complete answer whole.
fn whole(cli: &Cli, id: &str, content: serde_json::Value, usage: serde_json::Value) {
    cli.say(json!({
        "type": "assistant",
        "message": { "id": id, "content": content, "usage": usage },
    }));
}

fn shell_use(id: &str, script: &str) -> serde_json::Value {
    json!({ "type": "tool_use", "id": id, "name": "mcp__main__shell_exec", "input": { "script": script } })
}

/// A whole line of the message being streamed repeats a block that
/// streamed: its text shows once, and its tool uses wait for the batch that
/// `message_stop` ends.
// One run against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn the_whole_lines_of_a_streamed_message_repeat_it_and_its_batch_waits_for_message_stop() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(vec![user("run both")]))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        stream_event(&cli, json!({ "type": "message_start", "message": { "id": "msg_1" } }));
        stream_event(&cli, json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }));
        cli.text("running both");
        whole(&cli, "msg_1", json!([{ "type": "text", "text": "running both" }]), json!({}));
        cli.streamed_tool_use(1, "toolu_alpha", &["{\"script\": \"printf alpha\"}"]);
        whole(&cli, "msg_1", json!([shell_use("toolu_alpha", "printf alpha")]), json!({}));
        cli.call("call-alpha", 2, "toolu_alpha", "printf alpha");
        cli.streamed_tool_use(2, "toolu_beta", &["{\"script\": \"printf beta\"}"]);
        whole(&cli, "msg_1", json!([shell_use("toolu_beta", "printf beta")]), json!({}));
        cli.message_delta(7, 9);
        cli.message_stop();
        cli
    });
    let start = |id: &str| ProviderEvent::ToolCallStart {
        tool_use_id: id.into(),
        tool_name: "shell_exec".into(),
    };
    let input = |id: &str, piece: &str| ProviderEvent::ToolCallInput {
        tool_use_id: id.into(),
        partial_json: piece.into(),
    };
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("running both".into()),
            start("toolu_alpha"),
            input("toolu_alpha", "{\"script\": \"printf alpha\"}"),
            start("toolu_beta"),
            input("toolu_beta", "{\"script\": \"printf beta\"}"),
            call("toolu_alpha", "printf alpha"),
            call("toolu_beta", "printf beta"),
            usage(7, 9),
        ]
    );
    assert!(cli.unread().is_empty());
}

/// Claude Code retries a call whose stream failed without streaming, and prints
/// its answer as one whole line that no `message_start` or `message_stop`
/// frames.
///
/// That answer shows, and its batch reaches the agent at once, in a process
/// that streamed before: the CLI runs the call and waits for its result. (A
/// conversation stalled for good when the answer was taken for a repeat of what
/// streamed.)
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_complete_answer_shows_and_hands_its_tool_call_on_at_once_after_streamed_ones() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut items = vec![user("work")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(items.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        stream_event(&cli, json!({ "type": "message_start", "message": { "id": "msg_1" } }));
        cli.streamed_tool_use(0, "toolu_1", &["{\"script\": \"pwd\"}"]);
        whole(&cli, "msg_1", json!([shell_use("toolu_1", "pwd")]), json!({}));
        cli.call("call-1", 1, "toolu_1", "pwd");
        cli.message_delta(100, 10);
        cli.message_stop();
        cli
    });
    assert_eq!(events.last(), Some(&usage(100, 10)), "{events:?}");

    items.push(tool_use("toolu_1", "pwd"));
    items.push(tool_result("toolu_1", "/tmp"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items.clone()))), async {
        cli.mcp_reply("call-1").await;
        whole(
            &cli,
            "msg_2",
            json!([
                { "type": "text", "text": "Checking the runtime." },
                shell_use("toolu_2", "rg demi"),
            ]),
            json!({ "input_tokens": 300, "output_tokens": 20 }),
        );
        cli.call("call-2", 2, "toolu_2", "rg demi");
    });
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("Checking the runtime.".into()),
            call("toolu_2", "rg demi"),
            usage(300, 20),
        ]
    );

    items.push(InferenceItem::AssistantText {
        model_id: "claude-test".into(),
        text: "Checking the runtime.".into(),
    });
    items.push(tool_use("toolu_2", "rg demi"));
    items.push(tool_result("toolu_2", "found"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items))), async {
        let reply = cli.mcp_reply("call-2").await;
        assert_eq!(
            reply["result"]["content"],
            json!([{ "type": "text", "text": "found" }])
        );
        cli.text("done");
        cli.result(310, 5);
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("done".into()), usage(310, 5)]
    );
    assert_eq!(placement.starts(), 1);
}

/// A stream that fails midway leaves what it showed on the page; the
/// complete answer that replaces it repeats that and goes on, and only the
/// rest shows, so the text reads once and the call runs once.
// Two runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_complete_answer_after_a_failed_stream_shows_only_what_had_not_streamed_and_its_call_once()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut items = vec![user("work")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(items.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        stream_event(&cli, json!({ "type": "message_start", "message": { "id": "msg_a" } }));
        stream_event(&cli, json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "" } }));
        stream_event(&cli, json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "Plan the check." } }));
        stream_event(&cli, json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "signature_delta", "signature": "sig" } }));
        whole(&cli, "msg_a", json!([{ "type": "thinking", "thinking": "Plan the check.", "signature": "sig" }]), json!({}));
        stream_event(&cli, json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "text", "text": "" } }));
        cli.text("Let me");
        // The stream stalls, and the CLI asks again without streaming.
        whole(
            &cli,
            "msg_b",
            json!([
                { "type": "thinking", "thinking": "Plan the check.", "signature": "sig" },
                { "type": "text", "text": "Let me check the runtime." },
                shell_use("toolu_1", "rg demi"),
            ]),
            json!({ "input_tokens": 300, "output_tokens": 20 }),
        );
        cli.call("call-1", 1, "toolu_1", "rg demi");
        cli
    });
    assert_eq!(
        events,
        [
            ProviderEvent::ThinkingStart,
            ProviderEvent::ThinkingDelta("Plan the check.".into()),
            ProviderEvent::ThinkingSignature("sig".into()),
            ProviderEvent::TextDelta("Let me".into()),
            ProviderEvent::TextDelta(" check the runtime.".into()),
            call("toolu_1", "rg demi"),
            usage(300, 20),
        ]
    );

    items.push(InferenceItem::AssistantText {
        model_id: "claude-test".into(),
        text: "Let me check the runtime.".into(),
    });
    items.push(tool_use("toolu_1", "rg demi"));
    items.push(tool_result("toolu_1", "found"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(items))), async {
        cli.mcp_reply("call-1").await;
        cli.text("done");
        cli.result(310, 5);
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("done".into()), usage(310, 5)]
    );
    assert!(cli.unread().is_empty(), "the call is answered once");
}
