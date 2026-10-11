//! Compaction at the socket: the usage the page shows, the `compact` frame
//! the user may send only from half the window, a failed compaction, which
//! is a failure record like any failed request, and what a pass tells the
//! model still runs.

use std::{rc::Rc, sync::Arc};

use demi_agent_server::{
    AgentServer, ServerConfig, ServerDeps,
    testing::{ScriptedProviders, TestClient},
};
use demi_agent_store::{AgentTreeStore, testing::MemoryTreeStore};
use demi_agent_tools::{SubagentSettings, Toolset};
use demi_agent_transcript::testing::SequentialIds;
use demi_conversation_socket_protocol::{ClientFrame, ClientFrameKind, ServerFrame};
use demi_host_interface::CommandSet;
use demi_provider_common::testing::{FixedClock, ScriptedRuntime, Turn, event};
use demi_shared_types::{Block, ContextUsage, EXECUTION_SOURCE, NodeId, SessionPhase};
use serde_json::json;

use crate::support::{
    Fixture, Gate, LiveHosts, Model, ScriptedShells, agent, conversation, held, is_context_usage,
    is_idle, is_pending_steers, kinds, named_node, open, request_text, send, session_of, until,
};

/// The test model's window, which `model_of` gives every scripted model.
const WINDOW: u64 = 100_000;

fn usage(tokens: u64) -> ServerFrame {
    ServerFrame::ContextUsage {
        usage: ContextUsage {
            tokens,
            window: Some(WINDOW),
            compact_from: Some(WINDOW / 2),
        },
    }
}

fn last_usage(frames: &[ServerFrame]) -> Option<&ServerFrame> {
    frames.iter().rfind(|frame| is_context_usage(frame))
}

/// A turn that answers `text` and reports `input_tokens` of usage.
fn answer(text: &str, input_tokens: u64) -> Turn {
    Turn::Events(vec![event::text(text), event::response(input_tokens, 0)])
}

