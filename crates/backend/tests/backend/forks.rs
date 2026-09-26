//! Conversation Fork (`conversation-fork.md` § Backend creation and retries;
//! `web-api.md` § Conversation creation and Fork): a new conversation with the
//! source's history through one of its completed assistant texts, and the
//! command storage and edits of that history, created once per destination
//! id, while the source runs on and after the backend no longer holds the
//! source. No test calls a real model.

use demi_agent::testing::model_of;
use demi_core::{Block, BlockId, ToolView};
use demi_web_api::files::ChangeSides;
use demi_provider::testing::{MockResponse, MockVendor};
use demi_web_api::conversations::{ConversationStatus, ForkAnswer};
use demi_web_api::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{
    FIRST, SECOND, Socket, THIRD, anthropic, answer, create, kinds, last_text, on_device, send, settled, summaries,
    tool_use, transcript,
};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD};

/// A destination id no test takes otherwise.
const FOURTH: &str = "9e8d7c6b-5a49-4382-a716-f5e4d3c2b1a0";

fn fork(destination: &str, block: &BlockId) -> Value {
    json!({ "id": destination, "blockId": block })
}

/// The ids of the history's assistant texts, in order.
fn texts(blocks: &[Block]) -> Vec<BlockId> {
    blocks
        .iter()
        .filter(|block| matches!(block, Block::Text(_)))
        .map(|block| block.id().clone())
        .collect()
}

#[tokio::test]
async fn a_fork_keeps_the_history_through_the_chosen_text_while_the_source_runs_on() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("ana@example.test", "ana-pass-1", demi_web_api::auth::Role::User);
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let source_path = format!("/api/conversations/{FIRST}");
    backend.patch(&source_path, &master, json!({ "title": "Build" })).await;
    let model = model_of(&provider, "claude-opus-4-8");
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open(&model).await;
    vendor.respond(answer(&["A1"], 1, 1));
    source.chat("m1", "U1").await;
    vendor.respond(answer(&["A2"], 1, 1));
    source.chat("m2", "U2").await;
    let history = source.live().await;
    let Ok([first_text, second_text]) = <[BlockId; 2]>::try_from(texts(&history)) else {
        panic!("two answers: {history:?}");
    };
    // The source is running its third turn when the Fork arrives.
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    source.send(&send("m3", "U3")).await;
    vendor.received(3).await;

    let path = format!("{source_path}/fork");
    let created = backend.post(&path, Some(&master), fork(SECOND, &first_text)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let forked = created.json::<ForkAnswer>();
    let destination = &forked.conversation;
    assert_eq!(
        (destination.id.as_str(), destination.title.as_str(), destination.pinned, destination.archived),
        (SECOND, "Build (Fork)", false, false)
    );
    assert_eq!(destination.status, ConversationStatus::Idle);
    assert_eq!(forked.model, model, "the destination inherits the source's model");
    // A Cloud destination shares the source's directory.
    let directory = format!("/home/demi/sessions/{FIRST}");
    assert_eq!(serde_json::to_value(&destination.target).unwrap(), json!({ "kind": "cloud", "path": directory }));
    assert_eq!(destination.cwd, directory);
    assert_eq!(transcript(&backend, &master, SECOND).await.blocks, history[..2]);
    let listed: Vec<String> = summaries(&backend, &master)
        .await
        .into_iter()
        .map(|summary| summary.id.as_str().to_owned())
        .collect();
    assert_eq!(listed, [SECOND, FIRST], "the destination first");

    // A retry of the attempt finds its destination; the id with another
    // text, or of a conversation no Fork created, is refused.
    let again = backend.post(&path, Some(&master), fork(SECOND, &first_text)).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json::<ForkAnswer>(), forked);
    let conflict = backend.post(&path, Some(&master), fork(SECOND, &second_text)).await;
    assert_eq!(conflict.refusal(), (StatusCode::CONFLICT, ErrorCode::ForkConflict));
    let taken = backend.post(&path, Some(&master), fork(FIRST, &first_text)).await;
    assert_eq!(taken.refusal(), (StatusCode::CONFLICT, ErrorCode::IdUnavailable));
    // A block that is no completed assistant text reserves nothing.
    let user_block = history[0].id().clone();
    let not_text = backend.post(&path, Some(&master), fork(THIRD, &user_block)).await;
    assert_eq!(not_text.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidForkTarget));
    create(&backend, &master, THIRD).await;
    for body in [
        json!({ "id": "not-a-uuid", "blockId": first_text }),
        json!({ "id": FOURTH }),
        json!({ "id": FOURTH, "blockId": first_text, "title": "mine" }),
    ] {
        let refused = backend.post(&path, Some(&master), body.clone()).await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{body}");
    }
    // Another user reaches neither the source nor the destination's id.
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let foreign = backend.post(&path, Some(&ana), fork(FOURTH, &first_text)).await;
    assert_eq!(foreign.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound));
    let claimed = backend.post("/api/conversations", Some(&ana), json!({ "id": SECOND })).await;
    assert_eq!(claimed.refusal(), (StatusCode::CONFLICT, ErrorCode::IdUnavailable));

    // The source runs on; the destination goes its own way, replaying only
    // the history it kept.
    source.stop().await;
    let mut destination = Socket::connect(&backend, &master, SECOND).await;
    destination.open(&forked.model).await;
    vendor.respond(answer(&["A3"], 1, 1));
    destination.chat("m4", "U4").await;
    let replayed = vendor.requests()[3].json();
    let messages = replayed["messages"].to_string();
    assert_eq!(replayed["messages"].as_array().unwrap().len(), 3, "{replayed}");
    assert!(messages.contains("U1") && messages.contains("A1") && messages.contains("U4"), "{messages}");
    assert!(!messages.contains("U2"), "{messages}");
    assert_eq!(last_text(&destination.live().await), "A3");
    // A Fork's title is the user's: the destination's first message leaves it.
    let titles: Vec<String> = summaries(&backend, &master).await.into_iter().map(|summary| summary.title).collect();
    assert!(titles.contains(&"Build (Fork)".to_owned()), "{titles:?}");
    assert_eq!(kinds(&source.live().await), ["user", "text", "response", "user", "text", "response", "user", "abort"]);
    backend.close().await;
}

