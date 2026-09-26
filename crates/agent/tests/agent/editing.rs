//! Message editing, Fork and command storage at the agent's boundary
//! (`message-editing.md`, `conversation-fork.md`,
//! `command-state-history.md`): the frames an edit is answered with, the
//! requests the replacement makes, and the checkpoints the store keeps.

use demi_agent::{
    AgentTreeStore, ForkError, ServerConfig,
    store::{ClosePhase, NodeClose},
    testing::{MemoryTreeStore, TestClient, TestFiles, client_text, test_model},
    transcript::CutError,
};
use demi_agent_protocol::{
    ClientContent, ClientFrame, EditOutcome, EditRequest, ServerFrame, TranscriptPatch,
    TranscriptVersion,
};
use demi_core::{
    Attachment, B64Bytes, BlobRef, Block, BlockId, MediaSource, NodeId, OperationId, SessionPhase,
    Timestamp, TurnId, UserContentBlock,
};
use demi_provider::{
    InferenceItem,
    testing::{ScriptedRuntime, Turn, event},
};
use demi_shell::{PortError, Revision, StorageOp, StorageReply};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::{
    subagents::{checkpoint, child_record},
    support::{
        CommandRun, Fixture, Gate, Model, TestHarness, agent, command_storage, conversation,
        frames_until, held, is_idle, kinds, open, send, until,
    },
};

fn said(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(1, 1)])
}

fn session_of(fixture: &Fixture) -> demi_agent::AgentSession {
    fixture
        .server
        .tree(&conversation())
        .expect("the tree is live")
        .root()
        .session()
        .clone()
}

/// The `user` block of the turn `turn`.
fn user_block(fixture: &Fixture, turn: &str) -> BlockId {
    session_of(fixture)
        .transcript()
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::User(user) if user.turn_id.as_str() == turn => Some(user.id.clone()),
            _ => None,
        })
        .expect("the turn has a user block")
}

fn edit(
    operation: &str,
    target: &BlockId,
    version: &TranscriptVersion,
    content: Vec<ClientContent>,
) -> ClientFrame {
    ClientFrame::EditAndSend {
        request: EditRequest {
            operation_id: OperationId::try_from(operation).unwrap(),
            target_block_id: target.clone(),
            version: version.clone(),
            content,
        },
    }
}

