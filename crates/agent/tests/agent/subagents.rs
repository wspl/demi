//! Subagents at the tree's boundary (`subagents.md` § Acceptance): the
//! `demi agent` calls a node's jobs make, the frames the root's connection
//! receives, what each model is asked, and the tree store's records.

use std::rc::Rc;

use demi_agent::{
    AgentTreeStore, Profile, ServerConfig,
    store::{
        CheckpointState, CheckpointUpdate, ClosePhase, CommandStateSnapshot, NodeClose, NodeRecord,
    },
    testing::{MemoryTreeStore, test_model, text},
};
use demi_agent_protocol::{ClientFrame, JobPhase, ServerFrame, SubagentEvent, TranscriptPatch};
use demi_core::{
    AgentMessage, AgentMessageEvent, Block, BlockId, CompletionId, CompletionOutcome, NodeId,
    QueuedMessage, SessionPhase, TextBlock, Timestamp, TurnId, UserBlock, UserContentBlock,
};
use demi_provider::{
    InferenceItem,
    testing::{Turn, event},
};
use demi_shell::{RpcError, StorageOp, StorageReply};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::support::{
    CommandRun, Fixture, Gate, Model, TestHarness, agent, agent_call, conversation, frames_until,
    held, is_idle, is_pending_steers, open, request_text, send, texts, until,
};

fn said(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(1, 1)])
}

fn held_said(gate: &Gate, text: &str) -> Turn {
    held(gate, vec![event::text(text), event::response(1, 1)])
}

fn root() -> NodeId {
    conversation()
}

fn fixture(model: &Model, harness: TestHarness) -> Fixture {
    Fixture::with_model(
        model,
        harness,
        MemoryTreeStore::new(),
        ServerConfig::default(),
    )
}

/// The child a `spawn` or `resume` named on stdout.
fn child_id(run: &CommandRun) -> NodeId {
    let id = run
        .stdout
        .strip_prefix("subagentId: ")
        .and_then(|rest| rest.strip_suffix('\n'))
        .unwrap_or_else(|| panic!("{run:?} names no child"));
    NodeId::try_from(id).unwrap()
}

async fn spawn(fixture: &Fixture, caller: &NodeId, args: Value) -> NodeId {
    let run = agent(&fixture.server, caller, "spawn", args).await;
    assert_eq!(run.code, 0, "{run:?}");
    child_id(&run)
}

/// The failure a call wrote.
fn refused(run: &CommandRun) -> &str {
    assert_eq!(run.code, 1, "{run:?}");
    &run.stderr
}

/// Each `subagent` frame as its event, child and phase.
fn lifecycle(frames: &[ServerFrame]) -> Vec<(SubagentEvent, NodeId, JobPhase)> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::Subagent { event, job } => {
                Some((*event, job.subagent_id.clone(), job.phase))
            }
            _ => None,
        })
        .collect()
}

/// The agent messages written into the root's transcript.
fn root_receipts(fixture: &Fixture) -> Vec<AgentMessage> {
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

fn is_closed(child: &NodeId) -> impl Fn(&ServerFrame) -> bool + '_ {
    move |frame| matches!(frame, ServerFrame::Subagent { event: SubagentEvent::Closed, job } if &job.subagent_id == child)
}

fn closed_phase(fixture: &Fixture, child: &NodeId) -> Option<ClosePhase> {
    fixture
        .store
        .record(child)
        .and_then(|record| record.closed)
        .map(|close| close.phase)
}

