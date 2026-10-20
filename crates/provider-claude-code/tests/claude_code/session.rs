//! The session a new process resumes and the blocks the CLI's entries
//! belong to (`crates/provider-claude-code/src/session.rs`).

use bytes::Bytes;
use demi_provider_common::{
    EntriesOf, InferenceItem, InferenceRequest, MediaBytes, Medium, ProviderEvent, RequestBlock,
    ResultPart, UserPart, testing::every_vendor_accepts,
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
                entries: held(entries),
            };
            start += len;
            block
        })
        .collect();
    assert_eq!(start, items.len(), "the blocks give every item");
    InferenceRequest {
        session_id: SESSION.into(),
        affinity: None,
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
    // Codex joins its call id and item id with `|`, which Claude refuses.
    let codex_id = "call_a1b2c3|fc_68e0a1".to_owned();
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
            tool_use_id: codex_id.clone(),
            tool_name: "shell_exec".into(),
            input: json!({ "script": "rm x" }),
        },
        InferenceItem::ToolResult {
            tool_use_id: codex_id.clone(),
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
    // The second call's block holds its call's entry but not its result's,
    // as after an attempt given up for another account, which keeps none
    // of the entries it mirrored: the file still holds the result, written
    // from the block's item.
    let blocks = vec![
        (1, vec![]),
        (1, vec![date.clone(), thinking.clone()]),
        (2, vec![first_call.clone(), first_result.clone()]),
        (2, vec![second_call.clone()]),
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
        [ProviderEvent::Response(Some(TokenUsage {
            input_tokens: 3,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }))]
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
            "workingDirectory": "/home/demi/run",
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
        written(
            "user",
            json!({ "role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "toolu_2",
                "content": [{ "type": "text", "text": "a.txt" }],
            }] }),
        ),
        // Another vendor's reasoning is left out.
        written("assistant", gpt(json!({ "type": "text", "text": "Listing." }))),
        written(
            "assistant",
            gpt(json!({ "type": "tool_use", "id": "call_a1b2c3_fc_68e0a1", "name": "mcp__main__shell_exec", "input": { "script": "rm x" } })),
        ),
        written(
            "user",
            json!({ "role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "call_a1b2c3_fc_68e0a1",
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
    // Every call and result names an id the vendor accepts, Codex's too.
    let ids: Vec<&str> = session
        .iter()
        .flat_map(|entry| entry["message"]["content"].as_array().into_iter().flatten())
        .filter_map(|block| block["id"].as_str().or(block["tool_use_id"].as_str()))
        .collect();
    assert_eq!(ids.len(), 6);
    for id in ids {
        assert!(every_vendor_accepts(id), "{id}");
    }
    // The CLI's entries keep their ids.
    let ids: Vec<&Value> = session[2..7].iter().map(|entry| &entry["uuid"]).collect();
    assert_eq!(ids, ["e-date", "e-thinking", "e-call-1", "e-call-2", "e-result-1"]);
}

// 0.01 s.
#[tokio::test(flavor = "local")]
async fn calls_of_different_responses_are_written_each_with_its_result_and_parallel_calls_as_one_batch()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    // The model called `pwd` in one response and, once it had the result,
    // `ls` and `id` together in the next, with nothing between.
    let items = vec![
        user("look around"),
        tool_use("toolu_1", "pwd"),
        tool_result("toolu_1", "/tmp"),
        tool_use("toolu_2", "ls"),
        tool_result("toolu_2", "a.txt"),
        tool_use("toolu_3", "id"),
        tool_result("toolu_3", "uid=1"),
        user("continue"),
    ];
    let call = |uuid: &str, message: &str, id: &str, script: &str| {
        assistant_entry(
            uuid,
            message,
            json!({ "type": "tool_use", "id": id, "name": "mcp__main__shell_exec", "input": { "script": script } }),
        )
    };
    let result = |uuid: &str, id: &str, text: &str| {
        result_entry(
            uuid,
            json!({ "type": "tool_result", "tool_use_id": id, "content": [{ "type": "text", "text": text }] }),
        )
    };
    let pwd = call("e-call-1", "msg_1", "toolu_1", "pwd");
    let pwd_result = result("e-result-1", "toolu_1", "/tmp");
    let ls = call("e-call-2", "msg_2", "toolu_2", "ls");
    let ls_result = result("e-result-2", "toolu_2", "a.txt");
    let id = call("e-call-3", "msg_2", "toolu_3", "id");
    let id_result = result("e-result-3", "toolu_3", "uid=1");
    let blocks = vec![
        (1, vec![]),
        (2, vec![pwd, pwd_result]),
        (2, vec![ls, ls_result]),
        (2, vec![id, id_result]),
        (1, vec![]),
    ];
    let request = session_request(items, blocks);
    let (_, cli) = tokio::join!(all_events(runtime.run(request)), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.result(3, 1);
        cli
    });

    let session = cli.session();
    let uuids: Vec<&str> = session
        .iter()
        .filter_map(|entry| entry["uuid"].as_str())
        .filter(|uuid| uuid.starts_with("e-"))
        .collect();
    assert_eq!(
        uuids,
        ["e-call-1", "e-result-1", "e-call-2", "e-call-3", "e-result-2", "e-result-3"]
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
                entries: held([model, thinking]),
            },
            ProviderEvent::Entries {
                of: EntriesOf::Output(1),
                entries: held([text]),
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
            cli.take().await;
            cli.mirror(vec![message.clone(), reminder.clone()]);
            cli.handshake().await;
            call_tool(&cli, "toolu_1", "pwd");
        }
    );
    assert_eq!(
        events[0],
        ProviderEvent::Entries {
            of: EntriesOf::Item(3),
            entries: held([message]),
        }
    );
    assert!(
        matches!(
            events.as_slice(),
            [.., ProviderEvent::ToolCall(_), ProviderEvent::Response(_)]
        ),
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
            assert_eq!(cli.take().await["type"], "user");
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
                entries: held([reminder, call]),
            },
            ProviderEvent::Entries {
                of: EntriesOf::Item(5),
                entries: held([result(reference)]),
            },
            ProviderEvent::Entries {
                of: EntriesOf::Item(6),
                entries: held([folded]),
            },
            ProviderEvent::TextDelta("Done".into()),
            ProviderEvent::Entries {
                of: EntriesOf::Output(0),
                entries: held([answered]),
            },
            ProviderEvent::Response(Some(TokenUsage {
                input_tokens: 2,
                output_tokens: 2,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            })),
        ]
    );
    runtime.close().await;
}

