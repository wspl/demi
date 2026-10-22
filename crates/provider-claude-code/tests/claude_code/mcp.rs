//! The SDK MCP channel and the model's tool batches (`crates/provider-claude-code/src/mcp.rs`).

use demi_provider_common::{ProviderEvent, ToolCall};
use demi_shared_types::TokenUsage;
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

/// The incident: three receipts written at once to an idle kept process. The
/// CLI answers the first in a turn of its own and merges the other two into one
/// turn, which calls a tool. The next run delivers the result, and the merged
/// turn's `result` names both receipts, which ends that run; a further run goes
/// on in the same process. (A run that counted the messages the CLI printed
/// back waited for good after that `result`: the CLI printed the merged turn
/// back once.)
// Four runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_merged_turn_that_calls_a_tool_ends_the_run_that_delivers_its_result() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let first = vec![user("run the readers")];
    let (_, mut cli) = tokio::join!(all_events(runtime.run(request(first.clone()))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.handshake().await;
        cli.text("started");
        cli.result(1, 1);
        cli
    });

    let mut receipts = first.clone();
    receipts.push(user("receipt A"));
    receipts.push(user("receipt B"));
    receipts.push(user("receipt C"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(receipts.clone()))), async {
        cli.take().await;
        cli.text("A is in");
        cli.result(2, 2);
        cli.take().await;
        cli.take().await;
        cli.message_start();
        cli.tool_use("toolu_1", "pwd");
        cli.call("call-1", 2, "toolu_1", "pwd");
        cli.message_stop();
    });
    assert_eq!(events[..2], [ProviderEvent::TextDelta("A is in".into()), usage(2, 2)]);
    assert!(events.iter().any(|event| matches!(event, ProviderEvent::ToolCall(call) if call.tool_use_id == "toolu_1")), "{events:?}");

    let mut delivered = receipts.clone();
    delivered.push(tool_use("toolu_1", "pwd"));
    delivered.push(tool_result("toolu_1", "/tmp"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(delivered.clone()))), async {
        cli.mcp_reply("call-1").await;
        cli.text("B and C are in");
        cli.result(3, 3);
    });
    assert_eq!(events, [ProviderEvent::TextDelta("B and C are in".into()), usage(3, 3)]);

    let mut next = delivered.clone();
    next.push(user("next"));
    let (events, ()) = tokio::join!(all_events(runtime.run(request(next))), async {
        cli.take().await;
        cli.text("next answer");
        cli.result(4, 4);
    });
    assert_eq!(events, [ProviderEvent::TextDelta("next answer".into()), usage(4, 4)]);
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
