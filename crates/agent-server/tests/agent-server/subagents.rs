//! Subagents at the tree's boundary: the
//! `demi agent` calls a node's jobs make, the frames the root's connection
//! receives, what each model is asked, and the tree store's records.

use std::{cell::RefCell, rc::Rc, time::Duration};

use demi_agent_server::{
    ServerConfig,
    testing::{ScriptedProviders, TestClient},
};
use demi_agent_store::{
    RunningCommand,
    AgentTreeStore, CheckpointState, CheckpointUpdate, ChildDirectory, ClosePhase, NodeClose, NodeRecord, PendingAgentInput, Placement,
    testing::{MemoryTreeStore, test_model, text},
};
use demi_conversation_socket_protocol::{
    AgentRole, ClientFrame, HeldEdge, JobPhase, ServerFrame, ShellStatus, Subagent, TranscriptPatch,
};
use demi_agent_server::testing::{agent_entries, entry_in, phase_is};
use demi_provider_common::{
    InferenceItem, UserPart,
    testing::{Turn, event},
};
use demi_agent_tools::{Profile, ProfileModel, SubagentSettings};
use demi_shared_types::{
    ChangedFile, EditKind, EditSegment, EditedFile, ToolCallBlock, ToolView,
    AgentMessage, AgentMessageEvent, Block, BlockId, CommandId, CompletionId, CompletionOutcome, EXECUTION_SOURCE, NodeId,
    QueuedMessage, Sender, SessionPhase, TextBlock, Timestamp, TurnId, UserBlock,
};
use demi_host_interface::Seen;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::support::{
        AGENT_CALL, CommandRun, Fixture, Gate, LiveHosts, Model, ScriptedShells, TestProduct, agent, agent_call, conversation, held, is_request_output, listed_model, named_node, request_text, scripted_host_server, scripted_host_server_in, send, shell_call, shell_command, texts, until,
    };

pub(crate) fn said(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(1, 1)])
}

pub(crate) fn held_said(gate: &Gate, text: &str) -> Turn {
    held(gate, vec![event::text(text), event::response(1, 1)])
}

pub(crate) fn root() -> NodeId {
    conversation()
}

pub(crate) fn fixture(model: &Model, product: TestProduct) -> Fixture {
    Fixture::with_model(
        model,
        product,
        MemoryTreeStore::new(),
        ServerConfig::default(),
    )
}

pub(crate) async fn spawn(fixture: &Fixture, caller: &NodeId, args: Value) -> NodeId {
    let run = agent(&fixture.server, caller, "spawn", args).await;
    assert_eq!(run.code, 0, "{run:?}");
    named_node(&fixture.store, &run)
}

/// A spawn as a job of `caller`, in a task of its own: a start waits for its
/// parent's provider runtime, which the parent's running turn holds.
pub(crate) fn spawn_during_turn(fixture: &Fixture, caller: &NodeId, args: Value) -> JoinHandle<NodeId> {
    let server = fixture.server.clone();
    let store = fixture.store.clone();
    let caller = caller.clone();
    tokio::task::spawn_local(async move {
        let run = agent(&server, &caller, "spawn", args).await;
        assert_eq!(run.code, 0, "{run:?}");
        named_node(&store, &run)
    })
}

/// The failure a call wrote.
pub(crate) fn refused(run: &CommandRun) -> &str {
    assert_eq!(run.code, 1, "{run:?}");
    &run.stderr
}

/// A change of a child's round, as its entries tell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubagentEvent {
    /// A round runs that the frames had not shown running.
    Started,
    /// A round the frames had shown running, or not at all, is closed.
    Closed,
}

/// Each change of a child's round the frames tell, with the child and its
/// state: a round seen running for the first time starts, and one seen
/// closed after it ran, or first seen closed, closes.
pub(crate) fn lifecycle(frames: &[ServerFrame]) -> Vec<(SubagentEvent, NodeId, JobPhase)> {
    let mut seen: std::collections::HashMap<NodeId, (u64, JobPhase)> = std::collections::HashMap::new();
    let mut events = Vec::new();
    for entry in frames.iter().flat_map(agent_entries) {
        let AgentRole::Subagent(child) = &entry.role else {
            continue;
        };
        let before = seen.insert(entry.id.clone(), (child.round, child.state));
        let running = child.state == JobPhase::Running;
        match before {
            Some((round, state)) if round == child.round && (state == JobPhase::Running) == running => {}
            Some((round, _)) if round == child.round && !running => {
                events.push((SubagentEvent::Closed, entry.id.clone(), child.state));
            }
            _ if running => events.push((SubagentEvent::Started, entry.id.clone(), child.state)),
            None => events.push((SubagentEvent::Closed, entry.id.clone(), child.state)),
            _ => {}
        }
    }
    events
}

/// The child `child` as the frame's entry of it says.
pub(crate) fn child_in<'a>(frame: &'a ServerFrame, child: &NodeId) -> Option<&'a Subagent> {
    match &entry_in(frame, child)?.role {
        AgentRole::Subagent(subagent) => Some(subagent),
        _ => None,
    }
}

/// Whether the frame tells a child running.
pub(crate) fn is_started(frame: &ServerFrame) -> bool {
    agent_entries(frame)
        .iter()
        .any(|entry| matches!(&entry.role, AgentRole::Subagent(child) if child.state == JobPhase::Running))
}

/// The agent messages written into the root's transcript.
pub(crate) fn root_receipts(fixture: &Fixture) -> Vec<AgentMessage> {
    fixture
        .server
        .tree(&root())
        .expect("the tree is live")
        .root()
        .session()
        .transcript()
        .blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::AgentMessage(receipt) => Some(receipt.message),
            _ => None,
        })
        .collect()
}

fn is_receipt_patch(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::TranscriptPatch { patches, .. }
        if patches.iter().any(|patch| matches!(patch, TranscriptPatch::Add { value: Block::AgentMessage(_), .. })))
}

pub(crate) fn is_closed(child: &NodeId) -> impl Fn(&ServerFrame) -> bool + '_ {
    move |frame| child_in(frame, child).is_some_and(|subagent| subagent.state != JobPhase::Running)
}

pub(crate) fn closed_phase(fixture: &Fixture, child: &NodeId) -> Option<ClosePhase> {
    fixture
        .store
        .record(child)
        .and_then(|record| record.closed)
        .map(|close| close.phase)
}

// A page that opens while a child waits for its model's answer is told how
// long the child has waited, which its view of the child counts from instead
// of a clock of its own.
#[tokio::test(flavor = "local")]
async fn a_page_that_opens_while_a_child_waits_for_its_model_is_told_the_wait() {
    let gate = Gate::new();
    let model = Model::default();
    model.root([said("working"), said("got it")]);
    model.child("Read notes.md", [held_said(&gate, "the file says 42")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    client.send(send("m1", "start")).await;
    client.until_idle().await;
    let spawned = agent(
        &fixture.server,
        &root(),
        "spawn",
        json!({ "prompt": "Read notes.md and report its content", "description": "reader" }),
    )
    .await;
    let child = named_node(&fixture.store, &spawned);
    let waits = |frame: &ServerFrame, waiting: bool| {
        entry_in(frame, &child).is_some_and(|entry| entry.waited_ms.is_some() == waiting)
    };
    client.next_until(|frame| waits(frame, true)).await;

    let mut second = fixture.client();
    second.open().await;
    let handshake = second.received();
    assert!(handshake.iter().any(|frame| waits(frame, true)), "{handshake:?}");

    gate.open();
    let frames = client.next_until(is_closed(&child)).await;
    assert!(frames.iter().any(|frame| waits(frame, false)), "{frames:?}");
}

/// A shutdown while a child's start waits for its parent's runtime, which the
/// parent's turn holds and does not give back, ends: every session stops
/// before the shutdown waits for the start, so the parent's turn is cancelled
/// and the start fails as its owner closes. (The shutdown waited for the
/// start, the start for the parent's turn, and the turn for the shutdown's
/// later step that would have cancelled it: the backend never stopped.)
// One shutdown of a scripted tree: a few milliseconds; it hung before.
#[tokio::test(flavor = "local")]
async fn a_shutdown_ends_while_a_childs_start_waits_for_its_parents_runtime() {
    let model = Model::default();
    model.root([Turn::pending()]);
    let fixture = fixture(&model, TestProduct::default());
    let client = fixture.opened().await;
    client.send(send("m1", "start")).await;
    until(|| model.root_requests().len() == 1).await;
    let start = {
        let server = fixture.server.clone();
        tokio::task::spawn_local(async move {
            agent(&server, &root(), "spawn", json!({ "prompt": "Read notes.md", "description": "reader" })).await
        })
    };
    // The start runs up to its wait for the root's runtime.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    tokio::time::timeout(Duration::from_secs(5), fixture.server.shutdown())
        .await
        .expect("the shutdown ends");
    let run = start.await.unwrap();
    assert_ne!(run.code, 0, "{run:?}");
    assert_eq!(model.root_requests().len(), 1);
}

/// Whether `frame` carries the child `child`'s transcript.
fn transcript_of(child: &NodeId) -> impl Fn(&ServerFrame) -> bool + '_ {
    move |frame| {
        matches!(frame, ServerFrame::TranscriptReset { node, .. }
            | ServerFrame::TranscriptPatch { node, .. } if node == child)
    }
}

// A page receives a child's transcript only while it subscribes to the
// child's node (`crates/agent-server/src/server/tree.rs`).
//
// A running child's brief streams to nobody until the page subscribes,
// which answers with the child's transcript and its cache chart, and its
// changes follow from then on. Before, every page received every live
// child's transcript, which made a conversation with large children slow to
// open.
#[tokio::test(flavor = "local")]
async fn a_page_receives_a_childs_transcript_only_once_it_shows_the_child() {
    let gate = Gate::new();
    let model = Model::default();
    model.root([said("working"), said("got it")]);
    model.child("Read notes.md", [held_said(&gate, "the file says 42")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    client.send(send("m1", "start")).await;
    client.until_idle().await;
    let child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "Read notes.md and report its content", "description": "reader" }),
    )
    .await;
    let waits = |frame: &ServerFrame| entry_in(frame, &child).is_some_and(|entry| entry.waited_ms.is_some());
    // The rest of the step that showed the wait comes with it.
    let mut unshown = client.next_until(waits).await;
    unshown.extend(client.received());

    client.subscribe(&child).await;
    let shown = client.received();
    gate.open();
    let followed = client.next_until(is_closed(&child)).await;

    assert!(!unshown.iter().any(transcript_of(&child)), "{unshown:#?}");
    let [ServerFrame::TranscriptReset { blocks, .. }, ServerFrame::CacheChart { node, .. }] = shown.as_slice() else {
        panic!("the child's transcript and chart alone: {shown:#?}")
    };
    assert_eq!(node, &child);
    assert!(brief_of(blocks).contains("Read notes.md"), "{blocks:#?}");
    assert!(
        followed.iter().any(|frame| matches!(frame, ServerFrame::TranscriptPatch { node, .. } if node == &child)),
        "{followed:#?}"
    );
}