/// Claude Code retries a call whose stream failed without streaming, keeps only
/// the answer it got then, and writes it as one entry of all its blocks.
///
/// Each block keeps the part of that entry that stands for it, and none keeps
/// an entry of the abandoned stream, so a new process resumes the answer once,
/// as the CLI kept it.
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_complete_answer_after_a_failed_stream_is_kept_once_on_its_blocks_and_the_stream_nowhere()
 {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let thinking = json!({ "type": "thinking", "thinking": "Plan the check.", "signature": "sig" });
    let text = json!({ "type": "text", "text": "Let me check the runtime." });
    let call = json!({ "type": "tool_use", "id": "toolu_1", "name": "mcp__main__shell_exec", "input": { "script": "rg demi" } });
    let streamed = assistant_entry("e-streamed", "msg_a", thinking.clone());
    let mut answer = assistant_entry("e-answer", "msg_b", thinking.clone());
    answer["message"]["content"] = json!([thinking.clone(), text.clone(), call.clone()]);
    let (_, mut cli) = tokio::join!(
        all_events(runtime.run(session_request(vec![user("work")], vec![(1, vec![])]))),
        async {
            let mut cli = starts.next().await;
            cli.initialized().await;
            cli.handshake().await;
            cli.say(json!({ "type": "stream_event", "event": { "type": "message_start", "message": { "id": "msg_a" } } }));
            cli.say(json!({ "type": "stream_event", "event": {
                "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "" },
            } }));
            cli.say(json!({ "type": "stream_event", "event": {
                "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "Plan the check." },
            } }));
            cli.say(json!({ "type": "stream_event", "event": {
                "type": "content_block_delta", "index": 0, "delta": { "type": "signature_delta", "signature": "sig" },
            } }));
            cli.say(json!({ "type": "assistant", "message": { "id": "msg_a", "content": [thinking.clone()] } }));
            cli.mirror(vec![streamed.clone()]);
            cli.text("Let me");
            // The stream stalls, and the CLI asks again without streaming.
            cli.say(json!({ "type": "assistant", "message": {
                "id": "msg_b",
                "content": [thinking.clone(), text.clone(), call.clone()],
                "usage": { "input_tokens": 300, "output_tokens": 20 },
            } }));
            cli.mirror(vec![answer.clone()]);
            cli.call("call-1", 1, "toolu_1", "rg demi");
            cli
        }
    );

    let mut items = vec![
        user("work"),
        InferenceItem::AssistantThinking {
            model_id: "claude-test".into(),
            text: "Plan the check.".into(),
            signature: Some("sig".into()),
            kept_past_summary: false,
        },
        InferenceItem::AssistantText {
            model_id: "claude-test".into(),
            text: "Let me check the runtime.".into(),
        },
        tool_use("toolu_1", "rg demi"),
        tool_result("toolu_1", "found"),
    ];
    let blocks = vec![(1, vec![]), (1, vec![]), (1, vec![]), (2, vec![])];
    let (events, ()) = tokio::join!(
        all_events(runtime.run(session_request(items.clone(), blocks))),
        async {
            cli.mcp_reply("call-1").await;
            cli.result(310, 5);
        }
    );
    let kept: Vec<(EntriesOf, Vec<Value>)> = events
        .into_iter()
        .filter_map(|event| match event {
            ProviderEvent::Entries { of, entries } => Some((of, values(&entries))),
            _ => None,
        })
        .collect();
    let part = |content: &Value| {
        let mut part = answer.clone();
        part["message"]["content"] = json!([content]);
        unchained(&part)
    };
    let unchained_kept: Vec<(EntriesOf, Vec<Value>)> = kept
        .iter()
        .map(|(of, entries)| (*of, entries.iter().map(unchained).collect()))
        .collect();
    assert_eq!(
        unchained_kept,
        [
            (EntriesOf::Item(1), vec![part(&thinking)]),
            (EntriesOf::Item(2), vec![part(&text)]),
            (EntriesOf::Item(3), vec![part(&call)]),
        ]
    );

    // A new process resumes those blocks: the answer once, the stream not
    // at all, nothing written over again from the blocks' items.
    items.push(user("next"));
    let entries_of = |index: usize| kept[index].1.clone();
    let blocks = vec![
        (1, vec![]),
        (1, entries_of(0)),
        (1, entries_of(1)),
        (2, entries_of(2)),
        (1, vec![]),
    ];
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
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
    let answered: Vec<(&Value, &Value)> = session
        .iter()
        .filter(|entry| entry["type"] == "assistant")
        .map(|entry| (&entry["message"]["id"], &entry["message"]["content"]))
        .collect();
    let msg_b = json!("msg_b");
    assert_eq!(
        answered,
        [
            (&msg_b, &json!([thinking])),
            (&msg_b, &json!([text])),
            (&msg_b, &json!([call])),
        ]
    );
}

