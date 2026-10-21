//! `demi agent resume --profile` at the tree's boundary
//! (`crates/agent-server/src/server/tree/supervisor.rs`).
//!
//! A resume may move the archived child to another profile: it keeps its
//! number, description and history, and its next round runs on the new
//! profile's model, whichever provider that is.

use std::rc::Rc;

use demi_agent_server::ServerConfig;
use demi_agent_store::{
    AgentTreeStore, ClosePhase, NodeClose, NodeRecord,
    testing::{MemoryTreeStore, model_of, text},
};
use demi_agent_tools::{Profile, ProfileModel};
use demi_conversation_socket_protocol::ServerFrame;
use demi_shared_types::{Block, ModelSelection, NodeId, QueuedMessage, Timestamp, TurnId};
use serde_json::{Value, json};

use crate::subagents::{answer, checkpoint, checkpoint_on, child_in, child_record, closed_phase, held_said, is_closed, refused, root, said, spawn, spawn_during_turn, user};
use crate::support::{Fixture, Gate, Model, TestProduct, agent, listed_model, request_text, until};

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

/// A profile on the provider `other`'s `other-model`, with instructions of
/// its own.
fn frontend() -> Profile {
    Profile {
        context_limit: Some(40_000),
        name: "frontend".into(),
        description: "Writes the web app.".into(),
        model: Some(ProfileModel {
            provider_name: None,
            provider_id: "other".into(),
            model_id: "other-model".into(),
            thinking_effort: None,
            service_tier_id: None,
        }),
        instructions: Some("Write for the web app.".into()),
        ..research()
    }
}

fn fixture(model: &Model, other: &Model) -> Fixture {
    fixture_on(model, other, MemoryTreeStore::new())
}

/// As [`fixture`], over a store that holds children already.
fn fixture_on(model: &Model, other: &Model, store: Rc<MemoryTreeStore>) -> Fixture {
    let product = TestProduct::default();
    product.subagents.borrow_mut().profiles = vec![research(), frontend()];
    let fixture = Fixture::with_model(model, product, store, ServerConfig::default());
    fixture.resolver.provide_runtime("other", other.clone());
    fixture.resolver.list("other", vec![listed_model("other-model", &[], &[])]);
    fixture
}

