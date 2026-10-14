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
use demi_provider_common::{
    ErrorCode,
    testing::{FixedClock, ScriptedRuntime, Turn, event},
};
use demi_shared_types::{
    Block, ContextRowKind, EXECUTION_SOURCE, NodeId, SessionPhase, WORKING_DIRECTORY_SOURCE,
};
use serde_json::json;

use crate::support::{
    Fixture, Gate, LiveHosts, Model, ScriptedShells, agent, conversation, held, is_context_usage,
    is_idle, is_pending_steers, kinds, named_node, open, request_text, scripted_host_server, send,
    session_of, until,
};

/// The test model's window, which `model_of` gives every scripted model.
const WINDOW: u64 = 100_000;

/// The usage a frame tells: its estimate, its window and the estimate the
/// user may compact from.
fn usage(frame: Option<&ServerFrame>) -> Option<(u64, Option<u64>, Option<u64>)> {
    match frame? {
        ServerFrame::ContextUsage { usage: Some(usage), .. } => {
            Some((usage.tokens, usage.window, usage.compact_from))
        }
        _ => None,
    }
}

fn of_window(tokens: u64) -> Option<(u64, Option<u64>, Option<u64>)> {
    Some((tokens, Some(WINDOW), Some(WINDOW / 2)))
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
    // Before any message the estimate is the system prompt's and the
    // tools', all of it the System prompt and Commands rows.
    let Some(ServerFrame::ContextUsage { node, usage: Some(first) }) = opened.last() else {
        panic!("the open reported no usage: {opened:?}");
    };
    assert_eq!(node, &conversation());
    let row_kinds: Vec<_> = first.rows.iter().map(|row| row.kind).collect();
    let prompt: u64 = first.rows[..2].iter().map(|row| row.tokens).sum();
    assert_eq!(
        row_kinds,
        [
            ContextRowKind::SystemPrompt,
            ContextRowKind::Commands,
            ContextRowKind::CompactionBuffer,
            ContextRowKind::FreeSpace
        ]
    );
    assert!(first.tokens > 0 && first.tokens == prompt, "{first:?}");

    client.send(send("m1", "first")).await;
    let turn = client.next_until(is_idle).await;
    assert_eq!(usage(last_usage(&turn)), of_window(30_000));
    client.send(ClientFrame::Compact { node: None }).await;
    assert_eq!(
        client.next().await,
        Some(ServerFrame::Rejected {
            command: ClientFrameKind::Compact,
            reason: "Compaction is available from 50% context usage (now 30%)".into(),
        })
    );

    client.send(send("m2", "second")).await;
    let turn = client.next_until(is_idle).await;
    assert_eq!(usage(last_usage(&turn)), of_window(60_000));
    client.send(ClientFrame::Compact { node: None }).await;
    let compaction = client.next_until(is_idle).await;
    assert!(compaction.contains(&ServerFrame::Phase {
        subagent_id: None,
        phase: SessionPhase::Compacting
    }));
    let blocks = session_of(&fixture).transcript().blocks;
    assert_eq!(kinds(&blocks).last().map(String::as_str), Some("compaction_marker"));
    // The summary replaced what the response measured: the usage is the
    // estimate of the blocks from the boundary on, far below the half.
    let Some(ServerFrame::ContextUsage { usage: Some(usage), .. }) = last_usage(&compaction) else {
        panic!("the compaction reported no usage: {compaction:?}");
    };
    assert!(usage.tokens < WINDOW / 2, "{usage:?}");
}