/// The assistant entries a new process resumes after a stream of the stream
/// events `stream` stalled and the complete answer `answer` replaced it, ending
/// in the call `toolu_1`.
///
/// `shown` are the items the agent's blocks of that run hold, each a block of
/// its own but the call, which holds its result too.
async fn resumed_after(stream: Vec<Value>, answer: Value, shown: Vec<InferenceItem>) -> Vec<Value> {
    let provider = provider().await;
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let mut entry = assistant_entry("e-answer", "msg_b", json!({}));
    entry["message"]["content"] = answer.clone();
    let (first, mut cli) = tokio::join!(
        all_events(runtime.run(session_request(vec![user("work")], vec![(1, vec![])]))),
        async {
            let mut cli = starts.next().await;
            cli.initialized().await;
            cli.handshake().await;
            cli.say(json!({ "type": "stream_event", "event": { "type": "message_start", "message": { "id": "msg_a" } } }));
            for event in stream {
                cli.say(json!({ "type": "stream_event", "event": event }));
            }
            // The stream stalls, and the CLI asks again without streaming.
            cli.say(json!({ "type": "assistant", "message": {
                "id": "msg_b", "content": answer, "usage": { "input_tokens": 3, "output_tokens": 2 },
            } }));
            cli.mirror(vec![entry]);
            cli.call("call-1", 1, "toolu_1", "rg demi");
            cli
        }
    );

    let mut items = vec![user("work")];
    items.extend(shown);
    items.push(tool_result("toolu_1", "found"));
    let mut layout: Vec<usize> = vec![1; items.len() - 2];
    layout.push(2);
    let request = |items: &[InferenceItem], layout: &[usize], entries: &[Vec<Value>]| {
        let blocks = layout.iter().copied().zip(entries.iter().cloned()).collect();
        session_request(items.to_vec(), blocks)
    };
    let mut entries = vec![Vec::new(); layout.len()];
    let (second, ()) = tokio::join!(
        all_events(runtime.run(request(&items, &layout, &entries))),
        async {
            cli.mcp_reply("call-1").await;
            cli.result(4, 1);
        }
    );
    // Run 1 named its blocks after the user's; run 2 names them by item.
    for (run, event) in first.into_iter().map(|e| (1, e)).chain(second.into_iter().map(|e| (2, e))) {
        let ProviderEvent::Entries { of, entries: kept } = event else { continue };
        let block = match (run, of) {
            (1, EntriesOf::Output(index)) => index + 1,
            (2, EntriesOf::Item(item)) => item.min(layout.len() - 1),
            other => panic!("unexpected {other:?}"),
        };
        entries[block].extend(values(&kept));
    }

    items.push(user("next"));
    layout.push(1);
    entries.push(Vec::new());
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (_, cli) = tokio::join!(all_events(runtime.run(request(&items, &layout, &entries))), async {
        let mut cli = starts.next().await;
        cli.initialized().await;
        cli.result(1, 1);
        cli
    });
    let session = cli.session();
    chained(&session, SESSION);
    session
        .into_iter()
        .filter(|entry| entry["type"] == "assistant")
        .map(|entry| entry["message"]["content"].clone())
        .collect()
}

