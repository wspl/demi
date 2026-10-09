//! The session a new process resumes and the blocks the CLI's entries
//! belong to (`claude-code.md` § The session a process resumes).

use bytes::Bytes;
use demi_provider_common::{
    EntriesOf, InferenceItem, InferenceRequest, MediaBytes, Medium, ProviderEvent, RequestBlock,
    ResultPart, UserPart,
};
use demi_shared_types::{B64Bytes, BlobRef, TokenUsage};
use serde_json::{Value, json};

use crate::cli::*;

/// The conversation's session, which the CLI's session is named by.
const SESSION: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

/// `items` as a session's request: the blocks that give them, with the
/// entries kept on each.
fn session_request(items: Vec<InferenceItem>, blocks: Vec<(usize, Vec<Value>)>) -> InferenceRequest {
    let mut start = 0;
    let blocks: Vec<RequestBlock> = blocks
        .into_iter()
        .map(|(len, entries)| {
            let block = RequestBlock {
                items: start..start + len,
                entries,
            };
            start += len;
            block
        })
        .collect();
    assert_eq!(start, items.len(), "the blocks give every item");
    InferenceRequest {
        session_id: SESSION.into(),
        blocks: blocks.into(),
        ..request(items)
    }
}

fn image(bytes: &'static [u8]) -> MediaBytes {
    MediaBytes {
        data: B64Bytes::from(Bytes::from_static(bytes)),
        media_type: "image/png".into(),
    }
}

/// An entry the CLI wrote, of `kind`, with `fields`.
fn entry(kind: &str, uuid: &str, fields: Value) -> Value {
    let mut entry = json!({
        "type": kind,
        "uuid": uuid,
        "parentUuid": "the-cli's-own-parent",
        "isSidechain": false,
        "sessionId": SESSION,
        "timestamp": "2026-09-24T07:00:00.000Z",
        "cwd": "/home/demi/.demi/claude/run",
        "version": "2.1.3",
    });
    entry
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    entry
}

fn assistant_entry(uuid: &str, message: &str, content: Value) -> Value {
    entry(
        "assistant",
        uuid,
        json!({ "message": {
            "id": message, "type": "message", "role": "assistant", "model": "claude-test",
            "content": [content], "stop_reason": "tool_use",
        } }),
    )
}

fn result_entry(uuid: &str, content: Value) -> Value {
    entry(
        "user",
        uuid,
        json!({ "message": { "role": "user", "content": [content] }, "toolUseResult": [] }),
    )
}

/// `entry` without the ids that chain it.
fn unchained(entry: &Value) -> Value {
    let mut entry = entry.clone();
    let fields = entry.as_object_mut().unwrap();
    fields.remove("uuid");
    fields.remove("parentUuid");
    entry
}

/// Asserts that each entry names the one before it, and that every entry
/// is of `session`.
fn chained(session: &[Value], id: &str) {
    assert_eq!(session[0]["parentUuid"], Value::Null);
    for pair in session.windows(2) {
        assert_eq!(pair[1]["parentUuid"], pair[0]["uuid"], "{}", pair[1]);
    }
    for entry in session {
        assert_eq!(entry["sessionId"], id, "{entry}");
    }
}

/// An entry Demi wrote for a block no entry of the CLI's stands for.
fn written(kind: &str, message: Value) -> Value {
    json!({
        "type": kind,
        "isSidechain": false,
        "sessionId": SESSION,
        "timestamp": NOW,
        "message": message,
    })
}