// One session of five scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_turn_refused_for_its_size_compacts_at_any_share_and_then_resumes() {
    let script = ScriptedRuntime::new([
        answer("a", 30_000),
        // The vendor refuses the second request for its size, at 30% of
        // the window.
        Turn::Events(vec![event::error(
            "Request exceeds the maximum size",
            Some(ErrorCode::RequestTooLarge),
        )]),
        answer("the summary", 10),
        answer("done", 10),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "first")).await;
    client.next_until(is_idle).await;
    client.send(send("m2", "second")).await;
    let failed = client.next_until(is_idle).await;
    assert_eq!(usage(last_usage(&failed)).map(|usage| usage.0 < WINDOW / 2), Some(true));

    // Compact is taken below half the window: the media the vendor refused
    // are summarized away.
    client.send(ClientFrame::Compact { node: None }).await;
    let compaction = client.next_until(is_idle).await;
    assert!(
        compaction.contains(&ServerFrame::Phase {
            subagent_id: None,
            phase: SessionPhase::Compacting
        }),
        "{compaction:?}"
    );
    // The turn is still the unfinished one, and Resume finishes it.
    client.send(ClientFrame::Resume {}).await;
    client.next_until(is_idle).await;

    let blocks = session_of(&fixture).transcript().blocks;
    assert_eq!(
        kinds(&blocks)[kinds(&blocks).len() - 3..],
        ["compaction_marker", "text", "response"]
    );
    assert!(matches!(blocks.iter().rev().nth(1), Some(Block::Text(text)) if text.text == "done"));
    assert_eq!(script.remaining(), 0);
}

#[tokio::test(flavor = "local")]
async fn a_failed_compaction_is_a_failure_record_that_survives_a_reload_and_its_turn_resumes() {
    let script = ScriptedRuntime::new([
        // The answer reaches the window's threshold, 67,000 tokens, so the
        // next send compacts before its turn's request, and the summary
        // request fails for good.
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
    client.send(send("m2", "second")).await;
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
    // The pass ran inside the second turn, which the failure ended.
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
    let (server, store) = tree_server(&model);
    let mut client = TestClient::connect(&server, &conversation());
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

/// Whether `frame` is the usage of `node` with `tokens` in it.
fn is_usage_of(node: &NodeId, tokens: u64) -> impl Fn(&ServerFrame) -> bool + '_ {
    move |frame| matches!(frame, ServerFrame::ContextUsage { node: shown, usage: Some(usage) } if shown == node && usage.tokens == tokens)
}

/// A subagent's session reports its usage as the root's does, naming the
/// child: after its response, and to a page that opens; the child's context
/// card compacts that child with `compact { node }`, and the root keeps its
/// transcript (`subagents.md` § What an agent runs on).
// One tree, a child idle beside the command it left running, on a paused
// clock, which the command's quiet start-up waits on: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_subagent_reports_its_own_usage_and_compact_names_it() {
    let model = Model::default();
    model.child(
        "task serve",
        [
            Turn::Events(vec![
                event::tool_call(
                    "call-1",
                    "shell",
                    json!({"script": "serve", "description": "Start the server", "intervalMs": null}),
                ),
                event::response(55_000, 0),
            ]),
            answer("serving", 60_000),
            answer("the summary", 10),
        ],
    );
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, Rc::new(ScriptedShells::default()), store.clone());
    let mut page = TestClient::connect(&server, &conversation());
    page.send(open()).await;
    page.next_until(is_pending_steers).await;
    let spawned = agent(&server, &conversation(), "spawn", json!({ "prompt": "task serve" })).await;
    let child = named_node(&store, &spawned);

    // Each response's usage names the child.
    page.next_until(is_usage_of(&child, 55_000)).await;
    page.next_until(is_usage_of(&child, 60_000)).await;
    page.next_until(|frame| {
        matches!(frame, ServerFrame::Phase { subagent_id: Some(id), phase: SessionPhase::Idle } if id == &child)
    })
    .await;

    // A page that opens is told the root's usage and the live child's.
    let mut opening = TestClient::connect(&server, &conversation());
    opening.send(open()).await;
    let handshake = opening.next_until(is_usage_of(&child, 60_000)).await;
    let nodes: Vec<&NodeId> = handshake
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::ContextUsage { node, .. } => Some(node),
            _ => None,
        })
        .collect();
    assert_eq!(nodes, [&conversation(), &child]);
    drop(opening);

    page.send(ClientFrame::Compact { node: Some(child.clone()) }).await;
    let compaction = page
        .next_until(|frame| {
            matches!(frame, ServerFrame::Phase { subagent_id: Some(id), phase: SessionPhase::Idle } if id == &child)
        })
        .await;
    assert!(compaction.iter().any(|frame| matches!(frame,
        ServerFrame::Phase { subagent_id: Some(id), phase: SessionPhase::Compacting } if id == &child)),
        "{compaction:?}");
    assert!(compaction.iter().any(|frame| matches!(frame,
        ServerFrame::ContextUsage { node, usage: Some(usage) } if node == &child && usage.tokens < WINDOW / 2)),
        "{compaction:?}");
    let tree = server.tree(&conversation()).expect("the tree is live");
    let child_blocks = tree.node(&child).expect("the child is live").session().transcript().blocks;
    // Its command still runs, so the pass names it after the marker.
    assert_eq!(kinds(&child_blocks[child_blocks.len() - 2..]), ["compaction_marker", "context"]);
    assert!(!kinds(&tree.root().session().transcript().blocks).iter().any(|kind| kind == "compaction_marker"));
    assert!(model.is_done());

    // A node the tree no longer runs is refused.
    page.send(ClientFrame::Compact { node: Some(NodeId::try_from("gone").unwrap()) }).await;
    assert_eq!(
        page.next().await,
        Some(ServerFrame::Rejected {
            command: ClientFrameKind::Compact,
            reason: "The agent has ended".into(),
        })
    );
}