/// The model ids the text blocks of `node`'s saved transcript were written
/// by, in transcript order.
async fn text_models(fixture: &Fixture, node: &NodeId) -> Vec<(String, String)> {
    let saved = fixture.store.session_store(node).load().await.unwrap().unwrap();
    saved
        .transcript
        .iter()
        .filter(|block| matches!(block, Block::Text(_)))
        .map(|block| {
            let block = serde_json::to_value(block).unwrap();
            (
                block["text"].as_str().unwrap_or_default().to_owned(),
                block["model"]["model"]["id"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

async fn shown(fixture: &Fixture, number: u64) -> String {
    let shown = agent(&fixture.server, &root(), "show", json!({ "id": [number] })).await;
    assert_eq!(shown.code, 0, "{shown:?}");
    shown.stdout
}

async fn listed(fixture: &Fixture) -> Value {
    let list = agent_json(fixture, "list", json!({ "all": true })).await;
    list
}

async fn agent_json(fixture: &Fixture, command: &str, args: Value) -> Value {
    let run = crate::support::agent_call(
        &fixture.server,
        &root(),
        command,
        args,
        true,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("the call is dispatched");
    assert_eq!(run.code, 0, "{run:?}");
    serde_json::from_str(&run.stdout).expect("the output is JSON")
}

// A finished child on the parent's model resumes onto `frontend`, on another
// provider, and back onto `research`: each round runs on its profile's model
// over the whole history, the number and description stay, the record and
// both reads name the profile it runs on now, and the earlier rounds'
// blocks keep the model that wrote them.
#[tokio::test(flavor = "local")]
async fn a_resume_onto_another_profile_goes_on_with_its_history_on_that_models_provider_and_back() {
    let model = Model::default();
    model.root([said("noted"), said("noted"), said("noted")]);
    model.child(
        "task trace the estimate",
        [said("The card reads a stale cache."), said("Both are done.")],
    );
    let other = Model::default();
    other.child("task trace the estimate", [said("Fixed the cache read.")]);
    let fixture = fixture(&model, &other);
    let mut client = fixture.opened().await;
    let child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task trace the estimate", "profile": "research", "description": "trace the estimate" }),
    )
    .await;
    client.next_until(is_closed(&child)).await;
    until(|| fixture.store.record(&child).unwrap().delivered).await;

    // Round 2 on the other provider.
    let resumed = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": 1, "profile": "frontend", "message": "Fix it as you described." }),
    )
    .await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    let frames = client.next_until(round_closed(&child, 2)).await;
    // Its row names the profile and the model the round ran on.
    let row = child_in(frames.last().unwrap(), &child).unwrap();
    assert_eq!(
        (row.number, row.profile.as_deref(), row.model.provider_id.as_str(), row.round),
        (1, Some("frontend"), "other", 2)
    );
    let asked = other.requests_of("task trace the estimate");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].model_id, "other-model");
    let text = request_text(&asked[0]);
    let history = text.find("The card reads a stale cache.").expect("the history is replayed");
    let message = text.find("Fix it as you described.").expect("the message is sent");
    assert!(history < message, "{text}");
    assert!(asked[0].system_prompt.ends_with("Instructions for this subagent:\n\nWrite for the web app."), "{}", asked[0].system_prompt);
    assert_eq!(model.requests_of("task trace the estimate").len(), 1);

    let record = fixture.store.record(&child).unwrap();
    assert_eq!(
        (record.number, record.description.as_str(), record.profile.as_deref(), record.context_limit),
        (1, "trace the estimate", Some("frontend"), Some(40_000))
    );
    assert_eq!(record.model.as_ref().map(|model| model.model.id.as_str()), Some("other-model"));
    assert_eq!(
        closed_phase(&fixture, &child),
        Some(ClosePhase::Completed)
    );
    assert_eq!(
        text_models(&fixture, &child).await,
        [
            ("The card reads a stale cache.".to_owned(), "test-model".to_owned()),
            ("Fixed the cache read.".to_owned(), "other-model".to_owned()),
        ]
    );
    // The reads name the profile it runs on now.
    let rows = listed(&fixture).await;
    assert_eq!(rows[1]["id"], 1);
    assert_eq!(rows[1]["profile"], "frontend");
    assert!(shown(&fixture, 1).await.contains("Profile: frontend\n"));

    // Round 3 back onto the first provider.
    until(|| fixture.store.record(&child).unwrap().delivered).await;
    let back = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": 1, "profile": "research", "message": "Now confirm both." }),
    )
    .await;
    assert_eq!(back.code, 0, "{back:?}");
    let frames = client.next_until(round_closed(&child, 3)).await;
    let row = child_in(frames.last().unwrap(), &child).unwrap();
    assert_eq!((row.profile.as_deref(), row.model.provider_id.as_str()), (Some("research"), "stub"));
    let asked = model.requests_of("task trace the estimate");
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1].model_id, "test-model");
    let text = request_text(&asked[1]);
    assert!(text.contains("Fixed the cache read."), "{text}");
    assert!(!asked[1].system_prompt.contains("Write for the web app."), "{}", asked[1].system_prompt);
    let record = fixture.store.record(&child).unwrap();
    assert_eq!((record.profile.as_deref(), record.instructions, record.context_limit), (Some("research"), None, None));
    assert_eq!(record.model.as_ref().map(|model| model.model.id.as_str()), Some("test-model"));
    assert_eq!(
        text_models(&fixture, &child).await,
        [
            ("The card reads a stale cache.".to_owned(), "test-model".to_owned()),
            ("Fixed the cache read.".to_owned(), "other-model".to_owned()),
            ("Both are done.".to_owned(), "test-model".to_owned()),
        ]
    );
    assert!(shown(&fixture, 1).await.contains("Profile: research\n"));
}