// 0.01 s.
#[tokio::test(flavor = "local")]
async fn a_new_process_resumes_the_blocks_with_the_clis_entries_as_they_were_and_others_in_its_format()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let shot = image(b"shot");
    let items = vec![
        // The user's message, which Demi wrote.
        InferenceItem::UserMessage {
            content: vec![
                UserPart::Text("look at this".into()),
                UserPart::Image(Medium::Bytes(image(b"png"))),
            ],
        },
        // A Claude Code turn: reasoning, then two calls in one batch.
        InferenceItem::AssistantThinking {
            model_id: "claude-test".into(),
            text: "weighing".into(),
            signature: Some("sig-1".into()),
            kept_past_summary: false,
        },
        tool_use("toolu_1", "pwd"),
        InferenceItem::ToolResult {
            tool_use_id: "toolu_1".into(),
            output: vec![ResultPart::Text("/tmp".into()), ResultPart::Image(shot.clone())],
            is_error: false,
        },
        tool_use("toolu_2", "ls"),
        tool_result("toolu_2", "a.txt"),
        // Another provider's turn: reasoning, text and a call.
        InferenceItem::AssistantThinking {
            model_id: "gpt-5.5".into(),
            text: "planning".into(),
            signature: Some("gpt-signature".into()),
            kept_past_summary: false,
        },
        InferenceItem::AssistantText {
            model_id: "gpt-5.5".into(),
            text: "Listing.".into(),
        },
        InferenceItem::ToolUse {
            model_id: "gpt-5.5".into(),
            tool_use_id: "call_a1b2c3".into(),
            tool_name: "shell_exec".into(),
            input: json!({ "script": "rm x" }),
        },
        InferenceItem::ToolResult {
            tool_use_id: "call_a1b2c3".into(),
            output: vec![ResultPart::Text("denied".into())],
            is_error: true,
        },
        user("continue"),
    ];
    let date = entry(
        "attachment",
        "e-date",
        json!({ "attachment": { "type": "date", "date": "2026-09-24" } }),
    );
    let thinking = assistant_entry(
        "e-thinking",
        "msg_1",
        json!({ "type": "thinking", "thinking": "weighing", "signature": "sig-1" }),
    );
    let first_call = assistant_entry(
        "e-call-1",
        "msg_1",
        json!({ "type": "tool_use", "id": "toolu_1", "name": "mcp__main__shell_exec", "input": { "script": "pwd" } }),
    );
    let second_call = assistant_entry(
        "e-call-2",
        "msg_1",
        json!({ "type": "tool_use", "id": "toolu_2", "name": "mcp__main__shell_exec", "input": { "script": "ls" } }),
    );
    // The block keeps the image of the result as a reference to its own.
    let image_content = |data: Value| {
        json!({ "type": "tool_result", "tool_use_id": "toolu_1", "content": [
            { "type": "text", "text": "/tmp" },
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": data } },
            { "type": "text", "text": "[Image: source: /tmp/demi-claude-1/projects/demi/tool-results/1.png]" },
        ] })
    };
    let reference = json!({ "demiBlob": BlobRef::of(b"shot").as_str() });
    let first_result = result_entry("e-result-1", image_content(reference));
    let second_result = result_entry(
        "e-result-2",
        json!({ "type": "tool_result", "tool_use_id": "toolu_2", "content": [{ "type": "text", "text": "a.txt" }] }),
    );
    let blocks = vec![
        (1, vec![]),
        (1, vec![date.clone(), thinking.clone()]),
        (2, vec![first_call.clone(), first_result.clone()]),
        (2, vec![second_call.clone(), second_result.clone()]),
        (1, vec![]),
        (1, vec![]),
        (2, vec![]),
        (1, vec![]),
    ];
    let request = session_request(items, blocks);
    let (events, cli) = tokio::join!(all_events(runtime.run(request)), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.result(3, 1);
        cli
    });
    assert_eq!(
        events,
        [ProviderEvent::Response(TokenUsage {
            input_tokens: 3,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        })]
    );
    // The process waits for Demi's MCP server before it answers what the
    // session ends with, so that request offers the tools.
    let configured = cli
        .spawn
        .args
        .windows(2)
        .find(|pair| pair[0] == "--mcp-config")
        .map(|pair| serde_json::from_str::<Value>(&pair[1]).unwrap());
    assert_eq!(
        configured,
        Some(json!({ "mcpServers": { "main": { "type": "sdk", "name": "main" } } }))
    );

    let session = cli.session();
    chained(&session, SESSION);
    let environment = json!({
        "type": "attachment",
        "isSidechain": false,
        "sessionId": SESSION,
        "timestamp": NOW,
        "attachment": { "type": "environment", "snapshot": {
            "workingDirectory": "/home/demi/.demi/claude/run",
            "isWorktree": false,
            "isGitRepo": false,
            "additionalWorkingDirectories": [],
            "platform": "linux",
            "shell": "bash",
            "osVersion": "Linux 6.8.0-test",
        } },
        "rendered": [{ "content": " " }],
        "renderedRole": "system",
    });
    let gpt = |content: Value| {
        json!({ "id": "msg_demi_7", "type": "message", "role": "assistant", "model": "gpt-5.5", "content": [content] })
    };
    let restored = result_entry("e-result-1", image_content(json!("c2hvdA==")));
    let expected = [
        environment,
        written(
            "user",
            json!({ "role": "user", "content": [
                { "type": "text", "text": "look at this" },
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "cG5n" } },
            ] }),
        ),
        unchained(&date),
        unchained(&thinking),
        // A batch's calls come before its results, as the CLI wrote them.
        unchained(&first_call),
        unchained(&second_call),
        unchained(&restored),
        unchained(&second_result),
        // Another vendor's reasoning is left out.
        written("assistant", gpt(json!({ "type": "text", "text": "Listing." }))),
        written(
            "assistant",
            gpt(json!({ "type": "tool_use", "id": "call_a1b2c3", "name": "mcp__main__shell_exec", "input": { "script": "rm x" } })),
        ),
        written(
            "user",
            json!({ "role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "call_a1b2c3",
                "content": [{ "type": "text", "text": "denied" }], "is_error": true,
            }] }),
        ),
        written(
            "user",
            json!({ "role": "user", "content": [{ "type": "text", "text": "continue" }] }),
        ),
    ];
    let unchained_session: Vec<Value> = session.iter().map(unchained).collect();
    assert_eq!(unchained_session, expected);
    // The CLI's entries keep their ids.
    let ids: Vec<&Value> = session[2..8].iter().map(|entry| &entry["uuid"]).collect();
    assert_eq!(
        ids,
        ["e-date", "e-thinking", "e-call-1", "e-call-2", "e-result-1", "e-result-2"]
    );
}