#[tokio::test(flavor = "local")]
async fn the_page_is_told_the_usage_and_compacts_only_from_half_the_window() {
    let script = ScriptedRuntime::new([
        answer("a", 30_000),
        answer("b", 60_000),
        answer("the summary", 10),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.client();

    client.send(open()).await;
    client.next_until(is_pending_steers).await;
    // The usage follows the handshake.
    let opened = client.next_until(is_context_usage).await;
    assert_eq!(opened.last(), Some(&usage(0)));

    client.send(send("m1", "first")).await;
    let turn = client.next_until(is_idle).await;
    assert_eq!(last_usage(&turn), Some(&usage(30_000)));
    client.send(ClientFrame::Compact {}).await;
    assert_eq!(
        client.next().await,
        Some(ServerFrame::Rejected {
            command: ClientFrameKind::Compact,
            reason: "Compaction is available from 50% context usage (now 30%)".into(),
        })
    );

    client.send(send("m2", "second")).await;
    let turn = client.next_until(is_idle).await;
    assert_eq!(last_usage(&turn), Some(&usage(60_000)));
    client.send(ClientFrame::Compact {}).await;
    let compaction = client.next_until(is_idle).await;
    assert!(compaction.contains(&ServerFrame::Phase {
        phase: SessionPhase::Compacting
    }));
    let blocks = session_of(&fixture).transcript().blocks;
    assert_eq!(kinds(&blocks).last().map(String::as_str), Some("compaction_marker"));
    // The summary replaced what the response measured: the usage is the
    // estimate of the blocks from the boundary on, far below the half.
    let Some(ServerFrame::ContextUsage { usage }) = last_usage(&compaction) else {
        panic!("the compaction reported no usage: {compaction:?}");
    };
    assert!(usage.tokens < WINDOW / 2, "{usage:?}");
}

#[tokio::test(flavor = "local")]
async fn a_failed_compaction_is_a_failure_record_that_survives_a_reload_and_its_turn_resumes() {
    let script = ScriptedRuntime::new([
        // The answer reaches the window's threshold, 67,000 tokens, so the
        // turn compacts after it, and the summary request fails for good.
        answer("a", 85_000),
        Turn::Events(vec![event::error("The summary request failed", None)]),
        // Resume compacts first, then finishes the turn.
        answer("the summary", 10),
        answer("done", 10),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;

    client.send(send("m1", "first")).await;
    client.next_until(is_idle).await;

    client.send(ClientFrame::Close {}).await;
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;
    let mut reopened = fixture.client();
    reopened.send(open()).await;
    let handshake = reopened.next_until(is_pending_steers).await;
    let Some(ServerFrame::TranscriptReset { blocks, .. }) = handshake.get(1) else {
        panic!("no transcript in the handshake: {handshake:?}");
    };
    let Some(Block::Error(error)) = blocks.last() else {
        panic!("the transcript does not end with the failure: {blocks:?}");
    };
    assert_eq!(error.message, "The summary request failed");
    // The pass ran inside the turn, which the failure ended.
    assert!(!error.outside_turn);

    reopened.send(ClientFrame::Resume {}).await;
    reopened.next_until(is_idle).await;
    let blocks = session_of(&fixture).transcript().blocks;
    let answer = blocks.iter().rev().find_map(|block| match block {
        Block::Text(text) => Some(text.text.as_str()),
        _ => None,
    });
    assert_eq!(answer, Some("done"), "{:?}", kinds(&blocks));
    assert_eq!(script.remaining(), 0);
}

/// A pass appends a `context` block after its marker that names what the
/// agent still runs: its command left running, by its number and title,
/// and its live subagent, by its number and description; a pass while
/// nothing runs appends none. The next request carries the block.
// One tree, a scripted command and a child held at its first request, on a
// paused clock, which the command's quiet start-up waits on: a few
// milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_pass_names_the_command_and_the_subagent_that_still_run() {
    let model = Model::default();
    model.root([
        answer("nothing runs yet", 10),
        answer("the first summary", 10),
        Turn::Events(vec![event::tool_call(
            "call-1",
            "shell",
            json!({"script": "sleep 600", "description": "Build the release", "intervalMs": null}),
        )]),
        answer("building", 10),
        answer("the second summary", 10),
        answer("still building", 10),
    ]);
    let gate = Gate::new();
    model.child(
        "Check the migrations",
        [held(&gate, vec![event::text("checked"), event::response(1, 1)])],
    );
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let store = MemoryTreeStore::new();
    let server = AgentServer::new(ServerDeps {
        toolsets: Rc::new(Toolset {
            commands: Rc::new(CommandSet::new()),
            revision: Rc::from("none"),
        }),
        subagents: Rc::new(SubagentSettings {
            enabled: true,
            max_live: None,
            max_depth: None,
            profiles: Vec::new(),
        }),
        instructions: Rc::from("system prompt"),
        guide: Rc::from("harness guide"),
        hosts: Rc::new(LiveHosts),
        context: Rc::new([]),
        providers,
        shells: Rc::new(ScriptedShells::default()),
        stores: {
            let store = store.clone();
            Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>)
        },
        clock: Arc::new(FixedClock("2026-10-11T12:00:00.000Z".parse().unwrap())),
        ids: Rc::new(SequentialIds::new("id")),
        config: ServerConfig::default(),
        status_changed: Rc::new(|_| {}),
    });
    let mut client = TestClient::connect(&server, &conversation(), "/workspace");
    client.send(open()).await;
    client.next_until(is_pending_steers).await;
    let session = server
        .node(&conversation(), &conversation())
        .expect("the root is live")
        .session()
        .clone();

    client.send(send("m1", "Look around.")).await;
    client.next_until(is_idle).await;
    session.compact().unwrap().await.unwrap();
    assert_eq!(
        kinds(&session.transcript().blocks).last().map(String::as_str),
        Some("compaction_marker")
    );

    client.send(send("m2", "Build the release.")).await;
    client.next_until(is_idle).await;
    let spawned = agent(
        &server,
        &conversation(),
        "spawn",
        json!({"prompt": "Check the migrations", "description": "Check the migrations"}),
    )
    .await;
    let child = named_node(&store, &spawned);
    let number = store.record(&child).expect("the child is stored").number;
    session.compact().unwrap().await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks[blocks.len() - 2..]),
        ["compaction_marker", "context"]
    );
    let Some(Block::Context(context)) = blocks.last() else {
        unreachable!();
    };
    assert_eq!(context.source, EXECUTION_SOURCE);
    assert_eq!(
        context.text,
        format!(
            "[Still running after compaction]\nCommand 1 (Build the release) is still running.\nSubagent {number} (Check the migrations) is still working."
        )
    );
    client.send(send("m3", "How is it going?")).await;
    until(|| model.is_done()).await;
    let request = model.root_requests().pop().expect("the root asked again");
    assert!(request_text(&request).contains(&context.text), "{}", request_text(&request));
    gate.open();
}
