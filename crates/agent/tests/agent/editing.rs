//! Message editing, Fork and command storage at the agent's boundary
//! (`message-editing.md`, `conversation-fork.md`,
//! `command-state-history.md`): the frames an edit is answered with, the
//! requests the replacement makes, and the checkpoints the store keeps.

use demi_agent::{
    ForkError,
    testing::{TestClient, TestFiles, client_text, test_model},
    transcript::CutError,
};
use demi_agent_protocol::{
    ClientContent, ClientFrame, EditOutcome, EditRequest, ServerFrame, TranscriptPatch,
    TranscriptVersion,
};
use demi_core::{
    Attachment, B64Bytes, BlobRef, Block, BlockId, MediaSource, NodeId, OperationId, SessionPhase,
    TurnId, UserContentBlock,
};
use demi_provider::{
    InferenceItem,
    testing::{ScriptedRuntime, Turn, event},
};
use demi_shell::{PortError, Revision, StorageOp, StorageReply};
use serde_json::json;

use crate::support::{
    Fixture, Gate, TestHarness, agent, conversation, frames_until, held, is_idle, kinds, open, send,
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
    fixture
        .server
        .node(&conversation(), &conversation())
        .unwrap()
        .storage(
            StorageOp::WriteIf {
                key: "todo".into(),
                value: Some(json!(value)),
                expected: None,
            },
            Vec::new(),
        )
        .await
        .unwrap()
}

async fn read_todo(fixture: &Fixture) -> StorageReply {
    fixture
        .server
        .node(&conversation(), &conversation())
        .unwrap()
        .storage(StorageOp::Read { key: "todo".into() }, Vec::new())
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
    let generation = fixture
        .server
        .node(&conversation(), &conversation())
        .unwrap()
        .command_generation();
    fixture.store.fail_saves(1);
    client
        .send(edit("op2", &target, &before.version, client_text("A2")))
        .await;
    let failed = edit_outcome(&client.received());
    let generation_kept = !generation.is_cancelled();
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
    let root = fixture
        .server
        .node(&conversation(), &conversation())
        .unwrap();
    let write = |value: i64, expected: u64| StorageOp::WriteIf {
        key: "todo".into(),
        value: Some(json!(value)),
        expected: Some(Revision(expected)),
    };
    root.storage(write(1, 0), Vec::new()).await.unwrap();
    client.send(send("m1", "A")).await;
    frames_until(&mut client, is_idle).await;
    root.storage(write(2, 1), Vec::new()).await.unwrap();
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
    let fork_root = fixture.server.node(&destination, &destination).unwrap();
    let read = fork_root
        .storage(StorageOp::Read { key: "todo".into() }, Vec::new())
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
    let root = fixture
        .server
        .node(&conversation(), &conversation())
        .unwrap();
    let write = |value: i64, expected: Option<u64>| StorageOp::WriteIf {
        key: "todo".into(),
        value: Some(json!(value)),
        expected: expected.map(Revision),
    };
    let before_job = root.command_generation();
    let first = root.storage(write(1, Some(0)), Vec::new()).await.unwrap();
    let stale = root.storage(write(2, Some(0)), Vec::new()).await.unwrap();
    let same = root.storage(write(1, None), Vec::new()).await.unwrap();
    client.send(send("m1", "A")).await;
    frames_until(&mut client, is_idle).await;
    let stored = fixture.store.checkpoint(&conversation()).unwrap();
    client.send(ClientFrame::Retry {}).await;
    frames_until(&mut client, is_idle).await;
    let after_rewrite = root.storage(write(3, None), vec![before_job.clone()]).await;
    let fresh_job = root
        .storage(write(3, None), vec![root.command_generation()])
        .await
        .unwrap();
    // A job's `demi agent list` reads the tree through the same node.
    let listed = agent(&fixture.server, &conversation(), "list", json!({})).await;
    fixture.store.fail_saves(1);
    let failed = root.storage(write(4, None), Vec::new()).await;
    let after_failure = read_todo(&fixture).await;
    let generation = root.command_generation();
    client.send(ClientFrame::Close {}).await;
    frames_until(&mut client, |frame| *frame == ServerFrame::Closed).await;
    let after_dispose = root.storage(write(5, None), vec![generation]).await;

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
    assert!(
        before_job.is_cancelled(),
        "the retry's rewrite ended the generation"
    );
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
    // A commit that failed leaves the head as it was; dispose ends the jobs.
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
    assert_eq!(
        after_dispose,
        Err(PortError::Storage(
            "the command storage handle is no longer current".into()
        ))
    );
}
