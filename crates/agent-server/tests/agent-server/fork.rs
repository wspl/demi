//! `demi agent fork` at the tree's boundary
//! (`crates/agent-server/src/server/tree/supervisor.rs`): a new child of the
//! caller from what one of its children already knows, on the profile the
//! caller names, with the caller's message.

use std::rc::Rc;

use demi_agent_server::{ServerConfig, testing::ScriptedProviders};
use demi_agent_session::testing::COMPACTION_SUMMARY_INSTRUCTION;
use demi_agent_store::{AgentTreeStore, ClosePhase, testing::MemoryTreeStore};
use demi_agent_tools::{Profile, ProfileModel};
use demi_provider_common::{
    InferenceRequest,
    testing::{Turn, event},
};
use demi_shared_types::{Block, NodeId, ProviderModel};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::subagents::{
    closed_phase, is_closed, refused, root, root_receipts, said, spawn,
    wait_for_execution,
};
use crate::support::{
    CommandRun, Fixture, Gate, Model, ScriptedShells, TestProduct, agent, agent_call,
    conversation, held, listed_model, named_node, request_text, scripted_host_server,
    shell_command, streamed_then_held, until,
};

/// A profile on the parent's model.
fn research() -> Profile {
    Profile {
        context_limit: None,
        name: "research".into(),
        description: "Finds out why.".into(),
        model: None,
        instructions: None,
        can_spawn: true,
        enabled: true,
    }
}

/// A profile on the provider `other`'s `other-model`.
fn frontend() -> Profile {
    Profile {
        context_limit: None,
        name: "frontend".into(),
        description: "Writes the web app.".into(),
        model: Some(ProfileModel {
            provider_name: None,
            provider_id: "other".into(),
            model_id: "other-model".into(),
            thinking_effort: None,
            service_tier_id: None,
        }),
        ..research()
    }
}

/// A server of `model` on the provider `stub` and `other` on `other`, whose
/// catalog lists `other-model` as `listed`, for a user with the profiles
/// `research` and `frontend`.
fn fixture(model: &Model, other: &Model, listed: ProviderModel) -> Fixture {
    let product = TestProduct::default();
    product.subagents.borrow_mut().profiles = vec![research(), frontend()];
    let fixture = Fixture::with_model(model, product, MemoryTreeStore::new(), ServerConfig::default());
    fixture.resolver.provide_runtime("other", other.clone());
    fixture.resolver.list("other", vec![listed]);
    fixture
}

async fn fork(fixture: &Fixture, args: Value) -> CommandRun {
    agent(&fixture.server, &root(), "fork", args).await
}

/// The live session of the node `id`.
fn session(fixture: &Fixture, id: &NodeId) -> demi_agent_session::AgentSession {
    fixture
        .server
        .node(&conversation(), id)
        .expect("the node is live")
        .session()
        .clone()
}

fn is_summary_request(request: &InferenceRequest) -> bool {
    request_text(request).contains(COMPACTION_SUMMARY_INSTRUCTION)
}

// The root forks its archived `research` child onto `frontend`: the fork
// is a new child that answers on frontend's model from research's history
// and the root's message, and reports to the root; the source is as it was.
#[tokio::test(flavor = "local")]
async fn a_fork_of_an_archived_child_continues_its_history_on_another_profiles_model() {
    let model = Model::default();
    model.root([said("noted"), said("noted")]);
    model.child("task trace the estimate", [said("The card reads a stale cache.")]);
    let other = Model::default();
    other.child("task trace the estimate", [said("Fixed the cache read.")]);
    let fixture = fixture(&model, &other, listed_model("other-model", &[], &[]));
    let mut client = fixture.opened().await;
    let source = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task trace the estimate", "profile": "research", "description": "trace the estimate" }),
    )
    .await;
    client.next_until(is_closed(&source)).await;
    until(|| fixture.store.record(&source).unwrap().delivered).await;

    let forked = agent_call(
        &fixture.server,
        &root(),
        "fork",
        json!({ "id": 1, "profile": "frontend", "message": "Fix it as you described." }),
        true,
        CancellationToken::new(),
    )
    .await
    .expect("the call is dispatched");
    assert_eq!(forked.stdout, "{\"subagentId\":2}\n", "{forked:?}");
    let child = fixture.store.numbered(2).unwrap();
    client.next_until(is_closed(&child)).await;

    // It asked frontend's model once, with research's history and then the
    // message.
    let asked = other.requests_of("task trace the estimate");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].model_id, "other-model");
    let text = request_text(&asked[0]);
    let history = text.find("The card reads a stale cache.").expect("the history is replayed");
    let message = text.find("Fix it as you described.").expect("the message is sent");
    assert!(history < message, "{text}");
    // A new sibling of its source, with the source's description and
    // ` (fork)`, on frontend.
    let record = fixture.store.record(&child).unwrap();
    assert_eq!(record.parent, Some(root()));
    assert_eq!(record.description, "trace the estimate (fork)");
    assert_eq!(record.profile.as_deref(), Some("frontend"));
    assert_eq!(
        closed_phase(&fixture, &child),
        Some(ClosePhase::Completed { result: "Fixed the cache read.".into() })
    );
    // Its result reaches the root as any child's does.
    until(|| root_receipts(&fixture).len() == 2).await;
    let receipt = &root_receipts(&fixture)[1];
    assert_eq!(receipt.sender.clone().unwrap().id, child);
    assert_eq!(receipt.content, "Fixed the cache read.");
    // The source asked nothing more and stays archived in its round.
    assert_eq!(model.requests_of("task trace the estimate").len(), 1);
    let source_record = fixture.store.record(&source).unwrap();
    assert_eq!(source_record.round, 1);
    assert_eq!(
        closed_phase(&fixture, &source),
        Some(ClosePhase::Completed { result: "The card reads a stale cache.".into() })
    );
}