#[tokio::test]
async fn a_fork_of_a_conversation_the_backend_no_longer_holds_reads_its_stored_history() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let model = model_of(&provider, "claude-opus-4-8");
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open(&model).await;
    vendor.respond(answer(&["A1"], 1, 1));
    source.chat("m1", "U1").await;
    vendor.respond(answer(&["A2"], 1, 1));
    source.chat("m2", "U2").await;
    settled(&backend, &master, FIRST).await;
    backend.close().await;

    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let stored = transcript(&backend, &master, FIRST).await.blocks;
    let second_text = texts(&stored)[1].clone();
    let path = format!("/api/conversations/{FIRST}/fork");
    let created = backend.post(&path, Some(&master), fork(SECOND, &second_text)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let forked = created.json::<ForkAnswer>();
    // The first message titled the source.
    assert_eq!((forked.conversation.title.as_str(), &forked.model), ("U1 (Fork)", &model));
    // The history through the latest text, without the response after it.
    assert_eq!(transcript(&backend, &master, SECOND).await.blocks, stored[..5]);
    let again = backend.post(&path, Some(&master), fork(SECOND, &second_text)).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(transcript(&backend, &master, FIRST).await.blocks, stored, "the source is unchanged");
    backend.close().await;
}

#[tokio::test]
async fn a_fork_keeps_the_edits_its_history_made_in_a_copy_of_its_own() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_builtin_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    // The runner lives as long as its device binding.
    let (_paired, _root) = on_device(&harness, &backend, &master, FIRST).await;
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open(&model_of(&provider, "claude-opus-4-8")).await;
    let script = "printf 'hello\\n' | demi file create notes.txt";
    vendor.respond(tool_use(
        "toolu_1",
        "shell_exec",
        &json!({ "description": "Notes file", "script": script, "timeoutMs": 60_000 }),
    ));
    vendor.respond(answer(&["Written."], 1, 1));
    source.chat("m1", "Write the notes").await;
    settled(&backend, &master, FIRST).await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let Some(Block::ToolCall(call)) = blocks.get(1) else {
        panic!("{blocks:?}");
    };
    let Some(ToolView::Shell(view)) = &call.view else {
        panic!("{call:?}");
    };
    let files = view.files.clone().unwrap_or_else(|| panic!("the command lists what it changed: {view:?}"));
    assert!(files[0].edits[0].kept, "{files:?}");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([("path", files[0].path.as_str()), ("edit", "0")])
        .finish();
    let edit = |id: &str| format!("/api/conversations/{id}/commands/{}/changes/file?{query}", view.command_id);
    let kept = backend.get(&edit(FIRST), Some(&master)).await;
    assert_eq!(kept.status, StatusCode::OK, "{}", String::from_utf8_lossy(&kept.body));
    let sides = kept.json::<ChangeSides>();
    assert_eq!((sides.original.as_str(), sides.modified.as_str()), ("", "hello\n"));

    let text = texts(&blocks).last().unwrap().clone();
    let created = backend
        .post(&format!("/api/conversations/{FIRST}/fork"), Some(&master), fork(SECOND, &text))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    // The destination reads the edit from its own copy, which outlives the
    // source's objects.
    std::fs::remove_dir_all(harness.data_dir().join("changes").join(FIRST)).unwrap();
    assert_eq!(backend.get(&edit(FIRST), Some(&master)).await.status, StatusCode::NOT_FOUND);
    let copied = backend.get(&edit(SECOND), Some(&master)).await;
    assert_eq!(copied.status, StatusCode::OK, "{}", String::from_utf8_lossy(&copied.body));
    assert_eq!(copied.json::<ChangeSides>(), sides);
    backend.close().await;
}