// 0.01 s.
#[tokio::test(flavor = "local")]
async fn a_forks_session_renames_its_parents_entries_and_entries_stand_for_their_blocks_while_their_media_are_sent()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let parent = "6c84fb90-12c4-41b8-8b4a-0a1f6d8e9a47";
    let of_parent = |mut entry: Value| {
        entry["sessionId"] = json!(parent);
        entry
    };
    let call = of_parent(assistant_entry(
        "p-call",
        "msg_1",
        json!({ "type": "tool_use", "id": "toolu_1", "name": "mcp__main__shell_exec", "input": { "script": "pwd" } }),
    ));
    let mut result = of_parent(result_entry(
        "p-result",
        json!({ "type": "tool_result", "tool_use_id": "toolu_1", "content": [{ "type": "text", "text": "/tmp" }] }),
    ));
    result["sourceToolAssistantUUID"] = json!("p-call");
    // An image the request now sends as text, as to a model that takes
    // none: the entry that held it cannot be written as it was.
    let gone = result_entry(
        "e-gone",
        json!({ "type": "tool_result", "tool_use_id": "toolu_2", "content": [
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": { "demiBlob": BlobRef::of(b"gone").as_str() } } },
        ] }),
    );
    // A steer the CLI folded into its turn stands for the steer.
    let folded = entry(
        "attachment",
        "e-steer",
        json!({ "attachment": { "type": "queued_command", "prompt": [{ "type": "text", "text": "and this" }] } }),
    );
    let items = vec![
        user("look"),
        tool_use("toolu_1", "pwd"),
        tool_result("toolu_1", "/tmp"),
        tool_use("toolu_2", "shot"),
        tool_result("toolu_2", "[image:image/png, not sent: the model does not accept it]"),
        InferenceItem::UserSteer {
            content: vec![UserPart::Text("and this".into())],
        },
        user("next"),
    ];
    let blocks = vec![
        (1, vec![]),
        (2, vec![call.clone(), result.clone()]),
        (2, vec![gone]),
        (1, vec![folded.clone()]),
        (1, vec![]),
    ];
    let (_, cli) = tokio::join!(
        all_events(runtime.run(session_request(items, blocks))),
        async {
            let mut cli = starts.next().await;
            cli.initialized().await;
            cli.result(1, 1);
            cli
        }
    );
    let session = cli.session();
    chained(&session, SESSION);
    // The parent's entries carry new ids of the fork's own, and a field
    // that named the call names its new id.
    let (renamed_call, renamed_result) = (&session[2], &session[4]);
    assert_eq!(renamed_call["message"], call["message"]);
    assert_eq!(renamed_result["message"], result["message"]);
    assert_ne!(renamed_call["uuid"], "p-call");
    assert_ne!(renamed_result["uuid"], "p-result");
    assert_eq!(renamed_result["sourceToolAssistantUUID"], renamed_call["uuid"]);
    // The block whose medium is not sent is written from its items, its
    // call in the batch's message and its result after the batch's first.
    assert_eq!(
        session[3]["message"],
        json!({ "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-test", "content": [
            { "type": "tool_use", "id": "toolu_2", "name": "mcp__main__shell_exec", "input": { "script": "shot" } },
        ] })
    );
    assert_eq!(
        session[5]["message"]["content"][0]["content"],
        json!([{ "type": "text", "text": "[image:image/png, not sent: the model does not accept it]" }])
    );
    assert_eq!(unchained(&session[6]), unchained(&folded));
    assert_eq!(
        session[7]["message"]["content"],
        json!([{ "type": "text", "text": "next" }])
    );
    assert_eq!(session.len(), 8);
}