// The children a page subscribes to before its `open` are those whose
// transcripts the open's handshake sends; a page that subscribes to none
// opens with the children's entries, and none of their transcripts.
#[tokio::test(flavor = "local")]
async fn an_open_sends_the_transcripts_of_the_children_the_page_shows_alone() {
    let gate = Gate::new();
    let model = Model::default();
    model.root([said("working")]);
    model.child("Read notes.md", [held_said(&gate, "the file says 42")]);
    model.child("Read todo.md", [held_said(&gate, "nothing to do")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    client.send(send("m1", "start")).await;
    client.until_idle().await;
    let notes = spawn(&fixture, &root(), json!({ "prompt": "Read notes.md", "description": "notes" })).await;
    let todo = spawn(&fixture, &root(), json!({ "prompt": "Read todo.md", "description": "todo" })).await;

    let mut showing = fixture.client();
    showing.subscribe(&todo).await;
    showing.open().await;
    let shown = showing.received();
    let mut plain = fixture.client();
    plain.open().await;
    let summaries_only = plain.received();
    gate.open();

    assert!(shown.iter().any(transcript_of(&todo)), "{shown:#?}");
    assert!(!shown.iter().any(transcript_of(&notes)), "{shown:#?}");
    assert!(!summaries_only.iter().any(transcript_of(&notes)), "{summaries_only:#?}");
    assert!(!summaries_only.iter().any(transcript_of(&todo)), "{summaries_only:#?}");
    assert_eq!(lifecycle(&summaries_only).len(), 2, "{summaries_only:#?}");
}

// A page showed a running child, then lost its connection; the child
// finished meanwhile. Opening again holding the child as it last showed it,
// the page receives the child's finished transcript, from its checkpoint, so
// its window no longer ends at the old half answer. A page that holds the
// closed transcript reads nothing again, and one that subscribes to the
// closed child once open receives it too. Before, a closed node answered no
// version, and the window kept what it showed.
#[tokio::test(flavor = "local")]
async fn a_page_that_held_a_child_running_catches_up_with_it_closed() {
    let gate = Gate::new();
    let model = Model::default();
    model.root([said("working"), said("got it")]);
    model.child("Read notes.md", [held_said(&gate, "the file says 42")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut page = fixture.opened().await;
    page.send(send("m1", "start")).await;
    page.until_idle().await;
    let child = spawn(&fixture, &root(), json!({ "prompt": "Read notes.md", "description": "reader" })).await;
    page.next_until(|frame| entry_in(frame, &child).is_some_and(|entry| entry.waited_ms.is_some())).await;
    page.subscribe(&child).await;
    let held = held_edge(&page.received(), &child);
    drop(page);
    let mut observer = fixture.opened().await;
    gate.open();
    observer.next_until(is_closed(&child)).await;

    let mut again = fixture.client();
    again.send(ClientFrame::Subscribe { node: child.clone(), held: Some(held) }).await;
    again.open().await;
    let caught_up = again.received();
    let closed = held_edge(&caught_up, &child);
    assert!(answers_of(&caught_up, &child).contains("the file says 42"), "{caught_up:#?}");

    let mut holding = fixture.client();
    holding.send(ClientFrame::Subscribe { node: child.clone(), held: Some(closed) }).await;
    holding.open().await;
    let unchanged = holding.received();
    assert!(!unchanged.iter().any(transcript_of(&child)), "{unchanged:#?}");

    let mut later = fixture.opened().await;
    later.received();
    later.subscribe(&child).await;
    assert!(answers_of(&later.received(), &child).contains("the file says 42"));
}

/// What a page holds of the node `node` once `frames` reset it: its version,
/// and its latest block as the edge, from there.
fn held_edge(frames: &[ServerFrame], node: &NodeId) -> HeldEdge {
    let reset = frames.iter().rev().find_map(|frame| match frame {
        ServerFrame::TranscriptReset { node: of, start, blocks, version, .. } if of == node => Some((start, blocks, version)),
        _ => None,
    });
    let Some((start, blocks, version)) = reset else {
        panic!("a reset of {node}: {frames:#?}")
    };
    let last = blocks.len().checked_sub(1).expect("a block");
    HeldEdge {
        version: version.clone(),
        from: start + u32::try_from(last).unwrap(),
        edge: blocks[last].id().clone(),
    }
}

/// The text answers the resets of the node `node` in `frames` carry.
fn answers_of(frames: &[ServerFrame], node: &NodeId) -> String {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::TranscriptReset { node: of, blocks, .. } if of == node => Some(blocks),
            _ => None,
        })
        .flatten()
        .filter_map(|block| match block {
            Block::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

/// The text of the user blocks of `blocks`, where a child's brief is.
fn brief_of(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::User(user) => Some(format!("{:?}", user.content)),
            _ => None,
        })
        .collect()
}

/// The children an open's agents list, each as its number, its state and
/// whether it ended.
fn summaries(frames: &[ServerFrame]) -> Vec<(u64, JobPhase, bool)> {
    let lists: Vec<_> = frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::Agents { agents, .. } => Some(agents),
            _ => None,
        })
        .collect();
    let [list] = lists.as_slice() else {
        panic!("one list of the agents: {frames:?}")
    };
    list.iter()
        .filter_map(|entry| match &entry.role {
            AgentRole::Subagent(child) => Some((child.number, child.state, child.ended_at.is_some())),
            _ => None,
        })
        .collect()
}

/// A page whose connection dropped while a child worked opens again.
///
/// Its snapshot lists every child as it is now, the one that ended while it was
/// away and the one that started and ended meanwhile, finished
/// (`crates/agent-server/src/server/tree.rs`). Before, the handshake named only
/// live children, and the page kept showing the first running where it was when
/// the connection dropped.
#[tokio::test(flavor = "local")]
async fn a_reopened_page_is_told_of_the_children_that_ended_while_it_was_away() {
    let gate = Gate::new();
    let model = Model::default();
    model.root([said("working"), said("got it"), said("noted")]);
    model.child("Read notes.md", [held_said(&gate, "the file says 42")]);
    model.child("Count to three", [said("one two three")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut page = fixture.opened().await;
    page.send(send("m1", "start")).await;
    page.until_idle().await;
    let reader = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "Read notes.md and report its content", "description": "reader" }),
    )
    .await;
    page.next_until(is_started)
        .await;
    let mut running = fixture.client();
    running.open().await;
    assert_eq!(summaries(&running.received()), [(1, JobPhase::Running, false)]);
    // The page's connection drops while the reader runs; an observer stays.
    drop(page);
    drop(running);
    let mut observer = fixture.opened().await;
    gate.open();
    observer.next_until(is_closed(&reader)).await;
    let counter = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "Count to three", "description": "counter" }),
    )
    .await;
    observer.next_until(is_closed(&counter)).await;

    let mut again = fixture.client();
    again.open().await;
    let handshake = again.received();

    assert_eq!(
        summaries(&handshake),
        [(1, JobPhase::Completed, true), (2, JobPhase::Completed, true)],
        "{handshake:?}"
    );
    // Neither runs: the list says both closed, and nothing more is told.
    assert_eq!(
        lifecycle(&handshake),
        [
            (SubagentEvent::Closed, reader.clone(), JobPhase::Completed),
            (SubagentEvent::Closed, counter.clone(), JobPhase::Completed)
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn an_inherited_child_starts_from_its_brief_and_its_completion_wakes_the_idle_parent() {
    let model = Model::default();
    model.root([said("working"), said("got it")]);
    model.child("Read notes.md", [said("the file says 42")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    client.send(send("m1", "start")).await;
    client.until_idle().await;

    let empty = agent(
        &fixture.server,
        &root(),
        "spawn",
        json!({ "prompt": " \n " }),
    )
    .await;
    let brief = "Read notes.md and report its content";
    let spawned = agent(
        &fixture.server,
        &root(),
        "spawn",
        json!({ "prompt": brief, "description": "reader" }),
    )
    .await;
    let child = named_node(&fixture.store, &spawned);
    let frames = client.until_idle().await;

    assert_eq!(empty.code, 2, "{empty:?}");
    assert!(empty.stderr.starts_with("error: prompt must not be empty\nUsage: demi agent spawn "), "{empty:?}");
    // Creation, not the child's work, is what the call reports: the
    // conversation's first subagent is agent 1.
    assert_eq!(spawned.stdout, "subagentId: 1\n");
    let started = frames
        .iter()
        .find_map(|frame| child_in(frame, &child).filter(|subagent| subagent.state == JobPhase::Running))
        .expect("the child started");
    assert_eq!(
        (
            &started.parent,
            started.description.as_str(),
            started.profile.as_deref(),
            started.state,
            started.ended_at
        ),
        (&root(), "reader", None, JobPhase::Running, None)
    );
    let closed = frames
        .iter()
        .position(is_closed(&child))
        .expect("the child closed");
    let job = child_in(&frames[closed], &child).expect("its entry");
    assert_eq!(job.state, JobPhase::Completed);
    assert!(job.ended_at.is_some());
    // The receipt only says it finished: the child's answer stays in its own
    // transcript.
    assert_eq!(
        root_receipts(&fixture).last().map(|receipt| receipt.content.as_str()),
        Some("")
    );
    let receipt = frames
        .iter()
        .position(is_receipt_patch)
        .expect("the parent wrote the receipt");
    assert!(closed < receipt, "{frames:#?}");

    // The child was asked with its preamble and its brief, then how its parent
    // hears from it and where it works.
    let asked = model.requests_of("Read notes.md");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].session_id, child.as_str());
    assert!(asked[0].system_prompt.starts_with("system prompt\n"));
    assert!(
        asked[0]
            .system_prompt
            .contains("Operations: spawn, cwd, send, abort, resume, list, show, profiles list, profiles show, profiles create, profiles edit, profiles delete, workflows list, workflows check, workflows use\n")
    );
    let [
        InferenceItem::UserMessage { content },
        InferenceItem::UserMessage { content: reporting },
        InferenceItem::UserMessage { content: directory },
    ] = &*asked[0].items
    else {
        panic!("{:?}", asked[0].items)
    };
    assert!(texts(reporting).starts_with(REPORTING_RULE), "{reporting:?}");
    assert!(
        texts(directory).starts_with("Your working directory is /workspace: "),
        "{directory:?}"
    );
    let [UserPart::Text(preamble), UserPart::Text(first)] = content.as_slice() else {
        panic!("{content:?}")
    };
    assert!(
        preamble
            .starts_with("You are a subagent: agent 1 of this conversation, spawned by agent 0.")
    );
    assert!(preamble.contains("`demi agent spawn` spawns your own children."));
    assert_eq!(first, brief);

    // The parent was told of its end once, as agent input of the request that
    // answered it, with none of the child's words.
    let answered = model.root_requests().pop().unwrap();
    let Some(InferenceItem::UserSteer { content }) = answered.items.last() else {
        panic!("{:?}", answered.items)
    };
    let envelope = texts(content);
    assert!(envelope.contains("\"content\":\"\""), "{envelope}");
    assert!(!envelope.contains("the file says 42"), "{envelope}");
    assert!(envelope.contains("\"outcome\":\"completed\""), "{envelope}");
    let record = fixture.store.record(&child).unwrap();
    assert_eq!(record.parent, Some(root()));
    assert_eq!(
        closed_phase(&fixture, &child),
        Some(ClosePhase::Completed)
    );
    assert!(
        record.delivered,
        "the parent's save marked the round delivered"
    );
    assert_eq!(record.cwd.map(|own| own.cwd).as_deref(), Some("/workspace"));
    let receipts = root_receipts(&fixture);
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].id,
        CompletionId {
            child: child.clone(),
            round: record.round
        }
        .block_id()
    );
    assert_eq!(receipts[0].sender.clone().unwrap().description, "reader");
    assert_eq!(
        receipts[0].event,
        AgentMessageEvent::Completion {
            outcome: CompletionOutcome::Completed
        }
    );
    assert!(model.is_done());
}