// A running child is forked at its newest completed text: what it streams
// after it stays its own, and it goes on and completes.
#[tokio::test(flavor = "local")]
async fn a_fork_of_a_running_child_ends_at_its_newest_completed_text_and_leaves_it_running() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("noted"), said("noted")]);
    model.child(
        "task trace the estimate",
        [
            streamed_then_held(
                &gate,
                vec![event::text("First finding."), event::thinking("Looking further.")],
                vec![event::text("Final finding."), event::response(1, 1)],
            ),
            said("Applied the first finding."),
        ],
    );
    let fixture = fixture(&model, &Model::default(), listed_model("other-model", &[], &[]));
    let mut client = fixture.opened().await;
    let source = spawn(&fixture, &root(), json!({ "prompt": "task trace the estimate" })).await;
    until(|| {
        session(&fixture, &source)
            .transcript()
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Thinking(_)))
    })
    .await;

    // Without --profile it inherits the root; --description names it.
    let forked = fork(
        &fixture,
        json!({ "id": 1, "description": "apply the finding", "message": "Apply what you found." }),
    )
    .await;
    assert_eq!(forked.stdout, "subagentId: 2\n", "{forked:?}");
    let child = named_node(&fixture.store, &forked);
    client.next_until(is_closed(&child)).await;

    let asked = &model.requests_of("task trace the estimate")[1];
    let text = request_text(asked);
    assert!(text.contains("First finding."), "{text}");
    assert!(text.ends_with("Apply what you found."), "{text}");
    assert!(!text.contains("Final finding."), "{text}");
    assert_eq!(fixture.store.record(&child).unwrap().description, "apply the finding");
    // The source went on running, and completes with what it found after.
    assert!(fixture.store.record(&source).unwrap().closed.is_none());
    gate.open();
    client.next_until(is_closed(&source)).await;
    until(|| root_receipts(&fixture).len() == 2).await;
    let source_receipt = root_receipts(&fixture)
        .into_iter()
        .find(|receipt| receipt.sender.as_ref().unwrap().id == source)
        .unwrap();
    assert_eq!(source_receipt.content, "Final finding.");
    assert!(model.is_done());
}

