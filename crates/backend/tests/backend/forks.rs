//! Conversation Fork (`conversation-fork.md` § Backend creation and retries;
//! `web-api.md` § Conversation creation and Fork): a new conversation with the
//! source's history through one of its completed assistant texts, and the
//! edits and command outputs of that history, created once
//! per destination id, while the source runs on and after the backend no
//! longer holds the source. The edits' blobs are the source's: a Fork copies
//! no bytes; and the destination's numbers go on from the source's. No test
//! calls a real model.

use demi_agent_tools::testing::field;
use demi_backend_blobs::counting::ObjectCounts;
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{Block, BlockId, ToolView};
use demi_web_api_protocol::conversations::{ConversationStatus, ForkAnswer};
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{
    FIRST, SECOND, Socket, THIRD, answer, anthropic, choose, create, kinds, last_text, on_device,
    send, summaries, tool_result, tool_use, transcript,
};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD};
use crate::work::edit_sides;

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
    harness.add_user(
        "ana@example.test",
        "ana-pass-1",
        demi_web_api_protocol::auth::Role::User,
    );
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let source_path = format!("/api/conversations/{FIRST}");
    backend
        .patch(&source_path, &master, json!({ "title": "Build" }))
        .await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let raised = backend
        .patch(&source_path, &master, json!({ "thinkingEffort": "high" }))
        .await;
    let settings = raised
        .json::<demi_web_api_protocol::conversations::ConversationUpdate>()
        .conversation
        .model;
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open().await;
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
    let created = backend
        .post(&path, Some(&master), fork(SECOND, &first_text))
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let forked = created.json::<ForkAnswer>();
    let destination = &forked.conversation;
    assert_eq!(
        (
            destination.id.as_str(),
            destination.title.as_str(),
            destination.pinned,
            destination.archived
        ),
        (SECOND, "Build (Fork)", false, false)
    );
    assert_eq!(destination.status, ConversationStatus::Idle);
    assert_eq!(
        destination.model, settings,
        "the destination inherits the source's model settings"
    );
    // A Cloud destination shares the source's directory.
    let directory = format!("/home/demi/sessions/{FIRST}");
    assert_eq!(
        serde_json::to_value(&destination.target).unwrap(),
        json!({ "kind": "cloud", "path": directory })
    );
    assert_eq!(destination.cwd, directory);
    assert_eq!(
        transcript(&backend, &master, SECOND).await.blocks,
        history[..2]
    );
    let listed: Vec<String> = summaries(&backend, &master)
        .await
        .into_iter()
        .map(|summary| summary.id.as_str().to_owned())
        .collect();
    assert_eq!(listed, [SECOND, FIRST], "the destination first");

    // A retry of the attempt finds its destination; the id with another
    // text, or of a conversation no Fork created, is refused.
    let again = backend
        .post(&path, Some(&master), fork(SECOND, &first_text))
        .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json::<ForkAnswer>(), forked);
    let conflict = backend
        .post(&path, Some(&master), fork(SECOND, &second_text))
        .await;
    assert_eq!(
        conflict.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ForkConflict)
    );
    let taken = backend
        .post(&path, Some(&master), fork(FIRST, &first_text))
        .await;
    assert_eq!(
        taken.refusal(),
        (StatusCode::CONFLICT, ErrorCode::IdUnavailable)
    );
    // A block that is no completed assistant text reserves nothing.
    let user_block = history[0].id().clone();
    let not_text = backend
        .post(&path, Some(&master), fork(THIRD, &user_block))
        .await;
    assert_eq!(
        not_text.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidForkTarget)
    );
    create(&backend, &master, THIRD).await;
    for body in [
        json!({ "id": "not-a-uuid", "blockId": first_text }),
        json!({ "id": FOURTH }),
        json!({ "id": FOURTH, "blockId": first_text, "title": "mine" }),
    ] {
        let refused = backend.post(&path, Some(&master), body.clone()).await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{body}"
        );
    }
    // Another user reaches neither the source nor the destination's id.
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let foreign = backend
        .post(&path, Some(&ana), fork(FOURTH, &first_text))
        .await;
    assert_eq!(
        foreign.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound)
    );
    let claimed = backend
        .post("/api/conversations", Some(&ana), json!({ "id": SECOND }))
        .await;
    assert_eq!(
        claimed.refusal(),
        (StatusCode::CONFLICT, ErrorCode::IdUnavailable)
    );

    // The source runs on; the destination goes its own way, replaying only
    // the history it kept.
    source.stop().await;
    let mut destination = Socket::connect(&backend, &master, SECOND).await;
    destination.open().await;
    vendor.respond(answer(&["A3"], 1, 1));
    destination.chat("m4", "U4").await;
    let replayed = vendor.requests()[3].json();
    let messages = replayed["messages"].to_string();
    assert_eq!(
        replayed["messages"].as_array().unwrap().len(),
        3,
        "{replayed}"
    );
    assert!(
        messages.contains("U1") && messages.contains("A1") && messages.contains("U4"),
        "{messages}"
    );
    assert!(!messages.contains("U2"), "{messages}");
    assert_eq!(last_text(&destination.live().await), "A3");
    // A Fork's title is the user's: the destination's first message leaves it.
    let titles: Vec<String> = summaries(&backend, &master)
        .await
        .into_iter()
        .map(|summary| summary.title)
        .collect();
    assert!(titles.contains(&"Build (Fork)".to_owned()), "{titles:?}");
    assert_eq!(
        kinds(&source.live().await),
        [
            "user", "text", "response", "user", "text", "response", "user", "abort"
        ]
    );
    backend.close().await;
}