/// The text of the tool result `id` that `request` carries.
fn tool_result(request: &Value, id: &str) -> String {
    let result = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .find(|block| block["type"] == "tool_result" && block["tool_use_id"] == id)
        .unwrap_or_else(|| panic!("no result of {id}: {request}"));
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_fork_keeps_the_todos_its_history_had() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (_paired, _root) = on_device(&harness, &backend, &master, FIRST).await;
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open(&model_of(&provider, "claude-opus-4-8")).await;
    let shell = |id: &str, script: &str| {
        tool_use(id, "shell_exec", &json!({ "description": id, "script": script, "timeoutMs": 60_000 }))
    };
    vendor.respond(shell("toolu_add", "demi todo add \"first task\""));
    vendor.respond(answer(&["Added."], 1, 1));
    source.chat("m1", "Plan").await;
    vendor.respond(shell("toolu_done", "demi todo done T1"));
    vendor.respond(answer(&["Done."], 1, 1));
    source.chat("m2", "Finish it").await;
    let Ok([added, done]) = <[BlockId; 2]>::try_from(texts(&source.live().await)) else {
        panic!("two answers");
    };

    // A Fork's todos are the ones its history had, however the source went
    // on.
    for (destination, text, status) in [(SECOND, added, "pending"), (THIRD, done, "done")] {
        let created = backend
            .post(&format!("/api/conversations/{FIRST}/fork"), Some(&master), fork(destination, &text))
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
        let forked = created.json::<ForkAnswer>();
        let mut socket = Socket::connect(&backend, &master, destination).await;
        socket.open(&forked.model).await;
        let before = vendor.requests().len();
        vendor.respond(shell("toolu_list", "demi todo list --json"));
        vendor.respond(answer(&["Listed."], 1, 1));
        socket.chat("m3", "What is left?").await;
        let listed = tool_result(&vendor.requests()[before + 1].json(), "toolu_list");
        let todos: Value = serde_json::from_str(listed.lines().last().unwrap()).unwrap_or_else(|_| panic!("{listed}"));
        assert_eq!(todos["todos"][0]["status"], status, "{listed}");
    }
    backend.close().await;
}