#[tokio::test(flavor = "local")]
async fn messages_reach_any_live_agent_while_lifecycle_stays_with_the_spawner() {
    let model = Model::default();
    let (alpha_gate, beta_gate, gamma_gate) = (Gate::new(), Gate::new(), Gate::new());
    model.root([said("noted delta"), said("noted beta")]);
    model.child("task delta", [said("delta done")]);
    model.child(
        "task alpha",
        [held_said(&alpha_gate, "alpha waits"), said("alpha heard")],
    );
    model.child(
        "task beta",
        [
            held_said(&beta_gate, "beta first"),
            Turn::Respond(Box::new(|request| {
                assert!(request_text(request).contains("hello from gamma"));
                vec![event::text("beta heard"), event::response(1, 1)]
            })),
        ],
    );
    model.child("task gamma", [held_said(&gamma_gate, "gamma done")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;

    let delta = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task delta", "description": "delta" }),
    )
    .await;
    client.until_idle().await;
    let alpha = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task alpha", "description": "alpha" }),
    )
    .await;
    let beta = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task beta", "description": "beta" }),
    )
    .await;
    // Alpha's start waits for its runtime until its run ends.
    let gamma_start = tokio::task::spawn_local({
        let server = fixture.server.clone();
        let alpha = alpha.clone();
        async move {
            agent(
                &server,
                &alpha,
                "spawn",
                json!({ "prompt": "task gamma", "description": "gamma" }),
            )
            .await
        }
    });
    alpha_gate.open();
    let gamma = named_node(&fixture.store, &gamma_start.await.unwrap());
    let alpha_node = fixture.server.node(&root(), &alpha).unwrap();
    until(|| alpha_node.session().is_settled()).await;

    let across = agent(
        &fixture.server,
        &gamma,
        "send",
        json!({ "id": "3", "message": "hello from gamma" }),
    )
    .await;
    let to_parent = agent(
        &fixture.server,
        &gamma,
        "send",
        json!({ "id": "parent", "message": "gamma status" }),
    )
    .await;
    until(|| model.requests_of("task alpha").len() == 2).await;
    until(|| alpha_node.session().is_settled()).await;
    let to_self = agent(
        &fixture.server,
        &gamma,
        "send",
        json!({ "id": "4", "message": "me" }),
    )
    .await;
    let from_root = agent(
        &fixture.server,
        &root(),
        "send",
        json!({ "id": "parent", "message": "up" }),
    )
    .await;
    let to_archived = agent(
        &fixture.server,
        &gamma,
        "send",
        json!({ "id": "1", "message": "late" }),
    )
    .await;
    let abort_sibling = agent(&fixture.server, &alpha, "abort", json!({ "id": [3] })).await;
    let resume_foreign = agent(
        &fixture.server,
        &alpha,
        "resume",
        json!({ "id": 1, "message": "again" }),
    )
    .await;
    let listed_current = agent(&fixture.server, &gamma, "list", json!({})).await;
    let listed = agent(&fixture.server, &gamma, "list", json!({ "all": true })).await;
    let listed_json = agent_call(
        &fixture.server,
        &gamma,
        "list",
        json!({ "all": true }),
        true,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let shown = agent(&fixture.server, &alpha, "show", json!({ "id": [3] })).await;
    let show_root = agent(&fixture.server, &alpha, "show", json!({ "id": [0] })).await;
    let show_archived = agent(&fixture.server, &alpha, "show", json!({ "id": [1] })).await;
    let show_archived_json = agent_call(
        &fixture.server,
        &alpha,
        "show",
        json!({ "id": [1] }),
        true,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let show_missing = agent(&fixture.server, &alpha, "show", json!({ "id": [9] })).await;
    // Several ids: each in order, a line for the one that fails, as
    // `cat a missing b` does.
    let show_several = agent(&fixture.server, &alpha, "show", json!({ "id": [3, 9, 1] })).await;

    // Agents are numbered in spawn order: delta 1, alpha 2, beta 3, and
    // gamma, which alpha spawned, 4.
    let numbers: Vec<u64> = [&delta, &alpha, &beta, &gamma]
        .into_iter()
        .map(|node| fixture.number(node))
        .collect();
    assert_eq!(numbers, [1, 2, 3, 4]);
    assert_eq!(across.stdout, "sent to 3\n");
    assert_eq!(to_parent.stdout, "sent to 2\n");
    // The model reads the sender by its number and round, and no id of the
    // message's delivery.
    let alpha_heard = request_text(&model.requests_of("task alpha")[1]);
    assert!(
        alpha_heard.contains(
            r#"{"sender":{"agent":4,"description":"gamma","round":1},"event":"message","#
        ),
        "{alpha_heard}"
    );
    assert!(alpha_heard.contains("gamma status"), "{alpha_heard}");
    assert!(!alpha_heard.contains("recipientId"), "{alpha_heard}");
    assert_eq!(
        refused(&to_self),
        "demi agent send: cannot message your own session\n"
    );
    assert_eq!(
        refused(&from_root),
        "demi agent send: the root session has no parent\n"
    );
    assert_eq!(
        refused(&to_archived),
        "demi agent send: no live agent \"1\" (see `demi agent list`; an archived child is revived only by its parent via resume)\n"
    );
    assert_eq!(
        refused(&abort_sibling),
        "demi agent abort: 3: not one of your running children\n"
    );
    // Agent 1 is the root's child, not alpha's.
    assert_eq!(
        refused(&resume_foreign),
        "demi agent resume: no archived subagent 1 (see `demi agent list`)\n"
    );
    // A table from the root down, each node's children after it, its
    // archived ones only with --all (`crates/shared-types/src/output.rs`).
    let rows = |text: &str| -> Vec<Vec<String>> {
        text.lines()
            .map(|line| line.split_whitespace().map(str::to_owned).collect())
            .collect()
    };
    let header = ["ID", "PARENT", "STATUS", "PROFILE", "EXECUTION", "ELAPSED", "(S)", "IDLE", "(S)", "DESCRIPTION"];
    let expected = [
        &header[..],
        &["0", "-", "-", "-", "-", "-", "-", "-"],
        // Alpha's own child runs: it waits.
        &["2", "0", "running", "-", "waiting", "0", "0", "alpha"],
        &["4", "2", "running", "-", "provider_streaming", "0", "0", "gamma"],
        &["3", "0", "running", "-", "provider_streaming", "0", "0", "beta"],
        &["1", "0", "completed", "-", "-", "-", "-", "delta"],
    ];
    assert_eq!(rows(&listed.stdout), expected, "{}", listed.stdout);
    assert_eq!(rows(&listed_current.stdout), expected[..5], "{}", listed_current.stdout);
    let entries: Value = serde_json::from_str(&listed_json.stdout).unwrap();
    let entries = entries.as_array().unwrap();
    let flat: Vec<(u64, Option<u64>, &str, bool)> = entries
        .iter()
        .map(|entry| {
            (
                entry["id"].as_u64().unwrap(),
                entry["parent"].as_u64(),
                entry["kind"].as_str().unwrap(),
                entry["self"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        flat,
        [
            (0, None, "root", false),
            (2, Some(0), "live", false),
            (4, Some(2), "live", true),
            (3, Some(0), "live", false),
            (1, Some(0), "archived", false),
        ]
    );
    // The JSON holds the live facts the table shows and more.
    assert_eq!(entries[2]["execution"], "provider_streaming");
    assert_eq!(entries[2]["activity"], "streaming");
    assert_eq!(entries[2]["idleMs"], 0);
    assert_eq!(entries[2]["cwd"], ".");
    assert_eq!(entries[4]["status"], "completed");
    assert!(entries[4]["closedAt"].as_str().unwrap().ends_with('Z'));
    // One agent as labelled lines; an instant is RFC 3339 in UTC.
    let instants = |text: &str| -> (Vec<String>, Vec<String>) {
        text.lines().map(str::to_owned).partition(|line| {
            ["Started: ", "Last event: ", "Closed: "].iter().any(|key| line.starts_with(key))
        })
    };
    let (times, lines) = instants(&shown.stdout);
    assert_eq!(times.len(), 2, "{}", shown.stdout);
    for time in &times {
        let (_, instant) = time.split_once(": ").unwrap();
        assert!(instant.parse::<demi_shared_types::Timestamp>().is_ok() && instant.ends_with('Z'), "{time}");
    }
    assert_eq!(
        lines,
        [
            "ID: 3",
            "Parent: 0",
            "Status: running",
            "Profile: -",
            "Description: beta",
            "Directory: /workspace",
            "Elapsed (s): 0",
            "Execution: provider_streaming",
            "Execution for (s): 0",
            "Idle (s): 0",
            "Activity: streaming",
            "Recent tool calls:",
            "  STATUS  STARTED  DURATION (S)  ENDED  TITLE",
            "Last assistant text at: -",
            "Last assistant text: -",
        ]
    );
    assert_eq!(refused(&show_root), "demi agent show: 0: no such subagent\n");
    assert_eq!(refused(&show_missing), "demi agent show: 9: no such subagent\n");
    assert_eq!(
        (refused(&show_several), show_several.stdout.clone()),
        (
            "demi agent show: 9: no such subagent\n",
            format!("{}{}", shown.stdout, show_archived.stdout)
        )
    );
    // An archived child shows how and when it closed, and no result.
    let (times, lines) = instants(&show_archived.stdout);
    assert_eq!(times.len(), 2, "{}", show_archived.stdout);
    assert_eq!(
        lines,
        [
            "ID: 1",
            "Parent: 0",
            "Status: completed",
            "Profile: -",
            "Description: delta",
            "Directory: /workspace",
        ]
    );
    let mut archived: Value = serde_json::from_str(&show_archived_json.stdout).unwrap();
    let object = archived.as_object_mut().unwrap();
    for key in ["startedAt", "closedAt"] {
        assert!(object.remove(key).unwrap().as_str().unwrap().ends_with('Z'));
    }
    assert_eq!(
        archived,
        json!({
            "id": 1,
            "parent": 0,
            "description": "delta",
            "profile": null,
            "status": "completed",
            "cwd": "/workspace",
        })
    );

    // A child with a message waiting does not close before it reads it.
    let beta_session = fixture.server.node(&root(), &beta).unwrap().session().clone();
    beta_gate.open();
    let frames = client.next_until(is_closed(&beta)).await;
    assert!(
        !lifecycle(&frames)
            .iter()
            .any(|(event, id, _)| *event == SubagentEvent::Closed && id != &beta)
    );
    assert_eq!(model.requests_of("task beta").len(), 2);
    assert_eq!(
        closed_phase(&fixture, &beta),
        Some(ClosePhase::Completed)
    );
    client.until_idle().await;
    let senders: Vec<NodeId> = root_receipts(&fixture)
        .into_iter()
        .map(|message| message.sender.clone().unwrap().id)
        .collect();
    assert_eq!(senders, [delta, beta]);
    // Whether a message is a step of its receiver's run: gamma's status is
    // its parent alpha's, as a report from below is; its word to beta, its
    // parent's sibling, ends beta's run, as an instruction does.
    let in_run = |blocks: Vec<Block>, content: &str| {
        blocks.into_iter().find_map(|block| match block {
            Block::AgentMessage(received) if received.message.content == content => Some(received.run.is_some()),
            _ => None,
        })
    };
    assert_eq!(in_run(alpha_node.session().transcript().blocks, "gamma status"), Some(true));
    assert_eq!(in_run(beta_session.transcript().blocks, "hello from gamma"), Some(false));
}

#[tokio::test(flavor = "local")]
async fn abort_closes_the_subtree_and_dispose_detaches_it_for_the_next_open() {
    let model = Model::default();
    let (alpha_gate, gamma_gate, beta_gate) = (Gate::new(), Gate::new(), Gate::new());
    model.root([said("alpha is gone"), said("beta is back")]);
    model.child("task alpha", [held_said(&alpha_gate, "alpha waits")]);
    model.child("task gamma", [held_said(&gamma_gate, "gamma waits")]);
    model.child(
        "task beta",
        [held_said(&beta_gate, "never"), said("beta after restart")],
    );
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    let alpha = spawn(&fixture, &root(), json!({ "prompt": "task alpha" })).await;
    let gamma_start = tokio::task::spawn_local({
        let server = fixture.server.clone();
        let alpha = alpha.clone();
        async move { agent(&server, &alpha, "spawn", json!({ "prompt": "task gamma" })).await }
    });
    alpha_gate.open();
    let gamma = named_node(&fixture.store, &gamma_start.await.unwrap());

    let aborted = agent(
        &fixture.server,
        &root(),
        "abort",
        json!({ "id": [fixture.number(&alpha)] }),
    )
    .await;
    let frames = client.until_idle().await;

    assert_eq!(
        aborted.stdout,
        format!("aborted {}\n", fixture.number(&alpha))
    );
    let closes: Vec<(NodeId, JobPhase)> = lifecycle(&frames)
        .into_iter()
        .filter(|(event, ..)| *event == SubagentEvent::Closed)
        .map(|(_, id, phase)| (id, phase))
        .collect();
    assert_eq!(
        closes,
        [
            (gamma.clone(), JobPhase::Aborted),
            (alpha.clone(), JobPhase::Aborted)
        ]
    );
    let gamma_blocks = fixture.store.checkpoint(&gamma).unwrap().transcript;
    assert!(
        matches!(gamma_blocks.last(), Some(Block::Abort(_))),
        "{gamma_blocks:?}"
    );
    // Alpha keeps its child's completion for a later round, without a turn.
    let alpha_state = fixture.store.checkpoint(&alpha).unwrap().state;
    assert_eq!(alpha_state.phase, SessionPhase::Idle);
    assert_eq!(alpha_state.agent_inputs.len(), 1);
    assert_eq!(model.requests_of("task alpha").len(), 1);
    assert!(fixture.store.record(&gamma).unwrap().delivered);
    let receipts = root_receipts(&fixture);
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        (receipts[0].content.as_str(), &receipts[0].event),
        (
            "",
            &AgentMessageEvent::Completion {
                outcome: CompletionOutcome::Aborted
            }
        )
    );

    // Closing the connection disposes the tree; the live child stays live.
    let beta = spawn(&fixture, &root(), json!({ "prompt": "task beta" })).await;
    until(|| model.requests_of("task beta").len() == 1).await;
    client.send(ClientFrame::Close {}).await;
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;
    let detached = fixture.store.record(&beta).unwrap();
    assert!(detached.closed.is_none());
    assert_eq!(
        fixture.store.checkpoint(&beta).unwrap().state.phase,
        SessionPhase::Running
    );

    let mut reopened = fixture.client();
    reopened.open().await;
    let handshake = reopened.received();
    let frames = reopened.until_idle().await;

    let started: Vec<NodeId> = lifecycle(&[handshake, frames].concat())
        .into_iter()
        .filter(|(event, ..)| *event == SubagentEvent::Started)
        .map(|(_, id, _)| id)
        .collect();
    assert_eq!(started, std::slice::from_ref(&beta));
    assert_eq!(model.requests_of("task beta").len(), 2);
    assert_eq!(
        closed_phase(&fixture, &beta),
        Some(ClosePhase::Completed)
    );
    assert!(model.is_done());
}

/// Whether `frame` is the pages' view of a command of the subagent `child`
/// that `stopping` or has ended as stopped.
fn is_child_command(child: &NodeId, stopping: bool) -> impl Fn(&ServerFrame) -> bool + '_ {
    move |frame| match frame {
        ServerFrame::ShellOutput { node, status } if node == child => match **status {
            ShellStatus::Running { stopping: shown, .. } => stopping && shown,
            ShellStatus::Aborted { .. } => !stopping,
            _ => false,
        },
        _ => false,
    }
}

/// Whether `frame` is the subagent `child`'s phase `phase`.
fn is_child_phase(child: &NodeId, phase: SessionPhase) -> impl Fn(&ServerFrame) -> bool + '_ {
    move |frame| phase_is(frame, child, phase)
}

/// Polls `show`'s view of agent 1 for the root until its execution is
/// `execution`; the time between two looks passes on the paused clock.
pub(crate) async fn wait_for_execution(server: &Rc<demi_agent_server::AgentServer<crate::support::LiveHosts>>, execution: &str) {
    let mut shown = String::new();
    for _ in 0..100 {
        shown = agent(server, &root(), "show", json!({ "id": [1] })).await.stdout;
        if shown.contains(&format!("\nExecution: {execution}\n")) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("agent 1 never reached {execution}:\n{shown}");
}

// A child that ended its turn while its command runs is idle: a message opens
// its next turn at once.
//
// An abort of it closes it at once, although its command's runner never answers
// the stop, as a runner whose thread a macOS privacy prompt held once kept an
// aborted child live for an hour. The command stays in the pages' Running
// window as stopping, any agent can still look at it, and its end reaches the
// pages when its runner reports it
// (`crates/agent-server/src/server/tree/supervisor.rs`).
//
// Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn an_abort_closes_the_child_at_once_while_its_command_still_stops() {
    let model = Model::default();
    model.child(
        "task serve",
        [
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "serve", "description": "Start the server", "intervalMs": null}),
            )]),
            said("serving"),
            said("still serving"),
        ],
    );
    // The aborted child's completion wakes the idle root once.
    model.root([said("noted")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = TestClient::connect(&server, &conversation());
    page.open().await;
    page.next_until(is_request_output).await;
    let spawned = agent(&server, &root(), "spawn", json!({ "prompt": "task serve" })).await;
    let child = named_node(&store, &spawned);
    // The page shows the child, so it hears of the child's commands.
    page.subscribe(&child).await;
    wait_for_execution(&server, "waiting").await;

    // The pages see the child idle beside its command, and a page that
    // opens now is told so in its handshake.
    let frames = page.received();
    assert!(frames.iter().any(is_child_phase(&child, SessionPhase::Running)), "{frames:?}");
    assert!(frames.iter().any(is_child_phase(&child, SessionPhase::Idle)), "{frames:?}");
    let mut opening = TestClient::connect(&server, &conversation());
    opening.open().await;
    let handshake = opening.received();
    assert!(handshake.iter().any(is_child_phase(&child, SessionPhase::Idle)), "{handshake:?}");
    drop(opening);

    // A message to the idle child, whose command still runs, opens its turn.
    let sent = agent(&server, &root(), "send", json!({ "id": "1", "message": "Is it serving?" })).await;
    assert_eq!(sent.stdout, "sent to 1\n", "{sent:?}");
    wait_for_execution(&server, "waiting").await;
    let requests = model.requests_of("task serve");
    assert_eq!(requests.len(), 3);
    assert!(request_text(&requests[2]).contains("Is it serving?"));

    // The abort returns with the child closed, its command still stopping.
    let aborted = tokio::time::timeout(
        Duration::from_secs(60),
        agent(&server, &root(), "abort", json!({ "id": [1] })),
    )
    .await
    .expect("an abort waits for no command's end");
    assert_eq!(aborted.stdout, "aborted 1\n", "{aborted:?}");
    assert_eq!(
        store.record(&child).and_then(|record| record.closed).map(|close| close.phase),
        Some(ClosePhase::Aborted)
    );
    let frames = page.received();
    assert!(frames.iter().any(is_child_command(&child, true)), "{frames:?}");
    assert!(frames.iter().any(is_closed(&child)), "{frames:?}");
    let status = shell_command(&server, &root(), "status", json!({ "id": ["1"] })).await;
    assert!(status.stdout.contains("status: running"), "{status:?}");
    // Arguments that do not go together or do not read are a usage error,
    // exit 2: the error, then the leaf's usage, as clap tells one.
    let refusals = [
        ("status", json!({ "id": ["1"], "interval": "nonsense" }), "--interval nonsense: "),
        ("output", json!({ "id": "1", "stdout": true, "stderr": true }), "--stdout and --stderr do not go together"),
        ("output", json!({ "id": "1", "lines": "2-1" }), "--lines 2-1: "),
        ("output", json!({ "id": "1", "lines": "1-1", "tail": 1 }), "--lines and --tail do not go together"),
    ];
    for (verb, args, error) in refusals {
        let refused = shell_command(&server, &root(), verb, args).await;
        assert_eq!(refused.code, 2, "{refused:?}");
        let (told, usage) = refused.stderr.split_once('\n').expect("two lines");
        assert!(told.starts_with(&format!("error: {error}")), "{refused:?}");
        assert!(usage.starts_with(&format!("Usage: demi shell {verb} ")), "{refused:?}");
    }

    // The page's terminal input and End Command reach it where it stays.
    let command = CommandId::try_from("1").unwrap();
    page.send(ClientFrame::ShellWrite {
        command_id: command.clone(),
        stdin: "q\n".into(),
    })
    .await;
    page.send(ClientFrame::ShellAbort {
        command_id: command.clone(),
    })
    .await;
    let answers = page.received();
    assert!(
        answers.contains(&ServerFrame::ShellWriteResult { command_id: command }),
        "{answers:?}"
    );
    assert!(
        !answers.iter().any(|frame| matches!(frame, ServerFrame::Error { .. })),
        "{answers:?}"
    );

    // A page that opens now finds it stopping in its handshake.
    let mut second = TestClient::connect(&server, &conversation());
    second.subscribe(&child).await;
    second.open().await;
    let handshake = second.received();
    assert!(handshake.iter().any(is_child_command(&child, true)), "{handshake:?}");

    // Its runner reports its end, which reaches the pages.
    shells.last().end();
    page.next_until(is_child_command(&child, false)).await;
    second.next_until(is_child_command(&child, false)).await;
}

// A child starts its tests and ends its turn to wait for their end. They end,
// and the child's `demi shell status 1` shows the end, but the look never
// reaches the child, since its call's stdout broke.
//
// The child is woken with the tests' end report and answers anew, rather than
// being closed with its old "waiting" answer as its result. An end counts as
// seen only once a look showed it to the model
// (`crates/agent-session/src/reports.rs`). The store holds the report's read of
// the stored end until the look is over, so the report is made after the look
// in every run.
//
// Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_status_whose_stdout_broke_leaves_the_end_report() {
    let model = Model::default();
    model.child(
        "task test",
        [
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "test", "description": "Run the tests", "intervalMs": null}),
            )]),
            said("waiting"),
            said("tests passed"),
        ],
    );
    model.root([said("noted")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = TestClient::connect(&server, &conversation());
    page.open().await;
    page.next_until(is_request_output).await;
    let spawned = agent(&server, &root(), "spawn", json!({ "prompt": "task test" })).await;
    let child = named_node(&store, &spawned);
    wait_for_execution(&server, "waiting").await;

    let reads = store.hold_command_ends();
    shells.last().exit_with(Vec::new());
    // Its result is the broken call's, which reaches no one.
    let _ = shell_call(&server, &child, "status", json!({ "id": ["1"] }), CancellationToken::new(), false).await;
    reads.release();
    page.next_until(is_closed(&child)).await;

    let requests = model.requests_of("task test");
    assert_eq!(requests.len(), 3, "the end woke the child");
    let woken = request_text(&requests[2]);
    assert!(woken.contains("Command 1 (Run the tests) ended with exit code 0."), "{woken}");
    assert_eq!(
        store.record(&child).and_then(|record| record.closed).map(|close| close.phase),
        Some(ClosePhase::Completed)
    );
}

// A child reports with `demi agent send parent`, ends its turn to wait for its
// tests, and answers once more when their end wakes it.
//
// The parent holds its report as it was sent and one notice that the child
// finished, which carries no word of the later answer: nothing a child says
// after it reports can replace the report. The child's end is not noticed while
// its tests run.
//
// Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_report_by_message_is_not_replaced_by_the_answer_after_a_wake() {
    let model = Model::default();
    model.child(
        "task test",
        [
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "test", "description": "Run the tests", "intervalMs": null}),
            )]),
            said("waiting"),
            said("The test passed."),
        ],
    );
    model.root([said("noted"), said("noted")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = TestClient::connect(&server, &conversation());
    page.open().await;
    page.next_until(is_request_output).await;
    let spawned = agent(&server, &root(), "spawn", json!({ "prompt": "task test" })).await;
    let child = named_node(&store, &spawned);
    wait_for_execution(&server, "waiting").await;

    let sent = agent(&server, &child, "send", json!({ "id": "parent", "message": "The suite is running." })).await;
    assert_eq!(sent.code, 0, "{sent:?}");
    // The tests run, so the child has not ended.
    assert!(store.record(&child).and_then(|record| record.closed).is_none());
    shells.last().exit_with(Vec::new());
    page.next_until(is_closed(&child)).await;
    until(|| {
        server
            .tree(&root())
            .expect("the tree is live")
            .root()
            .session()
            .transcript()
            .blocks
            .iter()
            .filter(|block| matches!(block, Block::AgentMessage(_)))
            .count()
            == 2
    })
    .await;

    let received: Vec<(AgentMessageEvent, String)> = server
        .tree(&root())
        .expect("the tree is live")
        .root()
        .session()
        .transcript()
        .blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::AgentMessage(receipt) => Some((receipt.message.event, receipt.message.content)),
            _ => None,
        })
        .collect();
    assert_eq!(
        received,
        [
            (AgentMessageEvent::Message {}, "The suite is running.".to_owned()),
            (
                AgentMessageEvent::Completion {
                    outcome: CompletionOutcome::Completed
                },
                String::new()
            ),
        ]
    );
}