/// Whether a frame tells the child's round `round` closed.
fn round_closed(child: &NodeId, round: u64) -> impl Fn(&ServerFrame) -> bool + '_ {
    move |frame| is_closed(child)(frame) && child_in(frame, child).is_some_and(|row| row.round == round)
}

fn is_done(fixture: &Fixture, node: &NodeId) -> bool {
    fixture.store.record(node).unwrap().closed.is_some()
}

// Without `--profile` a resume runs as it did; a profile that cannot be had
// refuses the resume and leaves the child archived in its round, with the
// profile it ran on.
#[tokio::test(flavor = "local")]
async fn a_resume_without_a_profile_is_as_before_and_a_refused_profile_changes_nothing() {
    let model = Model::default();
    model.root([said("noted"), said("noted")]);
    model.child(
        "task trace the estimate",
        [said("The card reads a stale cache."), said("Still stale.")],
    );
    let fixture = fixture(&model, &Model::default());
    let mut client = fixture.opened().await;
    let child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task trace the estimate", "profile": "research" }),
    )
    .await;
    client.next_until(is_closed(&child)).await;
    until(|| fixture.store.record(&child).unwrap().delivered).await;

    let unknown = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": 1, "profile": "explorer", "message": "again" }),
    )
    .await;
    assert_eq!(
        refused(&unknown),
        "demi agent resume: unknown profile \"explorer\"; the user's profiles are research, frontend. Run `demi agent profiles list` for when to use each, or omit --profile to go on with the profile the child runs on.\n"
    );
    let kept = fixture.store.record(&child).unwrap();
    assert_eq!((kept.round, kept.profile.as_deref(), kept.closed.is_some()), (1, Some("research"), true));

    let resumed = agent(&fixture.server, &root(), "resume", json!({ "id": 1, "message": "again" })).await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    until(|| fixture.store.record(&child).unwrap().round == 2 && is_done(&fixture, &child)).await;
    assert_eq!(model.requests_of("task trace the estimate").len(), 2);
    assert_eq!(fixture.store.record(&child).unwrap().profile.as_deref(), Some("research"));
}

// What a child runs on is its record's. A child moved to `frontend` whose
// round ended before its first request landed the switch keeps the model its
// history was last requested with in its checkpoint; a resume without a
// profile after it runs on frontend's model and instructions.
#[tokio::test(flavor = "local")]
async fn a_resume_after_a_round_that_never_landed_its_switch_runs_on_the_records_model() {
    let store = MemoryTreeStore::new();
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let moved = NodeRecord {
        profile: Some("frontend".into()),
        instructions: Some("Write for the web app.".into()),
        model: Some(model_of("other", "other-model")),
        closed: Some(NodeClose {
            phase: ClosePhase::Aborted,
            at: Timestamp::UNIX_EPOCH,
        }),
        delivered: true,
        round: 2,
        ..child_record("moved", 1, "conversation", None)
    };
    // Its checkpoint is what the first round saved, on the stub's model.
    store
        .create_node(
            moved,
            checkpoint(Vec::new(), vec![user("m1", "task trace the estimate"), answer("m2", "The card reads a stale cache.")]),
        )
        .await
        .unwrap();
    let model = Model::default();
    model.root([said("noted")]);
    let other = Model::default();
    other.child("task trace the estimate", [said("Fixed.")]);
    let fixture = fixture_on(&model, &other, store);
    let mut client = fixture.opened().await;

    let resumed = agent(&fixture.server, &root(), "resume", json!({ "id": 1, "message": "Fix it now." })).await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    let child = fixture.store.numbered(1).unwrap();
    client.next_until(round_closed(&child, 3)).await;
    let asked = other.requests_of("task trace the estimate");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].model_id, "other-model");
    assert!(
        asked[0].system_prompt.ends_with("Instructions for this subagent:\n\nWrite for the web app."),
        "{}",
        asked[0].system_prompt
    );
    assert!(request_text(&asked[0]).contains("The card reads a stale cache."));
    assert!(model.requests_of("task trace the estimate").is_empty());
    // Its next checkpoint keeps what it ran on.
    let saved = fixture.store.session_store(&child).load().await.unwrap().unwrap();
    assert_eq!(saved.state.model.model.id, "other-model");
}