#[tokio::test(flavor = "local")]
async fn an_inherited_child_starts_from_its_brief_and_its_completion_wakes_the_idle_parent() {
    let model = Model::default();
    model.root([said("working"), said("got it")]);
    model.child("Read notes.md", [said("the file says 42")]);
    let fixture = fixture(&model, TestHarness::default());
    let mut client = fixture.opened().await;
    client.send(send("m1", "start")).await;
    client.next_until(is_idle).await;

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
    let child = child_id(&spawned);
    let frames = frames_until(&mut client, is_idle).await;

    assert_eq!(
        refused(&empty),
        "demi agent spawn: prompt must not be empty\n"
    );
    // Creation, not the child's work, is what the call reports.
    assert_eq!(spawned.stdout, format!("subagentId: {child}\n"));
    let started = frames
        .iter()
        .find_map(|frame| match frame {
            ServerFrame::Subagent {
                event: SubagentEvent::Started,
                job,
            } => Some(job.clone()),
            _ => None,
        })
        .expect("the child started");
    assert_eq!(
        (
            &started.subagent_id,
            &started.parent_session_id,
            started.description.as_str(),
            started.profile.as_deref(),
            started.phase,
            started.ended_at,
            started.result.as_deref()
        ),
        (
            &child,
            &root(),
            "reader",
            None,
            JobPhase::Running,
            None,
            None
        )
    );
    assert!(frames.iter().any(|frame| matches!(frame,
        ServerFrame::SubagentTranscriptReset { subagent_id, blocks, .. } if subagent_id == &child && blocks.is_empty())));
    let closed = frames
        .iter()
        .position(is_closed(&child))
        .expect("the child closed");
    let ServerFrame::Subagent { job, .. } = &frames[closed] else {
        unreachable!()
    };
    assert_eq!(
        (job.phase, job.result.as_deref()),
        (JobPhase::Completed, Some("the file says 42"))
    );
    assert!(job.ended_at.is_some());
    let receipt = frames
        .iter()
        .position(is_receipt_patch)
        .expect("the parent wrote the receipt");
    assert!(closed < receipt, "{frames:#?}");

    // The child was asked with its preamble and its brief alone.
    let asked = model.requests_of("Read notes.md");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].session_id, child.as_str());
    assert_eq!(asked[0].system_prompt, "system prompt");
    let [InferenceItem::UserMessage { content }] = &*asked[0].items else {
        panic!("{:?}", asked[0].items)
    };
    let [
        UserContentBlock::Text { text: preamble },
        UserContentBlock::Text { text: first },
    ] = content.as_slice()
    else {
        panic!("{content:?}")
    };
    assert!(preamble.starts_with(&format!(
        "You are a subagent: a child agent session (id {child}) spawned by parent agent session {}.",
        root()
    )));
    assert!(preamble.contains("`demi agent spawn` spawns your own children."));
    assert_eq!(first, brief);
    assert!(
        fixture
            .harness
            .prompts
            .borrow()
            .iter()
            .all(|help| help.contains("demi agent spawn") && help.contains("demi agent send"))
    );

    // The parent heard the result once, as agent input of the request that
    // answered it.
    let answered = model.root_requests().pop().unwrap();
    let Some(InferenceItem::UserSteer { content }) = answered.items.last() else {
        panic!("{:?}", answered.items)
    };
    let envelope = texts(content);
    assert!(
        envelope.contains("\"content\":\"the file says 42\""),
        "{envelope}"
    );
    assert!(envelope.contains("\"outcome\":\"completed\""), "{envelope}");
    let record = fixture.store.record(&child).unwrap();
    assert_eq!(record.parent, Some(root()));
    assert_eq!(
        closed_phase(&fixture, &child),
        Some(ClosePhase::Completed {
            result: "the file says 42".into()
        })
    );
    assert!(
        record.delivered,
        "the parent's save marked the round delivered"
    );
    assert_eq!(
        fixture.store.checkpoint(&child).unwrap().state.cwd,
        "/workspace"
    );
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
    assert_eq!(receipts[0].sender.description, "reader");
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
    let fixture = fixture(&model, TestHarness::default());
    let mut client = fixture.opened().await;

    let delta = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task delta", "description": "delta" }),
    )
    .await;
    frames_until(&mut client, is_idle).await;
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
    let gamma = child_id(&gamma_start.await.unwrap());
    let alpha_node = fixture.server.node(&root(), &alpha).unwrap();
    until(|| alpha_node.session().is_settled()).await;

    let across = agent(
        &fixture.server,
        &gamma,
        "send",
        json!({ "id": beta.as_str(), "message": "hello from gamma" }),
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
        json!({ "id": gamma.as_str(), "message": "me" }),
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
        json!({ "id": delta.as_str(), "message": "late" }),
    )
    .await;
    let abort_sibling = agent(
        &fixture.server,
        &alpha,
        "abort",
        json!({ "id": beta.as_str() }),
    )
    .await;
    let resume_foreign = agent(
        &fixture.server,
        &alpha,
        "resume",
        json!({ "id": delta.as_str(), "message": "again" }),
    )
    .await;
    let listed = agent(&fixture.server, &gamma, "list", json!({})).await;
    let listed_json = agent_call(
        &fixture.server,
        &gamma,
        "list",
        json!({}),
        true,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let shown = agent(
        &fixture.server,
        &alpha,
        "show",
        json!({ "id": beta.as_str() }),
    )
    .await;
    let show_root = agent(
        &fixture.server,
        &alpha,
        "show",
        json!({ "id": root().as_str() }),
    )
    .await;

    assert_eq!(across.stdout, format!("sent to {beta}\n"));
    assert_eq!(to_parent.stdout, format!("sent to {alpha}\n"));
    let alpha_heard = request_text(&model.requests_of("task alpha")[1]);
    assert!(alpha_heard.contains("gamma status"), "{alpha_heard}");
    assert!(
        alpha_heard.contains(&format!("\"id\":\"{gamma}\"")),
        "{alpha_heard}"
    );
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
        format!(
            "demi agent send: no live agent \"{delta}\" (see `demi agent list`; an archived child is revived only by its parent via resume)\n"
        )
    );
    assert_eq!(
        refused(&abort_sibling),
        format!("demi agent abort: \"{beta}\" is not one of your running children\n")
    );
    assert_eq!(
        refused(&resume_foreign),
        "demi agent resume: request-id references an agent owned by another session\n"
    );
    let live = |id: &NodeId, name: &str, execution: &str, activity: &str| {
        format!(
            "{id}  running  up 0s  last-event 0s ago  profile=(inherit)  \"{name}\"  execution={execution}  activity={activity}"
        )
    };
    assert_eq!(
        listed.stdout,
        [
            format!("● {}  (root session)", root()),
            format!("├─● {}", live(&alpha, "alpha", "idle", "idle")),
            format!(
                "│ └─● {} ← you",
                live(&gamma, "gamma", "provider_streaming", "streaming")
            ),
            format!(
                "├─● {}",
                live(&beta, "beta", "provider_streaming", "streaming")
            ),
            format!("└─○ {delta}  archived (completed 0s ago)  \"delta\""),
        ]
        .join("\n")
            + "\n"
    );
    let entries: Value = serde_json::from_str(&listed_json.stdout).unwrap();
    let entries = entries["tree"].as_array().unwrap();
    let flat: Vec<(&str, &str, bool)> = entries
        .iter()
        .map(|entry| {
            (
                entry["subagentId"].as_str().unwrap(),
                entry["kind"].as_str().unwrap(),
                entry["self"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        flat,
        [
            (root().as_str(), "root", false),
            (alpha.as_str(), "live", false),
            (gamma.as_str(), "live", true),
            (beta.as_str(), "live", false),
            (delta.as_str(), "archived", false),
        ]
    );
    assert_eq!(entries[4]["phase"], "completed");
    assert_eq!(entries[4]["closedAgoMs"], 0);
    assert_eq!(
        shown.stdout,
        [
            format!("id: {beta}"),
            format!("parent: {}", root()),
            "description: beta".into(),
            "profile: (inherit)".into(),
            "phase: running".into(),
            "elapsed: 0s".into(),
            "execution: provider_streaming (for 0s)".into(),
            "last-event: 0s ago".into(),
            "activity: streaming".into(),
            "last assistant text: (none yet)".into(),
        ]
        .join("\n")
            + "\n"
    );
    assert_eq!(
        refused(&show_root),
        format!("demi agent show: no live agent \"{}\"\n", root())
    );

    // A child with a message waiting does not close before it reads it.
    beta_gate.open();
    let frames = frames_until(&mut client, is_closed(&beta)).await;
    assert!(
        !lifecycle(&frames)
            .iter()
            .any(|(event, id, _)| *event == SubagentEvent::Closed && id != &beta)
    );
    assert_eq!(model.requests_of("task beta").len(), 2);
    assert_eq!(
        closed_phase(&fixture, &beta),
        Some(ClosePhase::Completed {
            result: "beta heard".into()
        })
    );
    frames_until(&mut client, is_idle).await;
    let senders: Vec<NodeId> = root_receipts(&fixture)
        .into_iter()
        .map(|message| message.sender.id)
        .collect();
    assert_eq!(senders, [delta, beta]);
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
    let fixture = fixture(&model, TestHarness::default());
    let mut client = fixture.opened().await;
    let alpha = spawn(&fixture, &root(), json!({ "prompt": "task alpha" })).await;
    let gamma_start = tokio::task::spawn_local({
        let server = fixture.server.clone();
        let alpha = alpha.clone();
        async move { agent(&server, &alpha, "spawn", json!({ "prompt": "task gamma" })).await }
    });
    alpha_gate.open();
    let gamma = child_id(&gamma_start.await.unwrap());

    let aborted = agent(
        &fixture.server,
        &root(),
        "abort",
        json!({ "id": alpha.as_str() }),
    )
    .await;
    let frames = frames_until(&mut client, is_idle).await;

    assert_eq!(aborted.stdout, format!("aborted {alpha}\n"));
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
    frames_until(&mut client, |frame| *frame == ServerFrame::Closed).await;
    let detached = fixture.store.record(&beta).unwrap();
    assert!(detached.closed.is_none());
    assert_eq!(
        fixture.store.checkpoint(&beta).unwrap().state.phase,
        SessionPhase::Running
    );

    let mut reopened = fixture.client();
    reopened.send(open(test_model())).await;
    let handshake = reopened.received();
    let frames = frames_until(&mut reopened, is_idle).await;

    let started: Vec<NodeId> = lifecycle(&handshake)
        .into_iter()
        .chain(lifecycle(&frames))
        .filter(|(event, ..)| *event == SubagentEvent::Started)
        .map(|(_, id, _)| id)
        .collect();
    assert_eq!(started, [beta.clone()]);
    assert_eq!(model.requests_of("task beta").len(), 2);
    assert_eq!(
        closed_phase(&fixture, &beta),
        Some(ClosePhase::Completed {
            result: "beta after restart".into()
        })
    );
    assert!(model.is_done());
}

fn checkpoint(queue: Vec<QueuedMessage>, blocks: Vec<Block>) -> CheckpointUpdate {
    CheckpointUpdate {
        state: CheckpointState {
            phase: SessionPhase::Idle,
            queue,
            agent_inputs: Vec::new(),
            wakeups: Vec::new(),
            cwd: "/workspace".into(),
            model: test_model(),
            harness: "test".into(),
            edits: Vec::new(),
        },
        command_state: Some(CommandStateSnapshot::initial()),
        block_count: blocks.len(),
        changed_blocks: blocks.into_iter().enumerate().collect(),
    }
}

fn child_record(id: &str, parent: &str, profile: Option<&str>) -> NodeRecord {
    NodeRecord {
        id: NodeId::try_from(id).unwrap(),
        parent: Some(NodeId::try_from(parent).unwrap()),
        description: id.into(),
        profile: profile.map(str::to_owned),
        round: 1,
        can_spawn_subagents: true,
        closed: None,
        delivered: false,
    }
}

fn user(id: &str, words: &str) -> Block {
    Block::User(UserBlock {
        id: BlockId::try_from(id).unwrap(),
        turn_id: TurnId::try_from(id).unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        content: text(words),
        preamble: None,
    })
}

fn answer(id: &str, words: &str) -> Block {
    Block::Text(TextBlock {
        id: BlockId::try_from(id).unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        text: words.into(),
        forkable: true,
    })
}

#[tokio::test(flavor = "local")]
async fn a_restore_runs_a_lost_brief_closes_a_quiet_child_and_delivers_a_missed_completion() {
    let store = MemoryTreeStore::new();
    let root_record = NodeRecord::root(root(), Timestamp::UNIX_EPOCH);
    store
        .create_node(root_record, checkpoint(Vec::new(), Vec::new()))
        .await
        .unwrap();
    // Lost before its first save: the brief is still queued.
    let brief = QueuedMessage {
        id: TurnId::try_from("brief").unwrap(),
        content: text("task lost"),
    };
    store
        .create_node(
            child_record("lost", "conversation", None),
            checkpoint(vec![brief], Vec::new()),
        )
        .await
        .unwrap();
    // Its final checkpoint saved, its close not yet.
    store
        .create_node(
            child_record("quiet", "conversation", None),
            checkpoint(
                Vec::new(),
                vec![user("q1", "task quiet"), answer("q2", "quiet result")],
            ),
        )
        .await
        .unwrap();
    // Closed, its completion not in the parent's checkpoint.
    store
        .create_node(
            child_record("closed", "conversation", None),
            checkpoint(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    let close = NodeClose {
        phase: ClosePhase::Completed {
            result: "closed result".into(),
        },
        at: Timestamp::UNIX_EPOCH,
    };
    let closed = NodeId::try_from("closed").unwrap();
    store.close_node(&closed, close).await.unwrap();
    // A profile the harness no longer declares: gone with its subtree.
    store
        .create_node(
            child_record("orphan", "conversation", Some("retired")),
            checkpoint(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    store
        .create_node(
            child_record("orphan-child", "orphan", None),
            checkpoint(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    let model = Model::default();
    model.root([said("noted"), said("noted"), said("noted")]);
    model.child("task lost", [said("lost result")]);
    let fixture = Fixture::with_model(
        &model,
        TestHarness::default(),
        store.clone(),
        ServerConfig::default(),
    );
    let mut client = fixture.client();
    client.send(open(test_model())).await;

    until(|| root_receipts(&fixture).len() == 3).await;
    let tree = fixture.server.tree(&root()).unwrap();
    until(|| tree.is_quiescent()).await;

    let mut senders: Vec<String> = root_receipts(&fixture)
        .into_iter()
        .map(|message| message.sender.id.to_string())
        .collect();
    senders.sort();
    assert_eq!(senders, ["closed", "lost", "quiet"]);
    for (id, result) in [
        ("lost", "lost result"),
        ("quiet", "quiet result"),
        ("closed", "closed result"),
    ] {
        let id = NodeId::try_from(id).unwrap();
        assert_eq!(
            closed_phase(&fixture, &id),
            Some(ClosePhase::Completed {
                result: result.into()
            })
        );
        assert!(store.record(&id).unwrap().delivered, "{id}");
    }
    assert!(store.record(&NodeId::try_from("orphan").unwrap()).is_none());
    assert!(
        store
            .record(&NodeId::try_from("orphan-child").unwrap())
            .is_none()
    );
    let frames = client.received();
    assert!(frames.iter().any(is_pending_steers));
}

#[tokio::test(flavor = "local")]
async fn a_start_request_is_safe_to_retry_and_outlives_a_cancelled_call() {
    let model = Model::default();
    let root_gate = Gate::new();
    model.root([
        held_said(&root_gate, "busy"),
        said("noted first"),
        said("noted second"),
        said("noted third"),
    ]);
    model.child(
        "first task",
        [
            said("first done"),
            Turn::Respond(Box::new(|request| {
                let seen = request_text(request);
                assert!(seen.contains("first done") && seen.contains("second task"));
                vec![event::text("second done"), event::response(1, 1)]
            })),
            said("third done"),
        ],
    );
    let fixture = fixture(&model, TestHarness::default());
    let mut client = fixture.opened().await;
    client.send(send("m1", "work")).await;
    until(|| model.root_requests().len() == 1).await;

    // The call is cancelled after its reservation committed, while the start
    // waits for the busy parent's runtime.
    let cancel = CancellationToken::new();
    let call = tokio::task::spawn_local({
        let server = fixture.server.clone();
        let cancel = cancel.clone();
        async move {
            agent_call(
                &server,
                &root(),
                "spawn",
                json!({ "prompt": "first task", "request-id": "r1" }),
                false,
                cancel,
            )
            .await
        }
    });
    let reservation = || {
        let root = fixture.server.node(&root(), &root()).unwrap();
        async move {
            match root
                .storage(
                    StorageOp::Read {
                        key: "agent.start.r1".into(),
                    },
                    Vec::new(),
                )
                .await
            {
                Ok(StorageReply::Value { value, .. }) => value,
                other => panic!("{other:?}"),
            }
        }
    };
    while reservation().await.is_none() {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    call.abort();
    root_gate.open();
    let frames = frames_until(&mut client, |frame| {
        matches!(
            frame,
            ServerFrame::Subagent {
                event: SubagentEvent::Closed,
                ..
            }
        )
    })
    .await;
    let child = lifecycle(&frames)[0].1.clone();
    frames_until(&mut client, is_idle).await;

    let retried = agent(
        &fixture.server,
        &root(),
        "spawn",
        json!({ "prompt": "first task", "request-id": "r1" }),
    )
    .await;
    let conflicting = agent(
        &fixture.server,
        &root(),
        "spawn",
        json!({ "prompt": "other task", "request-id": "r1" }),
    )
    .await;
    let first_round = fixture.store.record(&child).unwrap().round;
    let resumed = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": child.as_str(), "message": "second task", "request-id": "r2" }),
    )
    .await;
    frames_until(&mut client, is_closed(&child)).await;
    frames_until(&mut client, is_idle).await;
    let second_round = fixture.store.record(&child).unwrap().round;
    let resumed_again = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": child.as_str(), "message": "second task", "request-id": "r2" }),
    )
    .await;
    let third = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": child.as_str(), "message": "third task", "request-id": "r3" }),
    )
    .await;
    frames_until(&mut client, is_closed(&child)).await;
    frames_until(&mut client, is_idle).await;
    let superseded = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": child.as_str(), "message": "second task", "request-id": "r2" }),
    )
    .await;

    for run in [&retried, &resumed, &resumed_again, &third] {
        assert_eq!(child_id(run), child, "{run:?}");
    }
    assert_eq!(
        refused(&conflicting),
        "demi agent spawn: request-id already belongs to different agent arguments\n"
    );
    assert_eq!(
        refused(&superseded),
        "demi agent resume: resume request has been superseded by a later round\n"
    );
    assert!(second_round > first_round);
    let rounds: Vec<u64> = root_receipts(&fixture)
        .into_iter()
        .map(|message| message.sender.round)
        .collect();
    let third_round = fixture.store.record(&child).unwrap().round;
    assert_eq!(rounds, [first_round, second_round, third_round]);
    assert_eq!(model.requests_of("first task").len(), 3);
    assert!(model.is_done());
}

#[tokio::test(flavor = "local")]
async fn a_node_has_at_most_eight_live_children_and_the_browser_aborts_them_all() {
    let model = Model::default();
    let gate = Gate::new();
    let fixture = fixture(&model, TestHarness::default());
    let mut client = fixture.opened().await;
    for index in 1..=8 {
        model.child(&format!("limit {index}:"), [held_said(&gate, "waits")]);
        spawn(
            &fixture,
            &root(),
            json!({ "prompt": format!("limit {index}: hold") }),
        )
        .await;
    }
    let ninth = agent(
        &fixture.server,
        &root(),
        "spawn",
        json!({ "prompt": "limit 9: hold" }),
    )
    .await;
    assert_eq!(
        refused(&ninth),
        "demi agent spawn: at most 8 running subagents per session; abort one or wait for a result\n"
    );

    // Each completion may wake the root once.
    model.root((0..8).map(|_| said("stopped")));
    client.send(ClientFrame::AbortSubagents {}).await;
    let tree = fixture.server.tree(&root()).unwrap();
    until(|| tree.is_quiescent()).await;
    let frames = client.received();
    let closes = lifecycle(&frames)
        .into_iter()
        .filter(|(event, _, phase)| *event == SubagentEvent::Closed && *phase == JobPhase::Aborted)
        .count();
    assert_eq!(closes, 8);
    assert_eq!(root_receipts(&fixture).len(), 8);
}

#[tokio::test(flavor = "local")]
async fn profiles_and_the_spawn_restriction_shape_a_childs_prompt_and_commands() {
    let explorer = Profile {
        name: "explorer".into(),
        description: "Reads, never edits.".into(),
        system_prompt: Some(Rc::new(|_, help: &str| format!("explorer prompt\n{help}"))),
        commands: None,
        can_spawn_subagents: false,
        model: None,
    };
    let harness = TestHarness {
        preamble: Some("harness preamble".into()),
        profiles: vec![explorer],
        ..TestHarness::default()
    };
    let model = Model::default();
    model.root([said("noted"), said("noted"), said("noted")]);
    model.child("task explore", [said("explored")]);
    let (restricted_gate, resumed_gate) = (Gate::new(), Gate::new());
    model.child(
        "task restricted",
        [
            held_said(&restricted_gate, "restricted done"),
            held_said(&resumed_gate, "more done"),
        ],
    );
    let fixture = fixture(&model, harness);
    let mut client = fixture.opened().await;

    let unknown = agent(
        &fixture.server,
        &root(),
        "spawn",
        json!({ "prompt": "task x", "profile": "nope" }),
    )
    .await;
    let explorer_child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task explore", "profile": "explorer" }),
    )
    .await;
    let restricted = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task restricted", "no-subagents": true }),
    )
    .await;
    let from_restricted = agent_call(
        &fixture.server,
        &restricted,
        "spawn",
        json!({ "prompt": "task grandchild" }),
        false,
        CancellationToken::new(),
    )
    .await;
    let restricted_lists = agent(&fixture.server, &restricted, "list", json!({})).await;
    restricted_gate.open();
    frames_until(&mut client, is_closed(&restricted)).await;
    until(|| fixture.store.record(&restricted).unwrap().delivered).await;
    agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": restricted.as_str(), "message": "more" }),
    )
    .await;
    let after_resume = agent_call(
        &fixture.server,
        &restricted,
        "spawn",
        json!({ "prompt": "task grandchild" }),
        false,
        CancellationToken::new(),
    )
    .await;
    resumed_gate.open();
    frames_until(&mut client, is_closed(&restricted)).await;

    assert_eq!(
        refused(&unknown),
        "demi agent spawn: unknown profile \"nope\" (available: explorer)\n"
    );
    let explored = &model.requests_of("task explore")[0];
    assert!(explored.system_prompt.starts_with("explorer prompt\n"));
    assert!(explored.system_prompt.contains("demi agent send"));
    assert!(!explored.system_prompt.contains("demi agent spawn"));
    let explorer_preamble = request_text(explored);
    assert!(explorer_preamble.starts_with("You are a subagent"));
    assert!(explorer_preamble.contains("This session may not spawn subagents."));
    assert!(!explorer_preamble.contains("harness preamble"));
    assert_eq!(
        fixture
            .store
            .record(&explorer_child)
            .unwrap()
            .profile
            .as_deref(),
        Some("explorer")
    );
    let restricted_asked = &model.requests_of("task restricted")[0];
    assert_eq!(restricted_asked.system_prompt, "system prompt");
    assert!(request_text(restricted_asked).starts_with("harness preamble\n\nYou are a subagent"));
    let is_missing_spawn = |call: &Result<CommandRun, RpcError>| matches!(call, Err(RpcError::Usage(message)) if message == "\"demi agent spawn\" is not an rpc command");
    assert!(is_missing_spawn(&from_restricted), "{from_restricted:?}");
    assert!(is_missing_spawn(&after_resume), "{after_resume:?}");
    assert_eq!(restricted_lists.code, 0);
    assert!(
        !fixture
            .store
            .record(&restricted)
            .unwrap()
            .can_spawn_subagents
    );
    let help = fixture.harness.prompts.borrow();
    assert!(help.iter().any(|help| !help.contains("demi agent spawn")));

    let reserved = Profile {
        name: "default".into(),
        description: String::new(),
        system_prompt: None,
        commands: None,
        can_spawn_subagents: true,
        model: None,
    };
    let refusing = self::fixture(
        &Model::default(),
        TestHarness {
            profiles: vec![reserved],
            ..TestHarness::default()
        },
    );
    let mut refused_client = refusing.client();
    refused_client.send(open(test_model())).await;
    assert_eq!(
        refused_client.received(),
        [ServerFrame::Error {
            message: "subagent profile name \"default\" is reserved: omitting --profile already inherits the parent".into(),
            code: None,
            diagnostics: None,
        }]
    );
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_detached_tree_with_a_live_child_is_not_evicted_and_one_without_is() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("child done")]);
    model.child("task long", [held_said(&gate, "long result")]);
    let fixture = fixture(&model, TestHarness::default());
    let client = fixture.opened().await;
    let child = spawn(&fixture, &root(), json!({ "prompt": "task long" })).await;
    drop(client);

    tokio::time::sleep(std::time::Duration::from_secs(700)).await;
    assert!(fixture.server.tree(&root()).is_some());
    gate.open();
    until(|| root_receipts(&fixture).len() == 1).await;
    tokio::time::sleep(std::time::Duration::from_secs(700)).await;
    assert!(fixture.server.tree(&root()).is_none());

    let reopened = fixture.client();
    reopened.send(open(test_model())).await;
    let listed = agent(&fixture.server, &root(), "list", json!({})).await;
    assert_eq!(
        listed.stdout,
        format!(
            "● {}  (root session) ← you\n└─○ {child}  archived (completed 0s ago)  (no description)\n",
            root()
        )
    );
    assert_eq!(root_receipts(&fixture).len(), 1);
}