// A fork is refused, and creates nothing, before its source has written a
// completed text, for a node that is not the caller's child, and by every
// check of a spawn: the Subagent switch, the profile's name and the live
// limit.
#[tokio::test(flavor = "local")]
async fn a_fork_is_refused_without_a_completed_text_and_by_the_checks_of_a_spawn() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("noted")]);
    model.child(
        "task trace the estimate",
        [held(&gate, vec![event::text("Found it."), event::response(1, 1)])],
    );
    let fixture = fixture(&model, &Model::default(), listed_model("other-model", &[], &[]));
    let mut client = fixture.opened().await;
    let source = spawn(&fixture, &root(), json!({ "prompt": "task trace the estimate" })).await;
    until(|| model.requests_of("task trace the estimate").len() == 1).await;
    let message = |id: u64| json!({ "id": id, "message": "Go on." });

    let early = fork(&fixture, message(1)).await;
    assert_eq!(
        refused(&early),
        "demi agent fork: Agent 1 has no completed message to fork from yet\n"
    );
    let unknown = fork(&fixture, message(7)).await;
    assert_eq!(
        refused(&unknown),
        "demi agent fork: no subagent 7 of yours to fork (see `demi agent list`)\n"
    );
    let empty = fork(&fixture, json!({ "id": 1, "message": "  " })).await;
    assert_eq!(empty.code, 2, "{empty:?}");
    assert!(empty.stderr.starts_with("error: message must not be empty\nUsage: demi agent fork "), "{empty:?}");

    gate.open();
    client.next_until(is_closed(&source)).await;
    until(|| fixture.store.record(&source).unwrap().delivered).await;
    let named = fork(&fixture, json!({ "id": 1, "profile": "explorer", "message": "Go on." })).await;
    assert_eq!(
        refused(&named),
        "demi agent fork: unknown profile \"explorer\"; the user's profiles are research, frontend. Run `demi agent profiles list` for when to use each, or omit --profile to inherit the parent.\n"
    );
    fixture.product.subagents.borrow_mut().enabled = false;
    let off = fork(&fixture, message(1)).await;
    assert_eq!(
        refused(&off),
        "demi agent fork: subagents are turned off in settings; do this work in this session, or ask the user to turn subagents on in Settings > Subagents.\n"
    );
    {
        let mut settings = fixture.product.subagents.borrow_mut();
        settings.enabled = true;
        settings.max_live = std::num::NonZeroU32::new(1);
    }
    let live = spawn(&fixture, &root(), json!({ "prompt": "task hold" })).await;
    let full = fork(&fixture, message(1)).await;
    assert_eq!(
        refused(&full),
        "demi agent fork: 1 subagents are live across your conversations, 1 of them in this one, and Settings > Subagents allows 1; wait for one to finish, or do this work in this session.\n"
    );

    // Nothing was created: the next number went to the spawn in between.
    assert_eq!(fixture.number(&live), 2);
    assert_eq!(fixture.store.numbered(3), None);
}

// The source started a dev server and ended its turn: it stays live for its
// command. Its fork answers and closes, since the command is not its own.
// Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_fork_has_none_of_its_sources_running_commands() {
    let model = Model::default();
    model.child(
        "task serve",
        [
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "serve", "description": "Start the server", "intervalMs": null}),
            )]),
            said("Serving."),
            said("I started nothing."),
        ],
    );
    model.root([said("noted")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, Rc::new(ScriptedShells::default()), store.clone());
    let mut page = demi_agent_server::testing::TestClient::connect(&server, &conversation());
    page.send(crate::support::open()).await;
    let spawned = agent(&server, &root(), "spawn", json!({ "prompt": "task serve" })).await;
    let source = named_node(&store, &spawned);
    wait_for_execution(&server, "waiting").await;

    let forked = agent(&server, &root(), "fork", json!({ "id": 1, "message": "What runs?" })).await;
    let child = named_node(&store, &forked);
    page.next_until(is_closed(&child)).await;

    assert_eq!(
        store.record(&child).and_then(|record| record.closed).map(|close| close.phase),
        Some(ClosePhase::Completed { result: "I started nothing.".into() })
    );
    assert!(store.running_commands(&child).await.unwrap().is_empty());
    let saved = store.session_store(&child).load().await.unwrap().unwrap();
    assert!(saved.state.intervals.is_empty());
    assert!(saved.state.reports.is_empty());
    // The source keeps its command, and stays live for it.
    assert!(store.record(&source).unwrap().closed.is_none());
    let status = shell_command(&server, &root(), "status", json!({ "id": ["1"] })).await;
    assert!(status.stdout.contains("status: running"), "{status:?}");
}

// frontend's model has a window too small for research's history: the fork
// compacts it first with the source's model, then asks frontend's model
// with the summary in its place.
#[tokio::test(flavor = "local")]
async fn a_fork_onto_a_smaller_window_compacts_with_the_sources_model_first() {
    let long = "f".repeat(8_000);
    let model = Model::default();
    model.root([said("noted"), said("noted")]);
    model.child(
        "task trace the estimate",
        [said(&long), said("Summary: the card reads a stale cache.")],
    );
    // After the compaction, a request opens with the compaction note.
    let compacted = "The earlier part of this conversation was compacted.";
    let other = Model::default();
    other.child(compacted, [said("Fixed.")]);
    let small = ProviderModel {
        context_window: Some(1_000),
        ..listed_model("other-model", &[], &[])
    };
    let fixture = fixture(&model, &other, small);
    let mut client = fixture.opened().await;
    let source = spawn(&fixture, &root(), json!({ "prompt": "task trace the estimate" })).await;
    client.next_until(is_closed(&source)).await;
    until(|| fixture.store.record(&source).unwrap().delivered).await;

    let forked = fork(
        &fixture,
        json!({ "id": 1, "profile": "frontend", "message": "Fix it." }),
    )
    .await;
    let child = named_node(&fixture.store, &forked);
    client.next_until(is_closed(&child)).await;

    // The summary request went to the source's model, from the fork.
    let on_stub = model.requests_of("task trace the estimate");
    assert_eq!(on_stub.len(), 2);
    assert!(is_summary_request(&on_stub[1]));
    assert_eq!(on_stub[1].model_id, "test-model");
    assert_eq!(on_stub[1].session_id, child.as_str());
    // frontend's model read the summary, not the history it replaced.
    let asked = other.requests_of(compacted);
    assert_eq!(asked.len(), 1);
    let text = request_text(&asked[0]);
    assert!(text.contains("Summary: the card reads a stale cache."), "{text}");
    assert!(!text.contains(&long), "{text}");
    // The message comes last, then where the fork works.
    assert!(
        text.ends_with("Fix it.\nYour working directory is /workspace: your shell calls start there on the primary Host, and the relative paths you write are read against it. `demi agent cwd <dir>` changes it."),
        "{text}"
    );
    // The source's own history is as it was.
    let kept = fixture.store.session_store(&source).load().await.unwrap().unwrap();
    assert!(!kept.transcript.iter().any(|block| matches!(block, Block::CompactionBoundary(_))));
}