// A live child restored from the store runs its round on its record's model
// too, whatever its checkpoint saved.
#[tokio::test(flavor = "local")]
async fn a_restored_child_runs_on_the_records_model_not_its_checkpoints() {
    let store = MemoryTreeStore::new();
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let live = NodeRecord {
        profile: Some("frontend".into()),
        instructions: Some("Write for the web app.".into()),
        model: Some(model_of("other", "other-model")),
        ..child_record("live", 1, "conversation", None)
    };
    let brief = QueuedMessage {
        comments: Vec::new(),
        id: TurnId::try_from("brief").unwrap(),
        content: text("task trace the estimate"),
        automation: None,
    };
    store.create_node(live, checkpoint(vec![brief], Vec::new())).await.unwrap();
    let model = Model::default();
    model.root([said("noted")]);
    let other = Model::default();
    other.child("task trace the estimate", [said("Fixed.")]);
    let fixture = fixture_on(&model, &other, store);
    let mut client = fixture.opened().await;

    let child = fixture.store.numbered(1).unwrap();
    client.next_until(is_closed(&child)).await;
    let asked = other.requests_of("task trace the estimate");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].model_id, "other-model");
    assert!(model.requests_of("task trace the estimate").is_empty());
}

/// Child 1 of the conversation, whose record names `record_model` with the
/// profile `frontend`'s instructions, and whose checkpoint was last requested
/// with `checkpoint_model`, a model of a provider the fixture serves or not.
/// It is archived and delivered, or live with the brief queued.
async fn child_on(
    store: &MemoryTreeStore,
    record_model: ModelSelection,
    checkpoint_model: ModelSelection,
    history: &str,
    archived: bool,
) {
    store
        .create_node(NodeRecord::root(root(), Timestamp::UNIX_EPOCH), checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    let child = NodeRecord {
        profile: Some("frontend".into()),
        instructions: Some("Write for the web app.".into()),
        model: Some(record_model),
        closed: archived.then(|| NodeClose {
            phase: ClosePhase::Completed,
            at: Timestamp::UNIX_EPOCH,
        }),
        delivered: archived,
        ..child_record("moved", 1, "conversation", None)
    };
    let (queue, blocks) = if archived {
        (Vec::new(), vec![user("m1", "task trace the estimate"), answer("m2", history)])
    } else {
        let brief = QueuedMessage {
            comments: Vec::new(),
            id: TurnId::try_from("brief").unwrap(),
            content: text("task trace the estimate"),
            automation: None,
        };
        (vec![brief], Vec::new())
    };
    store.create_node(child, checkpoint_on(checkpoint_model, queue, blocks)).await.unwrap();
}

// What a child runs on is its record's alone: the provider its history was
// last requested with is not a condition of going on. With that provider
// gone, a resume with a profile and one without run on the record's model.
#[tokio::test(flavor = "local")]
async fn a_resume_goes_on_on_the_records_model_when_the_old_provider_is_gone() {
    let store = MemoryTreeStore::new();
    child_on(&store, model_of("gone", "gone-model"), model_of("gone", "gone-model"), "The card reads a stale cache.", true).await;
    let model = Model::default();
    model.root([said("noted")]);
    let other = Model::default();
    other.child("task trace the estimate", [said("Fixed."), said("Once more.")]);
    let fixture = fixture_on(&model, &other, store);
    let mut client = fixture.opened().await;
    let child = fixture.store.numbered(1).unwrap();

    // The record names a model whose provider is gone, so a profile on a
    // provider that is served moves it.
    let resumed = agent(&fixture.server, &root(), "resume", json!({ "id": 1, "profile": "frontend", "message": "Fix it." })).await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    client.next_until(round_closed(&child, 2)).await;
    assert_eq!(other.requests_of("task trace the estimate")[0].model_id, "other-model");
    assert_eq!(closed_phase(&fixture, &child), Some(ClosePhase::Completed));
    until(|| fixture.store.record(&child).unwrap().delivered).await;
    // The round saved the model it ran on; a resume after it names no profile.
    let again = agent(&fixture.server, &root(), "resume", json!({ "id": 1, "message": "Once more." })).await;
    assert_eq!(again.code, 0, "{again:?}");
    client.next_until(round_closed(&child, 3)).await;
    assert_eq!(other.requests_of("task trace the estimate").len(), 2);
}

// A child moved to a model whose round never landed its switch, with the old
// provider gone since: a resume without a profile and a restore go on on the
// record's model, as the history needs no compaction.
#[tokio::test(flavor = "local")]
async fn a_resume_and_a_restore_go_on_on_the_records_model_when_the_old_provider_is_gone() {
    for archived in [true, false] {
        let store = MemoryTreeStore::new();
        child_on(&store, model_of("other", "other-model"), model_of("gone", "gone-model"), "The card reads a stale cache.", archived).await;
        let model = Model::default();
        model.root([said("noted")]);
        let other = Model::default();
        other.child("task trace the estimate", [said("Fixed.")]);
        let fixture = fixture_on(&model, &other, store);
        let mut client = fixture.opened().await;
        let child = fixture.store.numbered(1).unwrap();
        if archived {
            let resumed = agent(&fixture.server, &root(), "resume", json!({ "id": 1, "message": "Fix it." })).await;
            assert_eq!(resumed.code, 0, "{resumed:?}");
        }
        client.next_until(is_closed(&child)).await;
        let asked = other.requests_of("task trace the estimate");
        assert_eq!(asked.len(), 1, "archived: {archived}");
        assert_eq!(asked[0].model_id, "other-model");
        assert_eq!(closed_phase(&fixture, &child), Some(ClosePhase::Completed));
    }
}

// A history over the new model's window needs the old model to compact it
// first. With the old provider gone, the round fails and says why.
#[tokio::test(flavor = "local")]
async fn a_history_that_needs_the_old_model_to_compact_fails_the_round_and_says_why() {
    let store = MemoryTreeStore::new();
    let long = "f".repeat(8_000);
    // The record's model has a window of 1,000 tokens.
    let mut small = model_of("other", "other-model");
    small.model.context_window = 1_000;
    child_on(&store, small, model_of("gone", "gone-model"), &long, true).await;
    let model = Model::default();
    model.root([said("noted"), said("noted")]);
    let other = Model::default();
    let fixture = fixture_on(&model, &other, store);
    let mut client = fixture.opened().await;
    let child = fixture.store.numbered(1).unwrap();

    let resumed = agent(&fixture.server, &root(), "resume", json!({ "id": 1, "message": "Fix it." })).await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    client.next_until(is_closed(&child)).await;
    let Some(ClosePhase::Error { failure }) = closed_phase(&fixture, &child) else {
        panic!("the round fails: {:?}", closed_phase(&fixture, &child));
    };
    assert!(failure.contains("gone-model"), "{failure}");
    assert!(failure.contains("Provider \"gone\" is not available"), "{failure}");
    assert!(other.requests_of("task trace the estimate").is_empty());
}

// The Subagents switch guards the making of children, not the going on of
// one: with it off, a resume that names a profile goes on as one that names
// none does, and a spawn is refused.
#[tokio::test(flavor = "local")]
async fn a_resume_that_names_a_profile_goes_on_while_subagents_are_off() {
    let store = MemoryTreeStore::new();
    child_on(&store, model_of("stub", "test-model"), model_of("stub", "test-model"), "The card reads a stale cache.", true).await;
    let model = Model::default();
    model.root([said("noted")]);
    let other = Model::default();
    other.child("task trace the estimate", [said("Fixed.")]);
    let fixture = fixture_on(&model, &other, store);
    let mut client = fixture.opened().await;
    fixture.product.subagents.borrow_mut().enabled = false;
    let child = fixture.store.numbered(1).unwrap();

    let refused_spawn = agent(&fixture.server, &root(), "spawn", json!({ "prompt": "task new" })).await;
    assert!(refused(&refused_spawn).contains("subagents are turned off in settings"), "{refused_spawn:?}");
    let resumed = agent(&fixture.server, &root(), "resume", json!({ "id": 1, "profile": "frontend", "message": "Fix it." })).await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    client.next_until(round_closed(&child, 2)).await;
    assert_eq!(other.requests_of("task trace the estimate")[0].model_id, "other-model");
}

// A child's instructions are its record's: one that inherited its parent's
// has the parent's own at the spawn, and a later change of its parent's
// profile reaches it in no round.
#[tokio::test(flavor = "local")]
async fn an_inheriting_child_keeps_the_instructions_its_parent_had_at_its_spawn() {
    let model = Model::default();
    model.root([said("noted")]);
    let other = Model::default();
    let gate = Gate::new();
    other.child("task parent", [held_said(&gate, "delegated")]);
    other.child("task fresh", [said("done")]);
    other.child("task grandchild", [said("done again")]);
    let fixture = fixture(&model, &other);
    let mut client = fixture.opened().await;
    let parent = spawn(&fixture, &root(), json!({ "prompt": "task parent", "profile": "frontend" })).await;

    // A grandchild stored with what a read-only parent had when it was
    // spawned, though its parent has moved to `frontend` since.
    fixture
        .store
        .create_node(
            NodeRecord {
                instructions: Some("Read only.".into()),
                model: Some(model_of("other", "other-model")),
                closed: Some(NodeClose {
                    phase: ClosePhase::Completed,
                    at: Timestamp::UNIX_EPOCH,
                }),
                delivered: true,
                ..child_record("grandchild", 9, parent.as_str(), None)
            },
            checkpoint_on(model_of("other", "other-model"), Vec::new(), vec![user("g1", "task grandchild"), answer("g2", "first")]),
        )
        .await
        .unwrap();
    let grandchild = fixture.store.numbered(9).unwrap();
    // Both starts wait for the parent's turn, which keeps it open.
    let fresh = spawn_during_turn(&fixture, &parent, json!({ "prompt": "task fresh" }));
    let resumed = {
        let server = fixture.server.clone();
        let parent = parent.clone();
        tokio::task::spawn_local(async move {
            agent(&server, &parent, "resume", json!({ "id": 9, "message": "Again." })).await
        })
    };
    gate.open();
    let fresh = fresh.await.unwrap();
    let resumed = resumed.await.unwrap();
    assert_eq!(resumed.code, 0, "{resumed:?}");
    client.next_until(|frame| is_closed(&fresh)(frame) || is_closed(&grandchild)(frame)).await;
    until(|| closed_phase(&fixture, &fresh).is_some() && closed_phase(&fixture, &grandchild).is_some() && other.requests_of("task grandchild").len() == 1).await;

    // A child that inherits records what its parent's prompt added, and
    // runs on it.
    assert_eq!(fixture.store.record(&fresh).unwrap().instructions.as_deref(), Some("Write for the web app."));
    let asked = other.requests_of("task fresh");
    assert!(asked[0].system_prompt.ends_with("Instructions for this subagent:\n\nWrite for the web app."), "{}", asked[0].system_prompt);
    // The grandchild runs on its own record's, not its parent's now.
    let asked = other.requests_of("task grandchild");
    assert!(asked[0].system_prompt.ends_with("Instructions for this subagent:\n\nRead only."), "{}", asked[0].system_prompt);
}