// A child that ended its turn to wait for its tests looks at them with `demi
// shell status 1` once they ended, which shows their end. The end report that
// follows tells it nothing more, so it wakes no turn and costs no request, and
// the child closes with its answer (`crates/agent-session/src/reports.rs`).
//
// The store holds the report's read of the stored end until the look is over,
// so the report is made after the look in every run.
//
// Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_status_that_shows_the_end_leaves_no_end_report() {
    let model = Model::default();
    model.child(
        "task test",
        [
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "test", "description": "Run the tests", "intervalMs": null}),
            )]),
            said("waiting"),
            // What a duplicate wake would answer.
            said("woken again"),
        ],
    );
    model.root([said("noted")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = TestClient::connect(&server, &conversation());
    page.open().await;
    page.next_until(is_request_output).await;
    let spawned = agent(&server, &root(), "spawn", json!({ "prompt": "task test" })).await;
    let child = named_node(&store, &spawned);
    wait_for_execution(&server, "waiting").await;

    let reads = store.hold_command_ends();
    shells.last().exit_with(Vec::new());
    let looked = shell_command(&server, &child, "status", json!({ "id": ["1"] })).await;
    assert!(looked.stdout.contains("status: exited\nexitCode: 0"), "{looked:?}");
    reads.release();
    page.next_until(is_closed(&child)).await;

    assert_eq!(model.requests_of("task test").len(), 2, "the shown end woke the child again");
    assert_eq!(
        store.record(&child).and_then(|record| record.closed).map(|close| close.phase),
        Some(ClosePhase::Completed)
    );
}

// The backend restarted while an aborted child's command, and its aborted
// child's, still stopped.
//
// Restore skips the closed children but takes up their commands, which show as
// stopping again under their child, titled by their records, are asked to stop
// again, and end as their runners report
// (`crates/agent-server/src/server/tree/supervisor.rs`).
//
// A page holds no transcript of a closed child, so the frames' title and script
// are all its Running tab has to name such a command by; production showed bare
// numbers there. About 0.02 s.
#[tokio::test(flavor = "local", start_paused = true)]
async fn restore_takes_up_the_commands_closed_children_left_stopping() {
    let store = MemoryTreeStore::new();
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let aborted = NodeClose {
        phase: ClosePhase::Aborted,
        at: Timestamp::UNIX_EPOCH,
    };
    for (id, number, parent, command) in [("stopped", 1, "conversation", "4"), ("nested", 2, "stopped", "5")] {
        let record = child_record(id, number, parent, None);
        let id = record.id.clone();
        store.create_node(record, checkpoint(Vec::new(), Vec::new())).await.unwrap();
        store.close_node(&id, aborted.clone()).await.unwrap();
        store.mark_delivered(&id, 1).await.unwrap();
        store.record_running(
            id,
            RunningCommand {
                command: CommandId::try_from(command).unwrap(),
                job: format!("job-{command}"),
                tool_use_id: format!("call-{command}"),
                turn: None,
                title: "Read the pipe".into(),
                script: "cat pipe".into(),
                interval_ms: None,
                started: Timestamp::UNIX_EPOCH,
                place: Seen::default(),
                unsent: None,
            },
        );
    }
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", Model::default());
    let shells = Rc::new(ScriptedShells::default());
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = TestClient::connect(&server, &conversation());
    let stopped = NodeId::try_from("stopped").unwrap();
    let nested = NodeId::try_from("nested").unwrap();
    // The page shows both children, so it hears of their commands.
    page.subscribe(&stopped).await;
    page.subscribe(&nested).await;
    page.open().await;
    let mut frames = page.received();
    frames.extend(page.next_until(is_child_command(&nested, true)).await);
    assert!(frames.iter().any(is_child_command(&stopped, true)), "{frames:?}");
    let named: Vec<(&str, &str, &str)> = frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::ShellOutput { status, .. } => {
                let command = status.command();
                Some((command.command_id.as_str(), command.title.as_str(), command.script.as_str()))
            }
            _ => None,
        })
        .collect();
    assert!(!named.is_empty(), "{frames:?}");
    assert!(named.iter().all(|(_, title, script)| (*title, *script) == ("Read the pipe", "cat pipe")), "{named:?}");
    let status = shell_command(&server, &root(), "status", json!({ "id": ["4"] })).await;
    assert!(status.stdout.contains("status: running"), "{status:?}");

    // Each runner reports its end, which reaches the page, and a page that
    // opens then finds nothing stopping.
    for shell in shells.0.borrow().iter() {
        shell.end();
    }
    page.next_until(is_child_command(&stopped, false)).await;
    page.next_until(is_child_command(&nested, false)).await;
    let mut later = TestClient::connect(&server, &conversation());
    later.open().await;
    let handshake = later.received();
    assert!(
        !handshake.iter().any(|frame| matches!(frame, ServerFrame::ShellOutput { .. })),
        "{handshake:?}"
    );
}

