//! The coding agent through a conversation (`runtime.md` § Sessions and
//! turns, `commands.md`, `sessions-and-targets.md` § Host operations): a
//! conversation that works in a directory of a paired device's real runner,
//! whose model's shell tool runs the `demi` commands the backend offers, the
//! `demi.builtin` package's among them. No test calls a real model.

use demi_agent::testing::model_of;
use demi_core::Block;
use demi_provider::testing::MockVendor;
use serde_json::json;

use crate::conversations::{FIRST, Socket, anthropic, answer, create, kinds, settled, tool_use, transcript};
use crate::support::Harness;

#[tokio::test]
async fn a_conversation_on_a_paired_device_runs_the_coding_agents_commands_there() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_builtin_package();
    let (backend, master) = harness.start_set_up().await;
    let paired = backend.pair(&master, "laptop").await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    // The conversation works in the device's `work` directory, as a target
    // switch would leave it.
    let root = format!("{}/work", paired.runner.home());
    std::fs::create_dir_all(&root).unwrap();
    harness
        .control_database()
        .execute(
            "UPDATE conversations SET target_kind = 'device', target_device_id = ?1, target_path = ?2 WHERE id = ?3",
            rusqlite::params![paired.id(), root, FIRST],
        )
        .unwrap();

    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model_of(&provider, "claude-opus-4-8")).await;
    let script = "printf 'hello from the agent\\n' | demi file create notes.txt && demi host current";
    vendor.respond(tool_use(
        "toolu_1",
        "shell_exec",
        &json!({ "description": "Notes file", "script": script, "timeoutMs": 60_000 }),
    ));
    vendor.respond(answer(&["Written."], 1, 1));
    socket.chat("m1", "Write the notes").await;

    // The native command wrote the file where the conversation works, and
    // the backend's command answered from the conversation's host access.
    assert_eq!(std::fs::read_to_string(format!("{root}/notes.txt")).unwrap(), "hello from the agent\n");
    let requests = vendor.requests();
    let system = requests[0].json()["system"].to_string();
    assert!(system.contains("demi file create") && system.contains("demi host"), "{system}");
    let continued = requests[1].json()["messages"].to_string();
    assert!(continued.contains("Created ") && continued.contains("notes.txt"), "{continued}");
    assert!(continued.contains("laptop"), "the main Host is the device: {continued}");

    settled(&backend, &master, FIRST).await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    // Each request's response follows what it streamed.
    assert_eq!(kinds(&blocks), ["user", "tool_call", "response", "text", "response"]);
    let Some(Block::ToolCall(call)) = blocks.get(1) else {
        unreachable!()
    };
    assert_eq!(call.status.to_string(), "completed", "{call:?}");
    backend.close().await;
}