/// The outcome the first `edit_result` frame carries.
fn edit_outcome(frames: &[ServerFrame]) -> EditOutcome {
    frames
        .iter()
        .find_map(|frame| match frame {
            ServerFrame::EditResult { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no edit_result in {frames:#?}"))
}

fn rejected(reason: &str) -> EditOutcome {
    EditOutcome::Rejected {
        reason: reason.to_owned(),
    }
}

/// Writes `value` under `todo` in the root's command storage.
async fn write_todo(fixture: &Fixture, value: i64) -> StorageReply {
    let write = StorageOp::WriteIf {
        key: "todo".into(),
        value: Some(json!(value)),
        expected: None,
    };
    command_storage(&fixture.server, &conversation(), write)
        .await
        .unwrap()
}

async fn read_todo(fixture: &Fixture) -> StorageReply {
    let read = StorageOp::Read { key: "todo".into() };
    command_storage(&fixture.server, &conversation(), read)
        .await
        .unwrap()
}

/// A, B and C, each answered, with the command storage's `todo` at 1 before
/// B and at 2 before C.
async fn three_turns(fixture: &Fixture, client: &mut TestClient<TestHarness>) {
    for (value, (id, text)) in [("m1", "A"), ("m2", "B"), ("m3", "C")]
        .into_iter()
        .enumerate()
    {
        if value > 0 {
            write_todo(fixture, value as i64).await;
        }
        client.send(send(id, text)).await;
        frames_until(client, is_idle).await;
    }
}

#[tokio::test(flavor = "local")]
async fn an_edit_replaces_its_message_and_what_follows_once_and_infers_on_a_fresh_runtime() {
    let script = ScriptedRuntime::new([
        said("answer A"),
        said("answer B"),
        said("answer C"),
        said("answer B2"),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    three_turns(&fixture, &mut client).await;
    let target = user_block(&fixture, "m2");
    let before = session_of(&fixture).transcript();
    let answer_a = before.blocks[1].id().clone();

    let not_user = edit("op0", &answer_a, &before.version, client_text("x"));
    client.send(not_user).await;
    let not_user = edit_outcome(&client.received());
    client
        .send(edit("op1", &target, &before.version, client_text("B2")))
        .await;
    let frames = frames_until(&mut client, is_idle).await;
    let accepted = session_of(&fixture).transcript();
    // A repeated request is answered from its receipt; another request
    // under its id, or from the old snapshot, is refused.
    client
        .send(edit("op1", &target, &before.version, client_text("B2")))
        .await;
    let repeated = edit_outcome(&client.received());
    client
        .send(edit("op1", &target, &accepted.version, client_text("B3")))
        .await;
    let conflicting = edit_outcome(&client.received());
    client
        .send(edit("op2", &target, &before.version, client_text("B3")))
        .await;
    let stale = edit_outcome(&client.received());

    assert_eq!(not_user, rejected(&CutError::NotUserMessage.to_string()));
    let EditOutcome::Accepted { turn_id } = edit_outcome(&frames) else {
        panic!("{frames:#?}")
    };
    let replaces: Vec<&Vec<Block>> = frames
        .iter()
        .flat_map(|frame| match frame {
            ServerFrame::TranscriptPatch { patches, .. } => patches.as_slice(),
            _ => &[],
        })
        .filter_map(|patch| match patch {
            TranscriptPatch::Replace { value } => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(replaces.len(), 1, "one rewrite");
    // The acceptance answers right after the rewrite, before the
    // replacement's turn writes anything.
    let rewrite = frames
        .iter()
        .position(|frame| {
            matches!(frame, ServerFrame::TranscriptPatch { patches, .. }
            if matches!(patches.as_slice(), [TranscriptPatch::Replace { .. }]))
        })
        .unwrap();
    assert!(matches!(
        frames[rewrite + 1],
        ServerFrame::EditResult { .. }
    ));
    assert_eq!(replaces[0][..3], before.blocks[..3]);
    assert_eq!(kinds(replaces[0]), ["user", "text", "response", "user"]);
    let Block::User(replacement) = &replaces[0][3] else {
        unreachable!()
    };
    assert_eq!(replacement.turn_id, turn_id);
    assert_eq!(replacement.content, demi_agent::testing::text("B2"));
    assert_eq!(accepted.blocks[..4], replaces[0][..]);
    assert_eq!(
        kinds(&accepted.blocks),
        ["user", "text", "response", "user", "text", "response"]
    );
    // The replacement's request carries the prefix and B2, nothing of B, C
    // or their answers, on a runtime of its own: the discarded one closed.
    let request = script.requests().pop().unwrap();
    assert_eq!(
        *request.items,
        [
            InferenceItem::UserMessage {
                content: demi_agent::testing::text("A")
            },
            InferenceItem::AssistantText {
                model_id: "test-model".into(),
                text: "answer A".into()
            },
            InferenceItem::UserMessage {
                content: demi_agent::testing::text("B2")
            },
        ]
    );
    assert_eq!(script.closes(), 1);
    let stored = fixture.store.checkpoint(&conversation()).unwrap();
    assert_eq!(stored.transcript, accepted.blocks);
    assert_eq!(stored.state.edits.len(), 1);
    assert_eq!(stored.state.edits[0].turn_id, turn_id);
    assert_eq!(
        repeated,
        EditOutcome::Accepted {
            turn_id: turn_id.clone()
        }
    );
    assert_eq!(
        conflicting,
        rejected("The edit operation ID was used for a different request")
    );
    assert_eq!(
        stale,
        rejected("The conversation changed; reopen the message to edit it")
    );
    assert_eq!(script.remaining(), 0);
    // Command state is back at the version before B.
    assert_eq!(
        read_todo(&fixture).await,
        StorageReply::Value {
            value: Some(json!(1)),
            revision: Revision(1)
        }
    );

    // A snapshot taken before a restart is stale after it, and an accepted
    // operation keeps its receipt.
    client.send(ClientFrame::Close {}).await;
    frames_until(&mut client, |frame| *frame == ServerFrame::Closed).await;
    let mut reopened = fixture.client();
    reopened.send(open(test_model())).await;
    reopened.received();
    let replacement_block = user_block(&fixture, turn_id.as_str());
    reopened
        .send(edit(
            "op3",
            &replacement_block,
            &accepted.version,
            client_text("B4"),
        ))
        .await;
    assert_eq!(
        edit_outcome(&reopened.received()),
        rejected("The conversation changed; reopen the message to edit it")
    );
    reopened
        .send(edit("op1", &target, &before.version, client_text("B2")))
        .await;
    assert_eq!(
        edit_outcome(&reopened.received()),
        EditOutcome::Accepted { turn_id }
    );
}

#[tokio::test(flavor = "local")]
async fn an_edit_is_refused_while_work_waits_and_a_failed_save_changes_nothing() {
    let gate = Gate::new();
    let script = ScriptedRuntime::new([
        said("answer A"),
        held(&gate, vec![event::text("answer B"), event::response(1, 1)]),
        said("answer A2"),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    frames_until(&mut client, is_idle).await;
    let target = user_block(&fixture, "m1");

    client.send(send("m2", "B")).await;
    let busy_version = session_of(&fixture).transcript().version;
    client
        .send(edit("op1", &target, &busy_version, client_text("A2")))
        .await;
    let busy = edit_outcome(&client.received());
    gate.open();
    frames_until(&mut client, is_idle).await;

    let before = session_of(&fixture).transcript();
    let job = fixture
        .server
        .node(&conversation(), &conversation())
        .unwrap()
        .job_caller();
    fixture.store.fail_saves(1);
    client
        .send(edit("op2", &target, &before.version, client_text("A2")))
        .await;
    let failed = edit_outcome(&client.received());
    let read = StorageOp::Read { key: "todo".into() };
    let generation_kept = fixture
        .server
        .command_storage(&conversation(), &job, read, CancellationToken::new())
        .await
        .is_ok();
    let after_failure = session_of(&fixture).transcript();
    let stored = fixture.store.checkpoint(&conversation()).unwrap();
    client
        .send(edit("op2", &target, &before.version, client_text("A2")))
        .await;
    let frames = frames_until(&mut client, is_idle).await;

    assert_eq!(
        busy,
        rejected("Message editing requires a settled session with no pending work")
    );
    assert_eq!(failed, rejected("the database refused the save"));
    assert_eq!(after_failure, before);
    assert!(generation_kept, "a rejected edit ends no job's storage");
    assert_eq!(stored.transcript, before.blocks);
    assert!(stored.state.edits.is_empty());
    assert!(matches!(
        edit_outcome(&frames),
        EditOutcome::Accepted { .. }
    ));
    assert_eq!(
        kinds(&session_of(&fixture).transcript().blocks),
        ["user", "text", "response"]
    );
}

#[tokio::test(flavor = "local")]
async fn an_edit_keeps_the_files_its_message_holds_and_refuses_a_path_it_does_not() {
    let script = ScriptedRuntime::new([said("a chart"), said("the same chart")]);
    let fixture = Fixture::new(&script);
    let files = TestFiles::new();
    let image = UserContentBlock::Image {
        source: MediaSource::Binary {
            data: B64Bytes::new(vec![0x89, b'P', b'N', b'G']),
            media_type: "image/png".into(),
        },
    };
    let record = UserContentBlock::Attachment(Attachment {
        name: "chart.png".into(),
        path: "/home/demi/.demi/attachments/conversation/chart.png".into(),
        media_type: "image/png".into(),
        size_bytes: 4,
        sha256: BlobRef::try_from("a".repeat(64)).unwrap(),
        snippet: None,
    });
    files.upload("upload-1", vec![image.clone(), record.clone()]);
    let mut client =
        TestClient::connect_with(&fixture.server, &conversation(), "/workspace", files);
    client.send(open(test_model())).await;
    client.received();
    client
        .send(ClientFrame::Send {
            message_id: TurnId::try_from("m1").unwrap(),
            content: vec![
                ClientContent::Text {
                    text: "look".into(),
                },
                ClientContent::Upload {
                    r#ref: "upload-1".into(),
                    file_name: "chart.png".into(),
                },
            ],
        })
        .await;
    frames_until(&mut client, is_idle).await;
    let target = user_block(&fixture, "m1");
    let version = session_of(&fixture).transcript().version;

    let unknown = vec![ClientContent::Attachment {
        path: "/elsewhere/chart.png".into(),
    }];
    client.send(edit("op1", &target, &version, unknown)).await;
    let refused = edit_outcome(&client.received());
    let kept = vec![
        ClientContent::Text {
            text: "look again".into(),
        },
        ClientContent::Attachment {
            path: "/home/demi/.demi/attachments/conversation/chart.png".into(),
        },
    ];
    client.send(edit("op2", &target, &version, kept)).await;
    let frames = frames_until(&mut client, is_idle).await;

    assert_eq!(
        refused,
        rejected("The edited message holds no attachment at /elsewhere/chart.png")
    );
    assert!(matches!(
        edit_outcome(&frames),
        EditOutcome::Accepted { .. }
    ));
    let Some(Block::User(replacement)) = session_of(&fixture)
        .transcript()
        .blocks
        .into_iter()
        .find(|block| matches!(block, Block::User(_)))
    else {
        panic!("the replacement is the first user block")
    };
    assert_eq!(
        replacement.content,
        [
            UserContentBlock::Text {
                text: "look again".into()
            },
            record
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_fork_seed_keeps_the_history_through_a_completed_text_from_a_live_or_a_stored_root() {
    let script = ScriptedRuntime::new([
        said("answer A"),
        said("answer B"),
        Turn::Respond(Box::new(|request| {
            assert_eq!(
                *request.items,
                [
                    InferenceItem::UserMessage {
                        content: demi_agent::testing::text("A")
                    },
                    InferenceItem::AssistantText {
                        model_id: "test-model".into(),
                        text: "answer A".into()
                    },
                    InferenceItem::UserMessage {
                        content: demi_agent::testing::text("U3")
                    },
                ]
            );
            vec![event::text("answer U3"), event::response(1, 1)]
        })),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    let write = |value: i64, expected: u64| StorageOp::WriteIf {
        key: "todo".into(),
        value: Some(json!(value)),
        expected: Some(Revision(expected)),
    };
    command_storage(&fixture.server, &conversation(), write(1, 0))
        .await
        .unwrap();
    client.send(send("m1", "A")).await;
    frames_until(&mut client, is_idle).await;
    command_storage(&fixture.server, &conversation(), write(2, 1))
        .await
        .unwrap();
    client.send(send("m2", "B")).await;
    frames_until(&mut client, is_idle).await;
    let blocks = session_of(&fixture).transcript().blocks;
    let answer_a = blocks[1].id().clone();
    let user_b = blocks[3].id().clone();

    let live = fixture
        .server
        .prepare_fork(&conversation(), &answer_a)
        .await
        .unwrap();
    let not_text = fixture.server.prepare_fork(&conversation(), &user_b).await;
    client.send(ClientFrame::Close {}).await;
    frames_until(&mut client, |frame| *frame == ServerFrame::Closed).await;
    let cold = fixture
        .server
        .prepare_fork(&conversation(), &answer_a)
        .await
        .unwrap();
    let mut running = cold.clone();
    running.state.phase = SessionPhase::Running;
    let destination = NodeId::try_from("fork").unwrap();
    let invalid = fixture.server.initialize_fork(&destination, running).await;
    fixture
        .server
        .initialize_fork(&destination, live.clone())
        .await
        .unwrap();

    assert_eq!(live, cold);
    assert_eq!(live.transcript[..], blocks[..2]);
    assert_eq!(live.state.phase, SessionPhase::Idle);
    assert!(live.state.queue.is_empty() && live.state.edits.is_empty());
    assert_eq!(not_text, Err(ForkError::Target(CutError::NotCompletedText)));
    assert_eq!(invalid, Err(ForkError::InvalidSeed));
    let record = fixture.store.record(&destination).unwrap();
    assert_eq!((record.parent, record.closed), (None, None));

    // The destination opens idle with the prefix and its command state, and
    // continues on its own.
    let mut forked = TestClient::connect(&fixture.server, &destination, "/workspace");
    forked.send(open(test_model())).await;
    let handshake = forked.received();
    let Some(ServerFrame::TranscriptReset { blocks: seeded, .. }) = handshake.get(1) else {
        panic!("{handshake:#?}")
    };
    assert_eq!(seeded[..], blocks[..2]);
    let read = command_storage(
        &fixture.server,
        &destination,
        StorageOp::Read { key: "todo".into() },
    )
    .await
    .unwrap();
    assert_eq!(
        read,
        StorageReply::Value {
            value: Some(json!(1)),
            revision: Revision(1)
        }
    );
    forked
        .send(ClientFrame::Send {
            message_id: TurnId::try_from("m3").unwrap(),
            content: client_text("U3"),
        })
        .await;
    frames_until(&mut forked, is_idle).await;
    assert_eq!(script.remaining(), 0);
}

#[tokio::test(flavor = "local")]
async fn command_storage_writes_compare_the_revision_and_a_rewrite_ends_older_jobs() {
    let script = ScriptedRuntime::new([said("answer A"), said("answer A again")]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    let write = |value: i64, expected: Option<u64>| StorageOp::WriteIf {
        key: "todo".into(),
        value: Some(json!(value)),
        expected: expected.map(Revision),
    };
    let root = conversation();
    let storage = |op| command_storage(&fixture.server, &root, op);
    let before_job = fixture
        .server
        .node(&conversation(), &conversation())
        .unwrap()
        .job_caller();
    let first = storage(write(1, Some(0))).await.unwrap();
    let stale = storage(write(2, Some(0))).await.unwrap();
    let same = storage(write(1, None)).await.unwrap();
    client.send(send("m1", "A")).await;
    frames_until(&mut client, is_idle).await;
    let stored = fixture.store.checkpoint(&conversation()).unwrap();
    client.send(ClientFrame::Retry {}).await;
    frames_until(&mut client, is_idle).await;
    let after_rewrite = fixture
        .server
        .command_storage(
            &conversation(),
            &before_job,
            write(3, None),
            CancellationToken::new(),
        )
        .await;
    let fresh_job = storage(write(3, None)).await.unwrap();
    // A job's `demi agent list` reads the tree through the same node.
    let listed = agent(&fixture.server, &conversation(), "list", json!({})).await;
    fixture.store.fail_saves(1);
    let failed = storage(write(4, None)).await;
    let after_failure = read_todo(&fixture).await;

    assert_eq!(
        first,
        StorageReply::Committed {
            revision: Revision(1)
        }
    );
    assert_eq!(
        stale,
        StorageReply::Conflict {
            revision: Revision(1)
        }
    );
    assert_eq!(
        same,
        StorageReply::Committed {
            revision: Revision(1)
        }
    );
    let versions: Vec<u64> = stored
        .command_state
        .versions
        .iter()
        .map(|version| version.revision)
        .collect();
    assert_eq!(versions, [0, 1]);
    assert_eq!(
        after_rewrite,
        Err(PortError::Storage(
            "the command storage handle is no longer current".into()
        ))
    );
    assert_eq!(
        fresh_job,
        StorageReply::Committed {
            revision: Revision(2)
        }
    );
    assert_eq!(
        listed.stdout,
        format!("● {}  (root session) ← you\n", conversation())
    );
    // A commit that failed leaves the head as it was.
    assert_eq!(
        failed,
        Err(PortError::Storage("the database refused the save".into()))
    );
    assert_eq!(
        after_failure,
        StorageReply::Value {
            value: Some(json!(3)),
            revision: Revision(2)
        }
    );
}

/// The child a `spawn` named on stdout.
fn spawned(run: &CommandRun) -> NodeId {
    let id = run
        .stdout
        .strip_prefix("subagentId: ")
        .and_then(|rest| rest.strip_suffix('\n'))
        .unwrap_or_else(|| panic!("{run:?} names no child"));
    NodeId::try_from(id).unwrap()
}

#[tokio::test(flavor = "local")]
async fn an_edit_waits_for_no_child_and_an_edit_and_a_child_start_refuse_each_other() {
    let model = Model::default();
    model.root([
        said("answer A"),
        said("noted"),
        said("answer A2"),
        said("noted again"),
    ]);
    let reading = Gate::new();
    model.child(
        "Read notes.md",
        [held(
            &reading,
            vec![event::text("notes say 42"), event::response(1, 1)],
        )],
    );
    model.child("Count the files", [said("7 files")]);
    let fixture = Fixture::with_model(
        &model,
        TestHarness::default(),
        MemoryTreeStore::new(),
        ServerConfig::default(),
    );
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    frames_until(&mut client, is_idle).await;
    let target = user_block(&fixture, "m1");
    let root = conversation();
    let spawn = |prompt: &str| agent(&fixture.server, &root, "spawn", json!({ "prompt": prompt }));

    // A live child refuses the edit and keeps its record.
    let child = spawned(&spawn("Read notes.md").await);
    let live = fixture.store.record(&child).unwrap();
    let version = session_of(&fixture).transcript().version;
    client
        .send(edit("op1", &target, &version, client_text("A2")))
        .await;
    assert_eq!(
        edit_outcome(&client.received()),
        rejected("Cannot edit while children or completion notifications are pending")
    );
    assert_eq!(fixture.store.record(&child).unwrap(), live);
    reading.open();
    frames_until(&mut client, is_idle).await;
    let delivered = fixture.store.record(&child).unwrap();
    assert!(delivered.closed.is_some() && delivered.delivered);

    // While an edit saves, a child start is refused; the edit is accepted,
    // and the delivered child's record stays as it was.
    let version = session_of(&fixture).transcript().version;
    let gate = fixture.store.hold_saves();
    {
        let editing = client.send(edit("op2", &target, &version, client_text("A2")));
        tokio::pin!(editing);
        while gate.waiting() == 0 {
            assert!(futures_util::poll!(editing.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        let refused = spawn("Count the files").await;
        assert_eq!(
            (refused.code, refused.stderr.as_str()),
            (
                1,
                "demi agent spawn: Cannot change children while a transcript edit is being prepared\n"
            )
        );
        gate.release();
        editing.await;
    }
    let frames = frames_until(&mut client, is_idle).await;
    let EditOutcome::Accepted { turn_id } = edit_outcome(&frames) else {
        panic!("{frames:#?}")
    };
    assert_eq!(fixture.store.record(&child).unwrap(), delivered);

    // While a child start saves its request, an edit is refused.
    let target = user_block(&fixture, turn_id.as_str());
    let version = session_of(&fixture).transcript().version;
    let gate = fixture.store.hold_saves();
    {
        let starting = spawn("Count the files");
        tokio::pin!(starting);
        while gate.waiting() == 0 {
            assert!(futures_util::poll!(starting.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        client
            .send(edit("op3", &target, &version, client_text("A3")))
            .await;
        assert_eq!(
            edit_outcome(&client.received()),
            rejected("Cannot edit while a child lifecycle operation is in progress")
        );
        gate.release();
        assert_eq!(starting.await.code, 0);
    }
    frames_until(&mut client, is_idle).await;
    assert!(model.is_done());
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_completion_whose_saves_failed_refuses_an_edit_until_a_later_save_delivers_it() {
    let model = Model::default();
    model.root([said("answer A"), said("answer B"), said("answer A2")]);
    let fixture = Fixture::with_model(
        &model,
        TestHarness::default(),
        MemoryTreeStore::new(),
        ServerConfig::default(),
    );
    let store = &fixture.store;
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    frames_until(&mut client, is_idle).await;
    client.send(ClientFrame::Close {}).await;
    // A child of the root closed before the root saved its completion, as a
    // process that died between the two leaves them.
    let child = NodeId::try_from("child").unwrap();
    store
        .create_node(
            child_record("child", "conversation", None),
            checkpoint(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    let close = NodeClose {
        phase: ClosePhase::Completed {
            result: "notes say 42".into(),
        },
        at: Timestamp::UNIX_EPOCH,
    };
    store.close_node(&child, close).await.unwrap();

    // The database refuses every save while the tree opens again: the save
    // of the completion the opening delivers fails, and so does the turn
    // the completion opens. The root ends idle, the completion undelivered.
    store.fail_saves(usize::MAX);
    let mut client = fixture.client();
    client.send(open(test_model())).await;
    let tree = fixture.server.tree(&conversation()).unwrap();
    until(|| tree.is_quiescent()).await;
    store.fail_saves(0);
    let undelivered = store.record(&child).unwrap();
    assert!(!undelivered.delivered);

    // The record refuses the edit and stays as it was.
    let target = user_block(&fixture, "m1");
    let version = session_of(&fixture).transcript().version;
    client
        .send(edit("op1", &target, &version, client_text("A2")))
        .await;
    // The refused edit's action has ended, so none of its frames is left
    // for the next wait.
    until(|| tree.is_quiescent()).await;
    assert_eq!(
        edit_outcome(&client.received()),
        rejected("Cannot edit while children or completion notifications are pending")
    );
    assert_eq!(store.record(&child).unwrap(), undelivered);

    // The next save carries the completion; then the edit is accepted.
    client.send(send("m2", "B")).await;
    frames_until(&mut client, is_idle).await;
    assert!(store.record(&child).unwrap().delivered);
    let version = session_of(&fixture).transcript().version;
    client
        .send(edit("op2", &target, &version, client_text("A2")))
        .await;
    let frames = frames_until(&mut client, is_idle).await;
    assert!(matches!(
        edit_outcome(&frames),
        EditOutcome::Accepted { .. }
    ));
    assert!(model.is_done());
}