// The backend restarted in a root's turn, whose call left command 7 running,
// and a call before it command 4. While 4 is taken up again, slowly, 7's end
// arrives.
//
// The restore recorded the interruption before any watch started, so the report
// is written into the interrupted turn, which holds it, and no turn starts.
//
// A few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_report_during_the_take_up_of_an_interrupted_root_starts_no_turn() {
    use demi_agent_store::{CommandInterval, StoredCommand, StoredOutput};
    use demi_host_interface::WholeOutput;
    use demi_shared_types::{CommandEnd, WakeupPlacement};

    let store = MemoryTreeStore::new();
    let mut interrupted = checkpoint(Vec::new(), vec![user("u1", "build and serve")]);
    interrupted.state.phase = SessionPhase::Running;
    interrupted.state.intervals = vec![CommandInterval {
        command_id: CommandId::try_from("7").unwrap(),
        interval_ms: None,
    }];
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), interrupted)
        .await
        .unwrap();
    store.keep_output(
        CommandId::try_from("7").unwrap(),
        StoredCommand {
            title: "Build".into(),
            end: CommandEnd::Exited { exit_code: 0 },
            output: StoredOutput::Stored {
                output: WholeOutput::default(),
            },
        },
    );
    store.record_running(
        root(),
        RunningCommand {
            command: CommandId::try_from("4").unwrap(),
            job: "job-4".into(),
            tool_use_id: "call-4".into(),
            turn: None,
            title: "Serve".into(),
            script: "true".into(),
            interval_ms: None,
            started: Timestamp::UNIX_EPOCH,
            place: Seen::default(),
            unsent: None,
        },
    );
    let model = Model::default();
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let take_up = Gate::new();
    shells.1.replace(Some(take_up.clone()));
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let page = TestClient::connect(&server, &conversation());
    let written = || {
        server.tree(&conversation()).is_some_and(|tree| {
            tree.root()
                .session()
                .transcript()
                .blocks
                .iter()
                .any(|block| matches!(block, Block::Wakeup(_)))
        })
    };
    tokio::join!(page.open(), async {
        until(written).await;
        assert!(model.requests().is_empty(), "a turn started during the take-up");
        take_up.open();
    });

    let tree = server.tree(&conversation()).unwrap();
    tree.root().session().settled().await;
    let blocks = tree.root().session().transcript().blocks;
    let kinds: Vec<&str> = blocks
        .iter()
        .map(|block| match block {
            Block::User(_) => "user",
            Block::Error(_) => "error",
            Block::Wakeup(wakeup) if wakeup.placement == WakeupPlacement::Steer => "wakeup:steer",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["user", "error", "wakeup:steer"]);
    assert!(model.requests().is_empty());
}

pub(crate) fn checkpoint(queue: Vec<QueuedMessage>, blocks: Vec<Block>) -> CheckpointUpdate {
    checkpoint_on(test_model(), queue, blocks)
}

/// A checkpoint whose history was last requested with `model`.
pub(crate) fn checkpoint_on(
    model: demi_shared_types::ModelSelection,
    queue: Vec<QueuedMessage>,
    blocks: Vec<Block>,
) -> CheckpointUpdate {
    CheckpointUpdate {
        state: CheckpointState {
            phase: SessionPhase::Idle,
            queue,
            agent_inputs: Vec::new(),
            reports: Vec::new(),
            next_turn_reports: Vec::new(),
            intervals: Vec::new(),
            model,
            edits: Vec::new(),
            account: None,
            turn_speed: None,
        },
        block_count: blocks.len(),
        moved_from: None,
        version: false,
        changed_blocks: blocks.into_iter().enumerate().collect(),
    }
}

/// A child in its first round, agent `number` of the conversation.
pub(crate) fn child_record(
    id: &str,
    number: u64,
    parent: &str,
    profile: Option<&str>,
) -> NodeRecord {
    NodeRecord {
        id: NodeId::try_from(id).unwrap(),
        number,
        parent: Some(NodeId::try_from(parent).unwrap()),
        description: id.into(),
        profile: profile.map(str::to_owned),
        instructions: None,
        context_limit: None,
        model: Some(test_model()),
        round: 1,
        started_at: Timestamp::UNIX_EPOCH,
        can_spawn_subagents: true,
        closed: None,
        delivered: false,
        forked_from: None,
        started_by: None,
        cwd: Some(ChildDirectory {
            cwd: "/workspace".into(),
            under: Placement {
                device: None,
                directory: "/workspace".into(),
            },
        }),
        side: None,
    }
}

pub(crate) fn user(id: &str, words: &str) -> Block {
    Block::User(UserBlock {
        comments: Vec::new(),
        entries: Vec::new(),
        id: BlockId::try_from(id).unwrap(),
        turn_id: TurnId::try_from(id).unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        request: None,
        model: test_model(),
        content: text(words),
        preamble: None,
        automation: None,
    })
}

pub(crate) fn answer(id: &str, words: &str) -> Block {
    Block::Text(TextBlock {
        entries: Vec::new(),
        id: BlockId::try_from(id).unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        request: None,
        code_files: None,
        pictures: None,
        model: test_model(),
        text: words.into(),
        forkable: true,
    })
}

// The backend opens a conversation whose children all closed and were
// delivered long ago.
//
// What the open reads does not grow with them: the names of the models they
// ran on come from one read of the product, and finding the closed children
// with commands still to take up asks the store nothing per child. Opening one
// with 700 children read the user's provider entries and model catalog 700
// times, and asked the store for each child's commands and children. About
// 0.05 s.
#[tokio::test(flavor = "local", start_paused = true)]
async fn an_open_reads_as_much_for_many_closed_children_as_for_one() {
    async fn reads(children: u64) -> (usize, usize) {
        let store = MemoryTreeStore::new();
        store
            .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
            .await
            .unwrap();
        let completed = NodeClose {
            phase: ClosePhase::Completed,
            at: Timestamp::UNIX_EPOCH,
        };
        for number in 1..=children {
            let record = child_record(&format!("closed-{number}"), number, "conversation", None);
            let id = record.id.clone();
            store.create_node(record, checkpoint(Vec::new(), Vec::new())).await.unwrap();
            store.close_node(&id, completed.clone()).await.unwrap();
            store.mark_delivered(&id, 1).await.unwrap();
        }
        let fixture = Fixture::with_model(&Model::default(), TestProduct::default(), store.clone(), ServerConfig::default());
        let before = store.node_reads();
        fixture.server.restore(&root()).await.unwrap();
        (fixture.product.runs_on_reads.get(), store.node_reads() - before)
    }

    assert_eq!(reads(40).await, reads(1).await);
}