#[tokio::test]
async fn a_fork_of_a_conversation_the_backend_no_longer_holds_reads_its_stored_history() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let settings = choose(&backend, &master, FIRST, &provider, "claude-opus-4-8")
        .await
        .model;
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open().await;
    vendor.respond(answer(&["A1"], 1, 1));
    source.chat("m1", "U1").await;
    vendor.respond(answer(&["A2"], 1, 1));
    source.chat("m2", "U2").await;
    backend.close().await;

    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let stored = transcript(&backend, &master, FIRST).await.blocks;
    let second_text = texts(&stored)[1].clone();
    let path = format!("/api/conversations/{FIRST}/fork");
    let created = backend
        .post(&path, Some(&master), fork(SECOND, &second_text))
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let forked = created.json::<ForkAnswer>();
    // The first message titled the source.
    assert_eq!(
        (
            forked.conversation.title.as_str(),
            &forked.conversation.model
        ),
        ("U1 (Fork)", &settings)
    );
    // The history through the latest text, without the response after it.
    assert_eq!(
        transcript(&backend, &master, SECOND).await.blocks,
        stored[..5]
    );
    let again = backend
        .post(&path, Some(&master), fork(SECOND, &second_text))
        .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(
        transcript(&backend, &master, FIRST).await.blocks,
        stored,
        "the source is unchanged"
    );
    backend.close().await;
}

// Several seconds: a real device installs the builtin package for the command
// whose edits the Fork keeps.
#[tokio::test]
async fn a_fork_reads_the_edits_its_history_made_from_the_same_blobs_and_writes_no_object() {
    let counts = ObjectCounts::default();
    let vendor = MockVendor::start().await;
    let harness = Harness::new()
        .with_file_package()
        .with_object_counts(&counts);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    // The runner lives as long as its device binding.
    let (_paired, _root) = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open().await;
    let script = "printf 'hello\\n' | demi file create notes.txt";
    vendor.respond(tool_use(
        "toolu_1",
        "shell_exec",
        &json!({ "description": "Notes file", "script": script, "timeoutMs": 60_000 }),
    ));
    vendor.respond(answer(&["Written."], 1, 1));
    source.chat("m1", "Write the notes").await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let files = |blocks: &[Block]| match blocks.get(1) {
        Some(Block::ToolCall(call)) => match &call.view {
            Some(ToolView::Shell(view)) => view.files.clone(),
            view => panic!("{view:?}"),
        },
        block => panic!("{block:?}"),
    };
    let edited = files(&blocks).expect("the command lists what it changed");
    assert_eq!(
        edit_sides(&backend, &master, &edited[0]).await,
        (String::new(), "hello\n".to_owned())
    );

    let text = texts(&blocks).last().unwrap().clone();
    let before = counts.tally();
    let created = backend
        .post(
            &format!("/api/conversations/{FIRST}/fork"),
            Some(&master),
            fork(SECOND, &text),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    // The destination's call names the same blobs, and the Fork put none.
    assert_eq!(counts.tally().since(&before).puts, 0);
    let copied = transcript(&backend, &master, SECOND).await.blocks;
    assert_eq!(files(&copied), Some(edited));
    backend.close().await;
}

/// The command of each shell call of the history, in order.
fn commands(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::ToolCall(call) => match &call.view {
                Some(ToolView::Shell(view)) => Some(view.command_id.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

// Several seconds: four turns run a shell job each on a real device.
#[tokio::test]
async fn a_fork_reads_the_outputs_of_the_commands_its_history_names() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (_paired, _root) = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut source = Socket::connect(&backend, &master, FIRST).await;
    source.open().await;
    let shell = |id: &str, script: &str| {
        tool_use(
            id,
            "shell_exec",
            &json!({ "description": id, "script": script, "timeoutMs": 60_000 }),
        )
    };
    vendor.respond(shell("toolu_before", "seq 1 3"));
    vendor.respond(answer(&["Counted."], 1, 1));
    source.chat("m1", "Count").await;
    vendor.respond(shell("toolu_after", "echo later"));
    vendor.respond(answer(&["Said."], 1, 1));
    source.chat("m2", "Say something").await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let Ok([before, after]) = <[String; 2]>::try_from(commands(&blocks)) else {
        panic!("two commands: {blocks:?}");
    };
    assert_eq!((before.as_str(), after.as_str()), ("1", "2"));
    let counted = texts(&blocks)[0].clone();

    // The destination's history names the first command, whose output
    // `demi shell output` reads there as in the source; the second is not
    // the destination's.
    let created = backend
        .post(
            &format!("/api/conversations/{FIRST}/fork"),
            Some(&master),
            fork(SECOND, &counted),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let mut socket = Socket::connect(&backend, &master, SECOND).await;
    socket.open().await;
    let requests = vendor.requests().len();
    let script = format!("demi shell output {before} --raw; demi shell output {after}");
    vendor.respond(shell("toolu_read", &script));
    vendor.respond(answer(&["Read."], 1, 1));
    socket.chat("m3", "What did it count?").await;
    let read = tool_result(&vendor.requests()[requests + 1].json(), "toolu_read");
    assert!(
        read.contains(&format!(
            "1\n2\n3\ndemi shell output: no command {after} in this conversation"
        )),
        "{read}"
    );
    // The destination goes on from its source's numbers.
    assert_eq!(field(&read, "commandId"), "3");
    backend.close().await;
}
