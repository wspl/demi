//! The SDK MCP channel and the model's tool batches (`claude-code.md` § The
//! SDK MCP channel, § Tool-call batches).

use bytes::Bytes;
use demi_core::{B64Bytes, TokenUsage, ToolMediaSource, ToolResultContentBlock};
use demi_provider::{InferenceItem, ProviderEvent, ToolCall};
use demi_shell::Signal;
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
    ProviderEvent::Response(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    })
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
        cli.read().await;
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
        cli.mcp(
            "ping",
            json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }),
        );
        assert_eq!(cli.mcp_reply("ping").await["result"], json!({}));
        cli.call("call-1", 3, "toolu_1", "pwd");
        cli
    });
    assert_eq!(events, [call("toolu_1", "pwd")]);
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
        cli.read().await;
        cli.handshake().await;
        cli.message_start();
        cli.text("running both");
        cli.tool_use("toolu_alpha", "printf alpha");
        // The CLI runs a tool as soon as its block is whole, before the
        // message ends; the second call stays blocked behind the first.
        cli.call("call-alpha", 2, "toolu_alpha", "printf alpha");
        cli.tool_use("toolu_beta", "printf beta");
        cli.message_stop();
        cli
    });
    assert_eq!(
        events,
        [
            ProviderEvent::TextDelta("running both".into()),
            call("toolu_alpha", "printf alpha"),
            call("toolu_beta", "printf beta"),
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

#[tokio::test(flavor = "local")]
async fn a_tool_result_goes_back_as_mcp_content_with_images_as_base64_and_errors_flagged() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("look")];
    let (events, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.read().await;
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
    let [ProviderEvent::ToolCall(held)] = &events[..] else {
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
            ToolResultContentBlock::Text {
                text: "captured".into(),
            },
            ToolResultContentBlock::Image {
                source: ToolMediaSource::Binary {
                    data: B64Bytes::new(Bytes::from_static(&[1, 2, 3])),
                    media_type: "image/png".into(),
                },
            },
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
        cli.read().await;
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
        cli.read().await;
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
        cli.read().await;
        batch(&cli);
        cli
    });
    let mut both = first.clone();
    both.push(tool_result("toolu_alpha", "alpha done"));
    both.push(tool_result("toolu_beta", "beta done"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(both))), async {
        cli.exit(demi_shell::ProcessEnd::Exited(0));
    });
    let [ProviderEvent::Error(failure)] = &events[..] else {
        panic!("expected a failure, got {events:?}")
    };
    assert_eq!(
        failure.message,
        "Claude Code exited before requesting SDK MCP tool result for toolu_alpha, toolu_beta"
    );
}

#[tokio::test(flavor = "local")]
async fn output_a_run_left_behind_belongs_to_no_request() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("run it")];
    let (_, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.read().await;
        cli.handshake().await;
        cli.message_start();
        cli.tool_use("toolu_1", "pwd");
        cli.call("call-1", 2, "toolu_1", "pwd");
        cli.message_stop();
        cli
    });
    // The result comes back with a steer; the CLI answers the steer in a
    // turn of its own after the run ended at the first result.
    let mut second = first.clone();
    second.push(tool_use("toolu_1", "pwd"));
    second.push(tool_result("toolu_1", "/tmp"));
    second.push(InferenceItem::UserSteer {
        content: vec![demi_core::UserContentBlock::Text {
            text: "also list it".into(),
        }],
    });
    let (events, ()) = tokio::join!(all_events(runtime.run(request(second.clone()))), async {
        let steer = cli.read().await;
        assert_eq!(steer["type"], "user", "{steer}");
        cli.mcp_reply("call-1").await;
        cli.text("done");
        cli.result(1, 1);
        cli.text("listed it");
        cli.result(2, 2);
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("done".into()), usage(1, 1)]
    );
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    let mut third = second.clone();
    third.push(user("next"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(third))), async {
        cli.read().await;
        cli.text("next answer");
        cli.result(3, 3);
    });
    assert_eq!(
        events,
        [ProviderEvent::TextDelta("next answer".into()), usage(3, 3)]
    );
}