fn text_delta(text: &str) -> Value {
    json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": text } })
}

fn text_item(text: &str) -> InferenceItem {
    InferenceItem::AssistantText { model_id: "claude-test".into(), text: text.into() }
}

fn rg_call() -> Value {
    json!({ "type": "tool_use", "id": "toolu_1", "name": "mcp__main__shell_exec", "input": { "script": "rg demi" } })
}

/// What an abandoned stream showed that its complete answer does not
/// repeat stays on the page, and a new process resumes only the answer,
/// as the CLI kept it: here the answer begins with reasoning of its own.
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn text_an_abandoned_stream_showed_is_not_resumed_when_the_answer_begins_otherwise() {
    let thinking = json!({ "type": "thinking", "thinking": "Plan anew.", "signature": "sig" });
    let text = json!({ "type": "text", "text": "new answer" });
    let resumed = resumed_after(
        vec![text_delta("old answer")],
        json!([thinking, text, rg_call()]),
        vec![
            InferenceItem::AssistantText { model_id: "claude-test".into(), text: "old answer".into() },
            InferenceItem::AssistantThinking {
                model_id: "claude-test".into(),
                text: "Plan anew.".into(),
                signature: Some("sig".into()),
                kept_past_summary: false,
            },
            InferenceItem::AssistantText { model_id: "claude-test".into(), text: "new answer".into() },
            tool_use("toolu_1", "rg demi"),
        ],
    )
    .await;
    assert_eq!(resumed, [json!([thinking]), json!([text]), json!([rg_call()])]);
}

/// The same when the answer is a call alone.
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn text_an_abandoned_stream_showed_is_not_resumed_when_the_answer_is_a_call_alone() {
    let resumed = resumed_after(
        vec![text_delta("old answer")],
        json!([rg_call()]),
        vec![
            InferenceItem::AssistantText { model_id: "claude-test".into(), text: "old answer".into() },
            tool_use("toolu_1", "rg demi"),
        ],
    )
    .await;
    assert_eq!(resumed, [json!([rg_call()])]);
}

/// An answer that repeats what streamed keeps each block's entry on the
/// block it repeats, the first text's on the first text, not on the latest
/// block of its kind.
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn each_block_a_complete_answer_repeats_keeps_its_entry_and_is_resumed_once() {
    let thinking = json!({ "type": "thinking", "thinking": "Weigh.", "signature": "sig" });
    let resumed = resumed_after(
        vec![
            text_delta("Intro."),
            json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "thinking", "thinking": "" } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "thinking_delta", "thinking": "Weigh." } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "signature_delta", "signature": "sig" } }),
            text_delta("Then I"),
        ],
        json!([
            { "type": "text", "text": "Intro." },
            thinking,
            { "type": "text", "text": "Then I run it." },
            rg_call(),
        ]),
        vec![
            text_item("Intro."),
            InferenceItem::AssistantThinking {
                model_id: "claude-test".into(),
                text: "Weigh.".into(),
                signature: Some("sig".into()),
                kept_past_summary: false,
            },
            text_item("Then I run it."),
            tool_use("toolu_1", "rg demi"),
        ],
    )
    .await;
    assert_eq!(
        resumed,
        [
            json!([{ "type": "text", "text": "Intro." }]),
            json!([thinking]),
            json!([{ "type": "text", "text": "Then I run it." }]),
            json!([rg_call()]),
        ]
    );
}

/// Text a call being written separates is two blocks, and both stay out
/// of the session when the answer repeats neither.
// Three runs against a scripted CLI: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn texts_an_abandoned_stream_showed_around_a_call_being_written_are_both_left_out() {
    let resumed = resumed_after(
        vec![
            text_delta("First."),
            json!({ "type": "content_block_start", "index": 1, "content_block": {
                "type": "tool_use", "id": "toolu_x", "name": "mcp__main__shell_exec", "input": {},
            } }),
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{}" } }),
            text_delta("Second."),
        ],
        json!([rg_call()]),
        vec![text_item("First."), text_item("Second."), tool_use("toolu_1", "rg demi")],
    )
    .await;
    assert_eq!(resumed, [json!([rg_call()])]);
}

/// `values` as a block holds them.
fn held(values: impl IntoIterator<Item = Value>) -> Vec<demi_shared_types::Entry> {
    values.into_iter().map(demi_shared_types::Entry::from).collect()
}

/// `entries` decoded.
fn values(entries: &[demi_shared_types::Entry]) -> Vec<Value> {
    entries.iter().map(|entry| entry.decode().unwrap()).collect()
}