/// A server whose tree `model` answers, its nodes' shells on the scripted
/// Host, whose directory is `/workspace`, and its store.
fn tree_server(model: &Model) -> (Rc<AgentServer<LiveHosts>>, Rc<MemoryTreeStore>) {
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
        hosts: Rc::default(),
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
    (server, store)
}

/// A child learns its working directory before its first request, and
/// again after a pass compacted what told it: the request after the pass
/// carries a `working_directory` block that names the directory.
// One tree and a child whose command runs on, on a paused clock, which the
// command's quiet start-up waits on: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_childs_working_directory_is_told_again_after_a_pass() {
    let model = Model::default();
    model.child(
        "Survey the code",
        [
            // The command runs on, which keeps the child live.
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "serve", "description": "Serve the docs", "intervalMs": null}),
            )]),
            answer("serving", 10),
            answer("Survey the code: the summary", 10),
            answer("still serving", 10),
        ],
    );
    let (server, store) = tree_server(&model);
    let mut client = TestClient::connect(&server, &conversation());
    client.send(open()).await;
    client.next_until(is_pending_steers).await;
    let spawned = agent(
        &server,
        &conversation(),
        "spawn",
        json!({"prompt": "Survey the code", "description": "survey"}),
    )
    .await;
    let child = named_node(&store, &spawned);
    let session = server
        .node(&conversation(), &child)
        .expect("the child is live")
        .session()
        .clone();
    let requests = || model.requests_of("Survey the code");
    while !(requests().len() == 2 && session.phase() == SessionPhase::Idle) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let told = "Your working directory is /workspace: ";
    assert!(request_text(&requests()[0]).contains(told), "{:?}", requests()[0]);

    session.compact().unwrap().await.unwrap();
    let number = store.record(&child).expect("the child is stored").number;
    agent(
        &server,
        &conversation(),
        "send",
        json!({"id": number.to_string(), "message": "How far?"}),
    )
    .await;
    until(|| requests().len() == 4).await;
    let after = requests().pop().unwrap();
    assert!(request_text(&after).contains(told), "{}", request_text(&after));
    let blocks = session.transcript().blocks;
    let boundary = blocks
        .iter()
        .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
        .expect("the pass left its boundary");
    assert!(
        blocks[boundary..].iter().any(|block| matches!(
            block,
            Block::Context(context) if context.source == WORKING_DIRECTORY_SOURCE
                && context.cwd.as_deref() == Some("/workspace")
        )),
        "{:?}",
        kinds(&blocks)
    );
}