/// A tool use the CLI streams and prints whole, and its `tools/call`.
fn call_tool(cli: &Cli, id: &str, script: &str) {
    cli.message_start();
    cli.streamed_tool_use(0, id, &[&json!({ "script": script }).to_string()]);
    cli.say(json!({ "type": "assistant", "message": { "id": "msg_2", "content": [{
        "type": "tool_use", "id": id, "name": "mcp__main__shell_exec", "input": { "script": script },
    }] } }));
    cli.call("call-1", 2, id, script);
    cli.message_stop();
}

// 0.01 s.
#[tokio::test(flavor = "local")]
async fn each_entry_the_cli_mirrors_goes_to_its_block_and_one_of_no_block_to_the_next() {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let model = entry(
        "attachment",
        "e-model",
        json!({ "attachment": { "type": "model" } }),
    );
    let thinking_content = json!({ "type": "thinking", "thinking": "weighing", "signature": "sig-1" });
    let thinking = assistant_entry("e-thinking", "msg_1", thinking_content.clone());
    let text = assistant_entry("e-text", "msg_1", json!({ "type": "text", "text": "Hello" }));
    let first = vec![user("do work")];
    let (events, mut cli) = tokio::join!(
        all_events(runtime.run(session_request(first.clone(), vec![(1, vec![])]))),
        async {
            let mut cli = starts.next().await;
            cli.initialized().await;
            cli.message_start();
            cli.say(json!({ "type": "stream_event", "event": {
                "type": "content_block_start", "index": 0,
                "content_block": { "type": "thinking", "thinking": "", "signature": "" },
            } }));
            cli.say(json!({ "type": "stream_event", "event": {
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "thinking_delta", "thinking": "weighing" },
            } }));
            cli.say(json!({ "type": "stream_event", "event": {
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "signature_delta", "signature": "sig-1" },
            } }));
            cli.say(json!({ "type": "assistant", "message": { "id": "msg_1", "content": [thinking_content] } }));
            // Text in pieces is one block.
            cli.text("Hel");
            cli.text("lo");
            cli.say(json!({ "type": "assistant", "message": { "id": "msg_1", "content": [{ "type": "text", "text": "Hello" }] } }));
            cli.message_stop();
            cli.mirror(vec![model.clone(), thinking.clone(), text.clone()]);
            cli.result(1, 1);
            cli
        }
    );
    // The model attachment stands for no block and goes to the next: the
    // reasoning, the run's first block.
    assert_eq!(
        &events[5..7],
        [
            ProviderEvent::Entries {
                of: EntriesOf::Output(0),
                entries: vec![model, thinking],
            },
            ProviderEvent::Entries {
                of: EntriesOf::Output(1),
                entries: vec![text],
            },
        ]
    );

    // The next message, given to the kept process: its entry goes to its
    // block, an item of the request.
    let mut second = first;
    second.extend([
        InferenceItem::AssistantThinking {
            model_id: "claude-test".into(),
            text: "weighing".into(),
            signature: Some("sig-1".into()),
            kept_past_summary: false,
        },
        InferenceItem::AssistantText {
            model_id: "claude-test".into(),
            text: "Hello".into(),
        },
        user("run it"),
    ]);
    let message = entry(
        "user",
        "e-message",
        json!({ "message": { "role": "user", "content": [{ "type": "text", "text": "run it" }] } }),
    );
    let reminder = entry(
        "attachment",
        "e-reminder",
        json!({ "attachment": { "type": "date" } }),
    );
    let blocks = vec![(1, vec![]), (1, vec![]), (1, vec![]), (1, vec![])];
    let (events, ()) = tokio::join!(
        all_events(runtime.run(session_request(second.clone(), blocks))),
        async {
            cli.read().await;
            cli.mirror(vec![message.clone(), reminder.clone()]);
            cli.handshake().await;
            call_tool(&cli, "toolu_1", "pwd");
        }
    );
    assert_eq!(
        events[0],
        ProviderEvent::Entries {
            of: EntriesOf::Item(3),
            entries: vec![message],
        }
    );
    assert!(
        matches!(events.last(), Some(ProviderEvent::ToolCall(_))),
        "{events:?}"
    );

    // The batch's entries come after its run ended: its call's block is an
    // item of the next request, and the result's image a reference to the
    // block's own.
    let call = assistant_entry(
        "e-call",
        "msg_2",
        json!({ "type": "tool_use", "id": "toolu_1", "name": "mcp__main__shell_exec", "input": { "script": "pwd" } }),
    );
    cli.mirror(vec![call.clone()]);
    let mut third = second;
    third.extend([
        tool_use("toolu_1", "pwd"),
        InferenceItem::ToolResult {
            tool_use_id: "toolu_1".into(),
            output: vec![ResultPart::Image(image(b"shot"))],
            is_error: false,
        },
        InferenceItem::UserSteer {
            content: vec![UserPart::Text("also list it".into())],
        },
    ]);
    // The CLI folds a message it is given while it works into the turn,
    // as an attachment of its own.
    let folded = entry(
        "attachment",
        "e-steer",
        json!({ "attachment": { "type": "queued_command", "prompt": [{ "type": "text", "text": "also list it" }] } }),
    );
    let result = |data: Value| {
        result_entry(
            "e-result",
            json!({ "type": "tool_result", "tool_use_id": "toolu_1", "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": data } },
            ] }),
        )
    };
    let answered = assistant_entry("e-answer", "msg_3", json!({ "type": "text", "text": "Done" }));
    let blocks = vec![
        (1, vec![]),
        (1, vec![]),
        (1, vec![]),
        (1, vec![]),
        (2, vec![]),
        (1, vec![]),
    ];
    let (events, ()) = tokio::join!(
        all_events(runtime.run(session_request(third, blocks))),
        async {
            assert_eq!(cli.read().await["type"], "user");
            cli.mcp_reply("call-1").await;
            cli.mirror(vec![result(json!("c2hvdA==")), folded.clone()]);
            cli.text("Done");
            cli.say(json!({ "type": "assistant", "message": { "id": "msg_3", "content": [{ "type": "text", "text": "Done" }] } }));
            cli.mirror(vec![answered.clone()]);
            cli.result(2, 2);
        }
    );
    let reference = json!({ "demiBlob": BlobRef::of(b"shot").as_str() });
    assert_eq!(
        events,
        [
            ProviderEvent::Entries {
                of: EntriesOf::Item(4),
                entries: vec![reminder, call],
            },
            ProviderEvent::Entries {
                of: EntriesOf::Item(5),
                entries: vec![result(reference)],
            },
            ProviderEvent::Entries {
                of: EntriesOf::Item(6),
                entries: vec![folded],
            },
            ProviderEvent::TextDelta("Done".into()),
            ProviderEvent::Entries {
                of: EntriesOf::Output(0),
                entries: vec![answered],
            },
            ProviderEvent::Response(TokenUsage {
                input_tokens: 2,
                output_tokens: 2,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }),
        ]
    );
    runtime.close().await;
}