#[tokio::test(flavor = "local")]
async fn a_restore_runs_a_lost_brief_closes_a_quiet_child_and_delivers_a_missed_completion() {
    let store = MemoryTreeStore::new();
    let root_record = NodeRecord::root(root(), Timestamp::UNIX_EPOCH);
    store
        .create_node(root_record, checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    // Lost before its first save: the brief is still queued. It was spawned
    // with a profile the user has deleted since, and restores with the
    // instructions it was spawned with.
    let brief = QueuedMessage {
        comments: Vec::new(),
        id: TurnId::try_from("brief").unwrap(),
        content: text("task lost"),
        automation: None,
    };
    let lost = NodeRecord {
        instructions: Some("retired prompt".into()),
        ..child_record("lost", 1, "conversation", Some("retired"))
    };
    store
        .create_node(lost, checkpoint(vec![brief], Vec::new()))
        .await
        .unwrap();
    // Its final checkpoint saved, its close not yet: it closes with its
    // answer.
    let quiet = checkpoint(
        Vec::new(),
        vec![user("q1", "task quiet"), answer("q2", "quiet result")],
    );
    store
        .create_node(child_record("quiet", 2, "conversation", None), quiet)
        .await
        .unwrap();
    // Closed, its completion not in the parent's checkpoint.
    store
        .create_node(
            child_record("closed", 3, "conversation", None),
            checkpoint(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    let close = NodeClose {
        phase: ClosePhase::Completed,
        at: Timestamp::UNIX_EPOCH,
    };
    let closed = NodeId::try_from("closed").unwrap();
    store.close_node(&closed, close).await.unwrap();
    let model = Model::default();
    model.root([said("noted"), said("noted"), said("noted")]);
    model.child("task lost", [said("lost result")]);
    let fixture = Fixture::with_model(
        &model,
        TestProduct::default(),
        store.clone(),
        ServerConfig::default(),
    );
    // The quiet child reads its own children before its supervisor first
    // looks at it, and runs nothing meanwhile.
    let quiet = NodeId::try_from("quiet").unwrap();
    let reading = store.hold_children_of(&quiet);
    let opening = tokio::task::spawn_local({
        let client = fixture.client();
        async move {
            client.open().await;
            client
        }
    });
    until(|| reading.waiting() == 1).await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    reading.release();
    let mut client = opening.await.unwrap();

    until(|| root_receipts(&fixture).len() == 3).await;
    let tree = fixture.server.tree(&root()).unwrap();
    until(|| tree.is_quiescent()).await;

    let mut senders: Vec<String> = root_receipts(&fixture)
        .into_iter()
        .map(|message| message.sender.clone().unwrap().id.to_string())
        .collect();
    senders.sort();
    assert_eq!(senders, ["closed", "lost", "quiet"]);
    for id in ["lost", "quiet", "closed"] {
        let id = NodeId::try_from(id).unwrap();
        assert_eq!(
            closed_phase(&fixture, &id),
            Some(ClosePhase::Completed)
        );
        assert!(store.record(&id).unwrap().delivered, "{id}");
    }
    assert!(model.requests_of("task quiet").is_empty());
    let restored = &model.requests_of("task lost")[0];
    assert!(restored.system_prompt.starts_with("system prompt\n"));
    assert!(
        restored
            .system_prompt
            .ends_with("Instructions for this subagent:\n\nretired prompt")
    );
    let frames = client.received();
    assert!(frames.iter().any(is_request_output));
}

// A parent holds, as waiting input, the report of a child that finished under
// the release before the notice-only completions: its body is the words the
// child ended with. The restore gives the parent those words once, and does not
// tell it of the child again.
#[tokio::test(flavor = "local")]
async fn a_restore_gives_the_parent_the_report_a_finished_child_left_waiting_for_it() {
    let store = MemoryTreeStore::new();
    let mut parent = checkpoint(Vec::new(), Vec::new());
    parent.state.agent_inputs.push(PendingAgentInput {
        turn_id: TurnId::try_from("subagent:done:1").unwrap(),
        model: test_model(),
        message: AgentMessage {
            id: CompletionId { child: NodeId::try_from("done").unwrap(), round: 1 }.block_id(),
            sender: Some(Sender {
                id: NodeId::try_from("done").unwrap(),
                number: 1,
                description: "done".into(),
                round: 1,
            }),
            recipient_id: root(),
            timestamp: Timestamp::UNIX_EPOCH,
            content: "The cache read is fixed.".into(),
            event: AgentMessageEvent::Completion { outcome: CompletionOutcome::Completed },
            origin: None,
        },
    });
    store.create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), parent).await.unwrap();
    let done = NodeId::try_from("done").unwrap();
    store
        .create_node(child_record("done", 1, "conversation", None), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    store
        .close_node(&done, NodeClose { phase: ClosePhase::Completed, at: Timestamp::UNIX_EPOCH })
        .await
        .unwrap();
    store.mark_delivered(&done, 1).await.unwrap();
    let model = Model::default();
    model.root([said("noted")]);
    let fixture = Fixture::with_model(&model, TestProduct::default(), store.clone(), ServerConfig::default());
    let mut client = fixture.client();
    client.open().await;
    until(|| root_receipts(&fixture).len() == 1).await;
    client.until_idle().await;

    let receipts = root_receipts(&fixture);
    assert_eq!(receipts.len(), 1, "{receipts:?}");
    assert_eq!(receipts[0].content, "The cache read is fixed.");
    let asked = model.root_requests();
    assert_eq!(asked.len(), 1);
    assert!(request_text(&asked[0]).contains("The cache read is fixed."));
}

/// What every subagent is told of how its parent hears from it.
const REPORTING_RULE: &str = "Your parent receives only what you send it with `demi agent send parent`";

// A child tells its parent only by message, and learns that before its first
// request. One that ran under the release before, whose stored history says
// its final answer reaches its parent, and that the parent's message wakes
// without a turn of the user's to carry a new preamble, learns it before its
// next request.
#[tokio::test(flavor = "local")]
async fn a_subagent_is_told_how_its_parent_hears_from_it_whether_new_or_restored() {
    let store = MemoryTreeStore::new();
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let Block::User(mut brief) = user("o1", "task old") else { unreachable!() };
    brief.preamble = Some("A turn you end with your answer returns that answer to the parent as the result.".into());
    let mut old = checkpoint(Vec::new(), vec![Block::User(brief), answer("o2", "halfway")]);
    old.state.agent_inputs.push(PendingAgentInput {
        turn_id: TurnId::try_from("m1").unwrap(),
        model: test_model(),
        message: AgentMessage {
            id: BlockId::try_from("m1").unwrap(),
            sender: Some(Sender { id: root(), number: 0, description: "root session".into(), round: 1 }),
            recipient_id: NodeId::try_from("old").unwrap(),
            timestamp: Timestamp::UNIX_EPOCH,
            content: "Any news?".into(),
            event: AgentMessageEvent::Message {},
            origin: None,
        },
    });
    store.create_node(child_record("old", 1, "conversation", None), old).await.unwrap();
    let model = Model::default();
    model.root([said("noted"), said("noted")]);
    model.child("task old", [said("old done")]);
    model.child("task new", [said("new done")]);
    let fixture = Fixture::with_model(&model, TestProduct::default(), store.clone(), ServerConfig::default());
    let mut client = fixture.client();
    client.open().await;
    until(|| model.requests_of("task old").len() == 1).await;
    spawn(&fixture, &root(), json!({ "prompt": "task new" })).await;
    until(|| model.requests_of("task new").len() == 1).await;

    for prompt in ["task old", "task new"] {
        let text = request_text(&model.requests_of(prompt)[0]);
        assert_eq!(text.matches(REPORTING_RULE).count(), 1, "{prompt}: {text}");
    }
    assert!(!request_text(&model.requests_of("task new")[0]).contains("returns that answer to the parent"));
}

// A restore of two closed children and one edit; well under a second.
#[tokio::test(flavor = "local")]
async fn an_edit_that_leaves_a_childs_start_takes_its_whole_subtree_off_the_current_path() {
    let store = MemoryTreeStore::new();
    store
        .create_node(
            NodeRecord::root(root(), Timestamp::UNIX_EPOCH),
            checkpoint(Vec::new(), vec![user("r1", "start"), answer("r2", "started")]),
        )
        .await
        .unwrap();
    let completed = NodeClose {
        phase: ClosePhase::Completed,
        at: Timestamp::UNIX_EPOCH,
    };
    // The child was started by the root's answer, the grandchild by the
    // child's own: both are on the current path until the edit.
    for (id, number, parent, start, blocks) in [
        ("child", 1, "conversation", "r2", vec![user("c1", "task"), answer("c2", "spawning")]),
        ("grandchild", 2, "child", "c2", vec![user("g1", "subtask"), answer("g2", "done")]),
    ] {
        let record = NodeRecord {
            started_by: Some(BlockId::try_from(start).unwrap()),
            closed: Some(completed.clone()),
            delivered: true,
            ..child_record(id, number, parent, None)
        };
        store.create_node(record, checkpoint(Vec::new(), blocks)).await.unwrap();
    }
    let model = Model::default();
    model.root([said("edited answer")]);
    let fixture = Fixture::with_model(&model, TestProduct::default(), store.clone(), ServerConfig::default());
    let mut client = fixture.opened().await;
    let tree = fixture.server.tree(&root()).unwrap();
    let on_path = || -> Vec<(u64, bool)> {
        tree.agents()
            .1
            .into_iter()
            .filter_map(|entry| match entry.role {
                AgentRole::Subagent(child) => Some((child.number, child.on_path)),
                _ => None,
            })
            .collect()
    };
    assert_eq!(on_path(), [(1, true), (2, true)]);

    let before = crate::support::session_of(&fixture).transcript();
    client
        .send(crate::editing::edit(
            "op1",
            &BlockId::try_from("r1").unwrap(),
            &before.version,
            demi_agent_server::testing::client_text("start over"),
        ))
        .await;
    let frames = client.until_idle().await;

    // The pages hear of both: the grandchild's start is still on its
    // parent's path, but its parent is off the conversation's.
    let changed: Vec<(u64, bool)> = frames
        .iter()
        .filter(|frame| matches!(frame, ServerFrame::Agent { .. }))
        .flat_map(agent_entries)
        .filter_map(|entry| match &entry.role {
            AgentRole::Subagent(child) => Some((child.number, child.on_path)),
            _ => None,
        })
        .collect();
    assert_eq!(changed, [(1, false), (2, false)]);
    assert_eq!(on_path(), [(1, false), (2, false)]);
}

// A restore of one stored child; well under a second.
#[tokio::test(flavor = "local")]
async fn a_child_whose_stored_state_does_not_rebuild_closes_as_failed_with_its_rows_kept() {
    let store = MemoryTreeStore::new();
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let written = vec![user("b1", "task broken"), answer("b2", "half done")];
    store
        .create_node(
            child_record("broken", 1, "conversation", None),
            checkpoint(Vec::new(), written.clone()),
        )
        .await
        .unwrap();
    let broken = NodeId::try_from("broken").unwrap();
    store.corrupt(&broken);
    let model = Model::default();
    model.root([said("noted")]);
    let fixture = Fixture::with_model(&model, TestProduct::default(), store.clone(), ServerConfig::default());
    let _client = fixture.opened().await;

    until(|| root_receipts(&fixture).len() == 1).await;
    // Closed as failed, never deleted: its record and its rows stay.
    assert!(
        matches!(closed_phase(&fixture, &broken), Some(ClosePhase::Error { .. })),
        "{:?}",
        store.record(&broken)
    );
    assert_eq!(store.rows(&broken), written);
}

#[tokio::test(flavor = "local")]
async fn a_start_under_way_outlives_a_cancelled_call() {
    let model = Model::default();
    let root_gate = Gate::new();
    model.root([held_said(&root_gate, "busy"), said("noted")]);
    model.child("first task", [said("first done")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    client.send(send("m1", "work")).await;
    until(|| model.root_requests().len() == 1).await;

    // The call reaches the start, which waits for the busy parent's
    // runtime; then the call is cancelled and dropped.
    let cancel = CancellationToken::new();
    let caller = root();
    let mut call = Box::pin(agent_call(
        &fixture.server,
        &caller,
        "spawn",
        json!({ "prompt": "first task" }),
        false,
        cancel.clone(),
    ));
    assert!(futures_util::poll!(&mut call).is_pending());
    cancel.cancel();
    drop(call);
    root_gate.open();
    let frames = client
        .next_until(|frame| {
            agent_entries(frame)
                .iter()
                .any(|entry| matches!(&entry.role, AgentRole::Subagent(child) if child.state != JobPhase::Running))
        })
        .await;
    client.until_idle().await;

    let child = lifecycle(&frames)[0].1.clone();
    assert_eq!(
        closed_phase(&fixture, &child),
        Some(ClosePhase::Completed)
    );
    assert!(model.is_done());
}

#[tokio::test(flavor = "local")]
async fn the_web_app_aborts_every_live_child() {
    let model = Model::default();
    let gate = Gate::new();
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    for index in 1..=3 {
        model.child(&format!("hold {index}:"), [held_said(&gate, "waits")]);
        spawn(
            &fixture,
            &root(),
            json!({ "prompt": format!("hold {index}: hold") }),
        )
        .await;
    }

    // Each completion may wake the root once.
    model.root((0..3).map(|_| said("stopped")));
    client.send(ClientFrame::AbortSubagents {}).await;
    let tree = fixture.server.tree(&root()).unwrap();
    until(|| tree.is_quiescent()).await;
    let frames = client.received();
    let closes = lifecycle(&frames)
        .into_iter()
        .filter(|(event, _, phase)| *event == SubagentEvent::Closed && *phase == JobPhase::Aborted)
        .count();
    assert_eq!(closes, 3);
    assert_eq!(root_receipts(&fixture).len(), 3);
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_detached_tree_with_a_live_child_is_not_evicted_and_one_without_is() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("child done")]);
    model.child("task long", [held_said(&gate, "long result")]);
    let fixture = fixture(&model, TestProduct::default());
    let client = fixture.opened().await;
    spawn(&fixture, &root(), json!({ "prompt": "task long" })).await;
    drop(client);

    tokio::time::sleep(std::time::Duration::from_secs(700)).await;
    assert!(fixture.server.tree(&root()).is_some());
    gate.open();
    until(|| root_receipts(&fixture).len() == 1).await;
    tokio::time::sleep(std::time::Duration::from_secs(700)).await;
    assert!(fixture.server.tree(&root()).is_none());

    let reopened = fixture.client();
    reopened.open().await;
    let listed = agent(&fixture.server, &root(), "list", json!({ "all": true })).await;
    let rows: Vec<Vec<&str>> = listed.stdout.lines().skip(1).map(|line| line.split_whitespace().collect()).collect();
    assert_eq!(
        rows,
        [
            ["0", "-", "-", "-", "-", "-", "-", "-"],
            ["1", "0", "completed", "-", "-", "-", "-", "-"],
        ],
        "{}",
        listed.stdout
    );
    assert_eq!(root_receipts(&fixture).len(), 1);
}

#[tokio::test(flavor = "local")]
async fn a_child_whose_turn_fails_closes_as_an_error_naming_its_failure_and_a_silent_one_completes_empty() {
    let model = Model::default();
    model.root([said("noted"), said("noted")]);
    model.child(
        "task fails",
        [Turn::Events(vec![event::error("the vendor refused", None)])],
    );
    model.child("task silent", [Turn::Events(vec![event::response(1, 1)])]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;

    let failing = spawn(&fixture, &root(), json!({ "prompt": "task fails" })).await;
    client.next_until(is_closed(&failing)).await;
    let silent = spawn(&fixture, &root(), json!({ "prompt": "task silent" })).await;
    client.next_until(is_closed(&silent)).await;
    until(|| root_receipts(&fixture).len() == 2).await;

    assert_eq!(
        closed_phase(&fixture, &failing),
        Some(ClosePhase::Error {
            failure: "the vendor refused".into()
        })
    );
    assert_eq!(
        closed_phase(&fixture, &silent),
        Some(ClosePhase::Completed)
    );
    let outcomes: Vec<(NodeId, String, AgentMessageEvent)> = root_receipts(&fixture)
        .into_iter()
        .map(|message| (message.sender.clone().unwrap().id, message.content, message.event))
        .collect();
    assert_eq!(
        outcomes,
        [
            (
                failing.clone(),
                "the vendor refused".to_owned(),
                AgentMessageEvent::Completion {
                    outcome: CompletionOutcome::Failed
                }
            ),
            (
                silent,
                String::new(),
                AgentMessageEvent::Completion {
                    outcome: CompletionOutcome::Completed
                }
            ),
        ]
    );
    // The failed child's receipt names its failure record, which its row's
    // name brings into view; the silent one has none to name.
    let failure = fixture
        .store
        .checkpoint(&failing)
        .unwrap()
        .transcript
        .iter()
        .find_map(|block| match block {
            Block::Error(error) => Some(error.id.clone()),
            _ => None,
        });
    assert!(failure.is_some(), "the failed child's transcript records its failure");
    let origins: Vec<Option<BlockId>> = root_receipts(&fixture)
        .into_iter()
        .map(|message| message.origin)
        .collect();
    assert_eq!(origins, [failure, None]);
}

/// A receipt names the block of its sender's transcript it came from, which its
/// row's name brings into view: for an update, the call whose command sent it;
/// a notice that a child finished names none, since the child's answers are
/// no one's.
#[tokio::test(flavor = "local")]
async fn a_message_receipt_names_the_call_that_sent_it_and_a_finished_notice_names_none() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("noted"), said("heard")]);
    model.child(
        "task",
        [
            Turn::Events(vec![
                event::tool_call(AGENT_CALL, "shell", json!({ "script": "demi agent send parent" })),
                event::response(1, 1),
            ]),
            held_said(&gate, "task done"),
        ],
    );
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    let child = spawn(&fixture, &root(), json!({ "prompt": "task" })).await;
    // The call is in the child's transcript, and its turn goes on.
    until(|| model.requests_of("task").len() == 2).await;

    let sent = agent(
        &fixture.server,
        &child,
        "send",
        json!({ "id": "parent", "message": "halfway there" }),
    )
    .await;
    gate.open();
    client.next_until(is_closed(&child)).await;
    until(|| root_receipts(&fixture).len() == 2).await;

    assert_eq!(sent.code, 0, "{sent:?}");
    let blocks = fixture.store.checkpoint(&child).unwrap().transcript;
    let call = blocks.iter().find_map(|block| match block {
        Block::ToolCall(call) if call.tool_use_id == AGENT_CALL => Some(call.id.clone()),
        _ => None,
    });
    let origins: Vec<(AgentMessageEvent, Option<BlockId>)> = root_receipts(&fixture)
        .into_iter()
        .map(|message| (message.event, message.origin))
        .collect();
    assert_eq!(
        origins,
        [
            (AgentMessageEvent::Message {}, call),
            (
                AgentMessageEvent::Completion {
                    outcome: CompletionOutcome::Completed
                },
                None
            ),
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_grandchild_completes_into_its_parent_which_then_completes_into_the_woken_root() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("heard"), said("noted"), said("noted")]);
    model.child(
        "task parent",
        [
            held_said(&gate, "delegating"),
            Turn::Respond(Box::new(|request| {
                let text = request_text(request);
                assert!(text.contains("\"outcome\":\"completed\""), "{text}");
                assert!(!text.contains("grandchild done"), "{text}");
                vec![event::text("parent done"), event::response(1, 1)]
            })),
        ],
    );
    model.child("task grandchild", [said("grandchild done")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    let parent = spawn(&fixture, &root(), json!({ "prompt": "task parent" })).await;

    // A message to the idle root wakes it.
    let status = agent(
        &fixture.server,
        &parent,
        "send",
        json!({ "id": "parent", "message": "status: delegating" }),
    )
    .await;
    client.until_idle().await;
    let grandchild_start =
        spawn_during_turn(&fixture, &parent, json!({ "prompt": "task grandchild" }));
    gate.open();
    let grandchild = grandchild_start.await.unwrap();
    let frames = client.next_until(is_closed(&parent)).await;
    until(|| root_receipts(&fixture).len() == 2).await;

    assert_eq!(status.stdout, "sent to 0\n");
    let closes: Vec<(NodeId, NodeId, JobPhase)> = lifecycle(&frames)
        .into_iter()
        .filter(|(event, ..)| *event == SubagentEvent::Closed)
        .map(|(_, id, state)| {
            let parent = frames.iter().rev().find_map(|frame| child_in(frame, &id)).expect("its entry").parent.clone();
            (id, parent, state)
        })
        .collect();
    assert_eq!(
        closes,
        [
            (grandchild.clone(), parent.clone(), JobPhase::Completed),
            (parent.clone(), root(), JobPhase::Completed)
        ]
    );
    let parent_blocks = fixture.store.checkpoint(&parent).unwrap().transcript;
    let heard: Vec<&AgentMessage> = parent_blocks
        .iter()
        .filter_map(|block| match block {
            Block::AgentMessage(receipt) => Some(&receipt.message),
            _ => None,
        })
        .collect();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].sender.clone().unwrap().id, grandchild);
    let received: Vec<(AgentMessageEvent, String)> = root_receipts(&fixture)
        .into_iter()
        .map(|message| (message.event, message.content))
        .collect();
    assert_eq!(
        received,
        [
            (
                AgentMessageEvent::Message {},
                "status: delegating".to_owned()
            ),
            (
                AgentMessageEvent::Completion {
                    outcome: CompletionOutcome::Completed
                },
                String::new()
            ),
        ]
    );
}