/// Waits until the saved checkpoint of `node` holds the loss of a resident
/// command for its next turn (`crates/agent-session/src/reports.rs`).
async fn until_loss_waits(store: &MemoryTreeStore, node: &NodeId) {
    for _ in 0..1_000 {
        let saved = store.session_store(node).load().await.unwrap().unwrap();
        if !saved.state.next_turn_reports.is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the loss never waited in the checkpoint of {node}");
}

// A dev server the root started was lost while the conversation was idle:
// its report waits for the next turn, and a Fork from the root's answer
// still succeeds, its new root holding no report of the source's command.
// Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_conversation_fork_after_a_resident_commands_loss_succeeds_and_holds_no_report() {
    let script = demi_provider_common::testing::ScriptedRuntime::new([
        Turn::Events(vec![event::tool_call(
            "call-1",
            "shell",
            json!({"script": "serve", "description": "Start the server", "intervalMs": null}),
        )]),
        said("Serving."),
    ]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide("stub", &script);
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = demi_agent_server::testing::TestClient::connect(&server, &conversation());
    page.send(crate::support::open()).await;
    page.next_until(crate::support::is_pending_steers).await;
    page.send(crate::support::send("m1", "Serve.")).await;
    page.next_until(crate::support::is_idle).await;
    shells.last().lose();
    until_loss_waits(&store, &conversation()).await;

    let blocks = server
        .node(&conversation(), &conversation())
        .unwrap()
        .session()
        .transcript()
        .blocks;
    let answer = blocks
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .unwrap()
        .id()
        .clone();
    let seed = server.prepare_fork(&conversation(), &answer).await.unwrap();
    let destination = NodeId::try_from("fork").unwrap();
    server.initialize_fork(&destination, seed).await.unwrap();
    let saved = store.session_store(&destination).load().await.unwrap().unwrap();
    assert!(saved.state.next_turn_reports.is_empty() && saved.state.reports.is_empty());
}

// The child's dev server was lost after it answered: its report waits for
// the child's next turn, and the child's fork holds no report of it
// (`crates/agent-server/src/server/tree/supervisor.rs`). Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_subagent_fork_after_a_resident_commands_loss_holds_no_report() {
    let model = Model::default();
    model.child(
        "task serve",
        [
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "serve", "description": "Start the server", "intervalMs": null}),
            )]),
            said("Serving."),
            said("I started nothing."),
        ],
    );
    model.root([said("noted"), said("noted")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = demi_agent_server::testing::TestClient::connect(&server, &conversation());
    page.send(crate::support::open()).await;
    let spawned = agent(&server, &root(), "spawn", json!({ "prompt": "task serve" })).await;
    let source = named_node(&store, &spawned);
    wait_for_execution(&server, "waiting").await;
    shells.last().lose();
    until_loss_waits(&store, &source).await;

    let forked = agent(&server, &root(), "fork", json!({ "id": 1, "message": "What runs?" })).await;
    let child = named_node(&store, &forked);
    page.next_until(is_closed(&child)).await;
    let saved = store.session_store(&child).load().await.unwrap().unwrap();
    assert!(saved.state.next_turn_reports.is_empty() && saved.state.reports.is_empty());
    // The fork's request, the last of the source's script, tells of no loss.
    let asked = model.requests_of("task serve");
    let last = request_text(asked.last().unwrap());
    assert!(last.contains("What runs?") && !last.contains("was lost"), "{last}");
}