/// The execution context the product gives each node once.
const ON_THE_CLOUD: &str = "The conversation now runs on the Cloud.";

/// How many execution context blocks the node's stored transcript holds.
fn contexts(fixture: &Fixture, node: &NodeId) -> usize {
    fixture
        .store
        .checkpoint(node)
        .unwrap()
        .transcript
        .iter()
        .filter(|block| matches!(block, Block::Context(context) if context.source == EXECUTION_SOURCE))
        .count()
}

#[tokio::test(flavor = "local")]
async fn a_grandchild_inherits_its_parents_profile_and_every_node_reads_the_execution_context_itself()
 {
    let worker = Profile {
        context_limit: Some(500_000),
        name: "worker".into(),
        description: "Works through a task list.".into(),
        model: Some(ProfileModel {
            provider_name: None,
            provider_id: "stub".into(),
            model_id: "worker-model".into(),
            thinking_effort: None,
            service_tier_id: None,
        }),
        instructions: Some("worker prompt".into()),
        can_spawn: true,
        enabled: true,
    };
    let product = TestProduct {
        subagents: RefCell::new(SubagentSettings {
            enabled: true,
            max_live: None,
            max_depth: None,
            profiles: vec![worker],
        }),
        context: RefCell::new(Some(ON_THE_CLOUD.into())),
        ..TestProduct::default()
    };
    let model = Model::default();
    let (outer_gate, inner_gate) = (Gate::new(), Gate::new());
    model.root([said("noted")]);
    model.child(
        "task outer",
        [held_said(&outer_gate, "delegated"), said("outer done")],
    );
    model.child("task inner", [held_said(&inner_gate, "inner done")]);
    let fixture = fixture(&model, product);
    fixture
        .resolver
        .list("stub", vec![listed_model("worker-model", &[], &[])]);
    let mut client = fixture.opened().await;

    let outer = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task outer", "profile": "worker" }),
    )
    .await;
    let inner_start = spawn_during_turn(&fixture, &outer, json!({ "prompt": "task inner" }));
    outer_gate.open();
    let inner = inner_start.await.unwrap();
    let listed = agent(&fixture.server, &inner, "list", json!({})).await;
    inner_gate.open();
    client.next_until(is_closed(&outer)).await;
    client.until_idle().await;

    // Spawned without a profile under the worker, the grandchild runs the
    // worker's prompt, its added instructions included, and model, with the
    // commands every node has.
    let inner_asked = &model.requests_of("task inner")[0];
    assert!(inner_asked.system_prompt.starts_with("system prompt\n"));
    assert!(
        inner_asked
            .system_prompt
            .ends_with("Instructions for this subagent:\n\nworker prompt")
    );
    assert!(inner_asked.system_prompt.contains("demi agent\n"));
    assert!(inner_asked.system_prompt.contains("greet\nGreets the caller by name."));
    assert_eq!(inner_asked.model_id, "worker-model");
    assert_eq!(listed.code, 0, "{listed:?}");
    // Each node reads the execution context itself, once, whatever its
    // prompt: the root reading it later took nothing from its children.
    for node in [&root(), &outer, &inner] {
        assert_eq!(contexts(&fixture, node), 1, "{node}");
    }
    assert!(request_text(inner_asked).contains(ON_THE_CLOUD));
    // With the worker's model comes the worker's Context.
    for node in [&outer, &inner] {
        assert_eq!(fixture.store.record(node).unwrap().context_limit, Some(500_000), "{node}");
    }
    assert!(model.is_done());
}

/// A profile without a model of its own runs its children on their
/// parent's model with their parent's Context (`crates/agent-server/src/server/tree/profiles.rs`),
/// as no profile does.
#[tokio::test(flavor = "local")]
async fn a_profile_without_a_model_of_its_own_gives_its_children_their_parents_context() {
    let worker = Profile {
        name: "worker".into(),
        description: "Works through a task list.".into(),
        model: Some(ProfileModel {
            provider_name: None,
            provider_id: "stub".into(),
            model_id: "worker-model".into(),
            thinking_effort: None,
            service_tier_id: None,
        }),
        context_limit: Some(500_000),
        instructions: None,
        can_spawn: true,
        enabled: true,
    };
    let helper = Profile {
        name: "helper".into(),
        description: "Helps.".into(),
        model: None,
        context_limit: None,
        instructions: Some("helper prompt".into()),
        can_spawn: false,
        enabled: true,
    };
    let product = TestProduct {
        subagents: RefCell::new(SubagentSettings {
            enabled: true,
            max_live: None,
            max_depth: None,
            profiles: vec![helper, worker],
        }),
        ..TestProduct::default()
    };
    let model = Model::default();
    let (outer_gate, inner_gate) = (Gate::new(), Gate::new());
    model.root([said("noted")]);
    model.child(
        "task outer",
        [held_said(&outer_gate, "delegated"), said("outer done")],
    );
    model.child("task inner", [held_said(&inner_gate, "inner done")]);
    let fixture = fixture(&model, product);
    fixture
        .resolver
        .list("stub", vec![listed_model("worker-model", &[], &[])]);
    let mut client = fixture.opened().await;

    let outer = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task outer", "profile": "worker" }),
    )
    .await;
    let inner_start = spawn_during_turn(
        &fixture,
        &outer,
        json!({ "prompt": "task inner", "profile": "helper" }),
    );
    outer_gate.open();
    let inner = inner_start.await.unwrap();
    inner_gate.open();
    client.next_until(is_closed(&outer)).await;
    client.until_idle().await;

    let inner_asked = &model.requests_of("task inner")[0];
    assert_eq!(inner_asked.model_id, "worker-model");
    assert!(inner_asked.system_prompt.ends_with("Instructions for this subagent:\n\nhelper prompt"));
    assert_eq!(fixture.store.record(&inner).unwrap().context_limit, Some(500_000));
    assert!(model.is_done());
}

#[tokio::test(flavor = "local")]
async fn a_reopened_tree_restores_a_childs_own_children_before_the_child_can_settle() {
    let model = Model::default();
    let (outer_gate, never) = (Gate::new(), Gate::new());
    model.root([said("noted")]);
    model.child(
        "task outer",
        [held_said(&outer_gate, "delegated"), said("outer done")],
    );
    model.child(
        "task inner",
        [held_said(&never, "never said"), said("inner done")],
    );
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    let outer = spawn(&fixture, &root(), json!({ "prompt": "task outer" })).await;
    let inner_start = spawn_during_turn(&fixture, &outer, json!({ "prompt": "task inner" }));
    outer_gate.open();
    let inner = inner_start.await.unwrap();
    let outer_session = fixture
        .server
        .node(&root(), &outer)
        .unwrap()
        .session()
        .clone();
    until(|| outer_session.is_settled()).await;

    // The tree goes while the inner child runs and the outer one, idle,
    // waits for it; both stay live in the store.
    client.send(ClientFrame::Close {}).await;
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;
    for node in [&outer, &inner] {
        assert!(
            fixture.store.record(node).unwrap().closed.is_none(),
            "{node}"
        );
    }

    // Reopened, the outer child reads its own children before anything
    // watches it: idle and without a child yet, it does not close.
    let reading = fixture.store.hold_children_of(&outer);
    let opening = tokio::task::spawn_local({
        let reopened = fixture.client();
        async move {
            reopened.open().await;
            reopened
        }
    });
    until(|| reading.waiting() == 1).await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    assert!(fixture.store.record(&outer).unwrap().closed.is_none());
    reading.release();
    let mut reopened = opening.await.unwrap();
    let frames = reopened.next_until(is_closed(&outer)).await;
    reopened.until_idle().await;

    // It waited for the inner child's rerun turn, and closed after it.
    let events: Vec<(SubagentEvent, NodeId)> = lifecycle(&frames)
        .into_iter()
        .map(|(event, node, _)| (event, node))
        .collect();
    assert_eq!(
        events,
        [
            (SubagentEvent::Started, outer.clone()),
            (SubagentEvent::Started, inner.clone()),
            (SubagentEvent::Closed, inner.clone()),
            (SubagentEvent::Closed, outer.clone()),
        ]
    );
    assert_eq!(
        closed_phase(&fixture, &inner),
        Some(ClosePhase::Completed)
    );
    assert_eq!(
        closed_phase(&fixture, &outer),
        Some(ClosePhase::Completed)
    );
    let senders: Vec<NodeId> = root_receipts(&fixture)
        .into_iter()
        .map(|message| message.sender.clone().unwrap().id)
        .collect();
    assert_eq!(senders, [outer]);
    assert!(model.is_done());
}

// One tree, a root and a child with three scripted runs: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_stop_of_the_parent_leaves_its_child_running_and_the_childs_completion_wakes_it() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([Turn::pending(), said("read the result")]);
    model.child("task alpha", [held_said(&gate, "alpha's result")]);
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    let child = spawn(&fixture, &root(), json!({ "prompt": "task alpha" })).await;
    until(|| model.requests_of("task alpha").len() == 1).await;
    client.send(send("m1", "anything else?")).await;
    until(|| model.root_requests().len() == 1).await;

    client.send(ClientFrame::Abort {}).await;
    client
        .next_until(|frame| matches!(frame, ServerFrame::AbortResult { .. }))
        .await;

    let tree = fixture.server.tree(&root()).unwrap();
    let blocks = tree.root().session().transcript().blocks;
    assert!(matches!(blocks.last(), Some(Block::Abort(_))), "{blocks:?}");
    // The Stop was the root's own: the child runs on.
    assert!(fixture.store.record(&child).unwrap().closed.is_none());
    gate.open();
    client.next_until(is_closed(&child)).await;
    // Its completion wakes the stopped root as it would an idle one.
    until(|| model.root_requests().len() == 2).await;
    client.until_idle().await;
    assert_eq!(root_receipts(&fixture).len(), 1);
    assert!(model.is_done());
}

/// A child works in its own directory only while the conversation works where
/// it did when that was set.
///
/// Once the conversation's target is another, which the test changes in its
/// store without a word to the tree:
///
/// - the child's next job starts in the conversation's directory;
/// - its next request tells it so;
/// - its record is as it was.
///
/// A page that hears of the move learns where the child works.
// One tree, a child kept live by the command it left running, on a paused
// clock, which the command's quiet start-up waits on: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_child_works_in_the_conversations_directory_once_the_conversation_moved_with_no_write() {
    let model = Model::default();
    let resident = |id: &str| {
        Turn::Events(vec![event::tool_call(
            id,
            "shell",
            json!({"script": "serve", "description": "Serve", "intervalMs": null}),
        )])
    };
    model.child(
        "task stay",
        [
            resident("c1"),
            said("serving"),
            resident("c2"),
            said("serving again"),
        ],
    );
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let store = MemoryTreeStore::new();
    let shells = Rc::new(ScriptedShells::default());
    let hosts = Rc::new(LiveHosts::default());
    let server = scripted_host_server_in(providers, shells.clone(), store.clone(), hosts.clone());
    let mut page = TestClient::connect(&server, &root());
    page.open().await;
    page.next_until(is_request_output).await;
    let spawned = agent(&server, &root(), "spawn", json!({ "prompt": "task stay" })).await;
    let child = named_node(&store, &spawned);
    let session = server.node(&root(), &child).expect("the child is live").session().clone();
    while !(model.requests_of("task stay").len() == 2 && session.phase() == SessionPhase::Idle) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let stored = store.record(&child).expect("the child is stored").cwd;

    hosts.move_to("/elsewhere");
    let number = store.record(&child).expect("the child is stored").number;
    agent(&server, &root(), "send", json!({"id": number.to_string(), "message": "Again."})).await;
    while model.requests_of("task stay").len() < 4 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let started: Vec<Option<String>> = shells.0.borrow().iter().flat_map(|shell| shell.started_in.borrow().clone()).collect();
    assert_eq!(started, [Some("/workspace".to_owned()), Some("/elsewhere".to_owned())]);
    let asked = request_text(&model.requests_of("task stay")[2]);
    assert!(asked.contains("Your working directory is /elsewhere: "), "{asked}");
    assert_eq!(store.record(&child).expect("the child is stored").cwd, stored);

    server.placement_changed(&root()).await;
    let frames = page
        .next_until(|frame| child_in(frame, &child).is_some_and(|subagent| subagent.cwd == "/elsewhere"))
        .await;
    let job = frames.last().and_then(|frame| child_in(frame, &child)).expect("its entry");
    assert_eq!(job.cwd, "/elsewhere");
}

/// A shell call's lists hold only what its agent changed in the working
/// directory its call started in (`crates/agent-store/src/changes.rs`).
///
/// A child that works in `/workspace/sub` writes there and in the
/// conversation's `/workspace`, and its pills and lists name its own
/// directory's file alone. Its call keeps the other in its record, though the
/// conversation moved, and the child's directory with it, while the call ran.
// One restored child on a paused clock: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_childs_lists_hold_only_what_it_changed_in_its_own_directory() {
    let store = MemoryTreeStore::new();
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let brief = QueuedMessage {
        comments: Vec::new(),
        id: TurnId::try_from("brief").unwrap(),
        content: text("task edit"),
        automation: None,
    };
    let editor = NodeRecord {
        cwd: Some(ChildDirectory {
            cwd: "/workspace/sub".into(),
            under: Placement {
                device: Some("scripted".into()),
                directory: "/workspace".into(),
            },
        }),
        ..child_record("editor", 1, "conversation", None)
    };
    store.create_node(editor, checkpoint(vec![brief], Vec::new())).await.unwrap();
    let model = Model::default();
    model.root([said("noted")]);
    let edit = Turn::Events(vec![event::tool_call(
        "c1",
        "shell",
        json!({"script": "edit", "description": "Edit", "intervalMs": 600000}),
    )]);
    model.child("task edit", [edit, said("edited")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let hosts = Rc::new(LiveHosts::default());
    let server = scripted_host_server_in(providers, shells.clone(), store.clone(), hosts.clone());
    let page = TestClient::connect(&server, &root());
    page.open().await;
    until(|| shells.0.borrow().iter().any(|shell| !shell.started_in.borrow().is_empty())).await;
    assert_eq!(*shells.last().started_in.borrow(), [Some("/workspace/sub".to_owned())]);
    hosts.move_to("/elsewhere");

    let written = |path: &str| EditedFile {
        path: path.into(),
        kind: EditKind::Added,
        from: None,
        added: 1,
        removed: 0,
        outside: false,
        edits: vec![EditSegment { copies: None, binary: false }],
    };
    shells
        .last()
        .exit_with(vec![written("/workspace/b.ts"), written("/workspace/sub/a.ts")]);
    until(|| model.requests_of("task edit").len() == 2).await;

    let editor = NodeId::try_from("editor").unwrap();
    let transcript = store.checkpoint(&editor).expect("the child is stored").transcript;
    let view = transcript
        .iter()
        .find_map(|block| match block {
            Block::ToolCall(ToolCallBlock {
                view: Some(ToolView::Shell(view)),
                ..
            }) => Some(view.clone()),
            _ => None,
        })
        .expect("the child's call has its view");
    let outside: Vec<(&str, bool)> = view
        .files
        .iter()
        .flatten()
        .map(|file| (file.path.as_str(), file.outside))
        .collect();
    assert_eq!(outside, [("/workspace/b.ts", true), ("/workspace/sub/a.ts", false)]);
    let paths = |files: &Option<Vec<ChangedFile>>| -> Vec<String> {
        files.iter().flatten().map(|file| file.path.clone()).collect()
    };
    assert_eq!(paths(&view.run_files), ["/workspace/sub/a.ts"]);
    assert_eq!(paths(&view.request_files), ["/workspace/sub/a.ts"]);
}
