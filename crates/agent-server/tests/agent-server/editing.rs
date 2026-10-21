//! Message editing and Fork at the agent's boundary: the frames an edit is
//! answered with, the requests the replacement makes, and the checkpoints
//! the store keeps.

use demi_agent_server::{
    ServerConfig,
    testing::{TestClient, TestFiles, client_text, waiting_frames},
};
use demi_agent_session::ForkError;
use demi_agent_store::{
    AgentTreeStore, ClosePhase, NodeClose,
    testing::{MemoryTreeStore, model_of, model_reading},
};
use demi_agent_store::{
    media::{BlobStore, HeldMedia},
    testing::MemoryBlobs,
};
use demi_agent_transcript::CutError;
use demi_conversation_socket_protocol::MediaRef;
use demi_conversation_socket_protocol::{
    AgentCode, ClientContent, ClientFrame, EditOutcome, EditRequest, Refusal, ServerFrame, TranscriptPatch,
    Version,
};
use demi_provider_common::{
    InferenceItem, MediaBytes, Medium, UserPart,
    testing::{ScriptedRuntime, Turn, event},
};
use demi_shared_types::{
    Attachment, B64Bytes, BlobRef, Block, BlockId, DocumentSource, FileExtension, MediaSource,
    MessageComment, NodeId, OperationId, SessionPhase, Timestamp, TurnId, UserContentBlock, attachment_tag,
};
use serde_json::json;

use crate::{
    subagents::{checkpoint, child_record},
    support::{
        Fixture, Gate, Model, TestProduct, agent, conversation, held, kinds, named_node, send, session_of, switch, until,
    },
};

pub(crate) fn said(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(1, 1)])
}

/// The `user` block of the turn `turn`.
pub(crate) fn user_block(fixture: &Fixture, turn: &str) -> BlockId {
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

pub(crate) fn edit(
    operation: &str,
    target: &BlockId,
    version: &Version,
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
pub(crate) fn edit_outcome(frames: &[ServerFrame]) -> EditOutcome {
    frames
        .iter()
        .find_map(|frame| match frame {
            ServerFrame::EditResult { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no edit_result in {frames:#?}"))
}

/// Whether a frame tells the page of an edit: its result, or a change of the
/// transcript.
fn tells_of_an_edit(frame: &ServerFrame) -> bool {
    matches!(
        frame,
        ServerFrame::EditResult { .. }
            | ServerFrame::TranscriptPatch { .. }
            | ServerFrame::TranscriptReset { .. }
    )
}

fn rejected(code: AgentCode, reason: &str) -> EditOutcome {
    EditOutcome::Rejected {
        refusal: Refusal::new(code, reason),
    }
}

/// A, B and C, each answered.
async fn three_turns(client: &mut TestClient<TestProduct>) {
    for (id, text) in [("m1", "A"), ("m2", "B"), ("m3", "C")] {
        client.send(send(id, text)).await;
        client.until_idle().await;
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
    three_turns(&mut client).await;
    let target = user_block(&fixture, "m2");
    let before = session_of(&fixture).transcript();
    let answer_a = before.blocks[1].id().clone();

    let not_user = edit("op0", &answer_a, &before.version, client_text("x"));
    client.send(not_user).await;
    let not_user = edit_outcome(&client.received());
    client
        .send(edit("op1", &target, &before.version, client_text("B2")))
        .await;
    let frames = client.until_idle().await;
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

    assert_eq!(not_user, rejected(AgentCode::EditTarget, &CutError::NotUserMessage.to_string()));
    let EditOutcome::Accepted { turn_id } = edit_outcome(&frames) else {
        panic!("{frames:#?}")
    };
    // The rewrite is one batch: a cut after the kept turn and the message
    // added, which never sends the kept turn again.
    let rewrites: Vec<(usize, &[TranscriptPatch])> = frames
        .iter()
        .enumerate()
        .filter_map(|(position, frame)| match frame {
            ServerFrame::TranscriptPatch { patches, .. }
                if matches!(patches.first(), Some(TranscriptPatch::Truncate { .. })) =>
            {
                Some((position, patches.as_slice()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(rewrites.len(), 1, "one rewrite");
    let (rewrite, patches) = rewrites[0];
    // The acceptance answers right after the rewrite, before the
    // replacement's turn writes anything.
    assert!(matches!(
        frames[rewrite + 1],
        ServerFrame::EditResult { .. }
    ));
    let [
        TranscriptPatch::Truncate { length: 3 },
        TranscriptPatch::Add { index: 3, value: Block::User(replacement), .. },
    ] = patches
    else {
        panic!("{patches:#?}")
    };
    assert_eq!(replacement.turn_id, turn_id);
    assert_eq!(replacement.content, demi_agent_store::testing::text("B2"));
    assert_eq!(accepted.blocks[..3], before.blocks[..3]);
    assert_eq!(accepted.blocks[3], Block::User(replacement.clone()));
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
                content: demi_agent_store::testing::sent_text("A")
            },
            InferenceItem::AssistantText {
                model_id: "test-model".into(),
                text: "answer A".into()
            },
            InferenceItem::UserMessage {
                content: demi_agent_store::testing::sent_text("B2")
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
        rejected(AgentCode::EditConflict, "The edit operation ID was used for a different request")
    );
    assert_eq!(
        stale,
        rejected(AgentCode::EditStale, "The conversation changed; reopen the message to edit it")
    );
    assert_eq!(script.remaining(), 0);

    // A snapshot taken before a restart is stale after it, and an accepted
    // operation keeps its receipt.
    client.send(ClientFrame::Close {}).await;
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;
    let mut reopened = fixture.client();
    reopened.open().await;
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
        rejected(AgentCode::EditStale, "The conversation changed; reopen the message to edit it")
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
async fn the_page_hears_of_an_edit_only_once_its_save_commits() {
    let script = ScriptedRuntime::new([said("answer A"), said("answer A2")]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    client.until_idle().await;
    let target = user_block(&fixture, "m1");
    let version = session_of(&fixture).transcript().version;

    // While the edit's save waits, the outbox, which the page reads in
    // order, holds nothing of the edit.
    let gate = fixture.store.hold_saves();
    {
        let (connection, outbox) = client.split();
        let editing = connection.handle(edit("op1", &target, &version, client_text("A2")));
        tokio::pin!(editing);
        while gate.waiting() == 0 {
            assert!(futures_util::poll!(editing.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        let meanwhile = waiting_frames(outbox);
        assert!(!meanwhile.iter().any(tells_of_an_edit), "{meanwhile:#?}");
        gate.release();
        editing.await;
    }
    // Once it commits, the result and the replacement follow.
    let answered = client.until_idle().await;
    assert!(
        matches!(edit_outcome(&answered), EditOutcome::Accepted { .. }),
        "{answered:#?}"
    );
    assert!(
        answered.iter().any(|frame| matches!(
            frame,
            ServerFrame::TranscriptPatch { .. } | ServerFrame::TranscriptReset { .. }
        )),
        "{answered:#?}"
    );
}

/// A model switch while an edit is being prepared is not refused: it waits
/// for the edit. The replacement turn's first request carries the model it
/// was prepared with, and its next request the new one.
#[tokio::test(flavor = "local")]
async fn a_switch_while_an_edit_is_prepared_lands_after_the_replacements_first_request() {
    let script = ScriptedRuntime::new([
        said("answer A"),
        Turn::Events(vec![
            event::tool_call("call-1", "shell", json!({})),
            event::response(1, 1),
        ]),
        said("answer A2"),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    client.until_idle().await;
    let target = user_block(&fixture, "m1");
    let version = session_of(&fixture).transcript().version;

    let gate = fixture.store.hold_saves();
    {
        let editing = client.send(edit("op1", &target, &version, client_text("A2")));
        tokio::pin!(editing);
        while gate.waiting() == 0 {
            assert!(futures_util::poll!(editing.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        switch(&fixture, model_of("stub", "model-b")).await;
        gate.release();
        editing.await;
    }
    let frames = client.until_idle().await;
    assert!(
        matches!(edit_outcome(&frames), EditOutcome::Accepted { .. }),
        "{frames:#?}"
    );
    let models: Vec<String> = script
        .requests()
        .iter()
        .map(|request| request.model_id.clone())
        .collect();
    assert_eq!(models, ["test-model", "test-model", "model-b"]);
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
    client.until_idle().await;
    let target = user_block(&fixture, "m1");

    client.send(send("m2", "B")).await;
    let busy_version = session_of(&fixture).transcript().version;
    client
        .send(edit("op1", &target, &busy_version, client_text("A2")))
        .await;
    let busy = edit_outcome(&client.received());
    gate.open();
    client.until_idle().await;

    let before = session_of(&fixture).transcript();
    fixture.store.fail_saves(1);
    client
        .send(edit("op2", &target, &before.version, client_text("A2")))
        .await;
    let failed = edit_outcome(&client.received());
    let after_failure = session_of(&fixture).transcript();
    let stored = fixture.store.checkpoint(&conversation()).unwrap();
    client
        .send(edit("op2", &target, &before.version, client_text("A2")))
        .await;
    let frames = client.until_idle().await;

    assert_eq!(
        busy,
        rejected(AgentCode::EditBusy, "Message editing requires a settled session with no pending work")
    );
    assert_eq!(failed, rejected(AgentCode::SaveFailed, "the database refused the save"));
    assert_eq!(after_failure, before);
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

/// An edit keeps a file by naming it: an attachment record by its number, and
/// an image, a video or a document by its kind and blob.
///
/// The session puts the edited message's own block in its place, so the model
/// reads the bytes and the block keeps its media type and file name. A file the
/// message does not hold is refused.
#[tokio::test(flavor = "local")]
async fn an_edit_keeps_the_files_its_message_holds_and_refuses_one_it_does_not() {
    let script = ScriptedRuntime::new([said("a chart"), said("the same chart")]);
    // The store keeps media in a blob namespace, as the product's does, so
    // the page names the media the message holds by their blobs.
    let blobs = MemoryBlobs::new();
    let store = MemoryTreeStore::with_blobs(blobs.clone());
    let fixture = Fixture::with(&script, store, ServerConfig::default());
    fixture.resolver.select(model_reading(
        "stub",
        "test-model",
        &[FileExtension::Png, FileExtension::Mp4, FileExtension::Pdf],
    ));
    let files = TestFiles::new();
    let png = B64Bytes::new(vec![0x89, b'P', b'N', b'G']);
    let mp4 = B64Bytes::new(b"\0\0\0\x18ftypmp42".to_vec());
    let pdf = B64Bytes::new(b"%PDF-1.7".to_vec());
    let bytes = |data: &B64Bytes, media_type: &str| MediaBytes {
        data: data.clone(),
        media_type: media_type.into(),
    };
    let image = UserPart::Image(Medium::Bytes(bytes(&png, "image/png")));
    let video = UserPart::Video(Medium::Bytes(bytes(&mp4, "video/mp4")));
    let document = UserPart::Document {
        bytes: bytes(&pdf, "application/pdf"),
        file_name: "paper.pdf".into(),
    };
    let attachment = Attachment {
        name: "chart.png".into(),
        id: "a1".into(),
        media_type: "image/png".into(),
        size_bytes: 4,
        sha256: BlobRef::try_from("a".repeat(64)).unwrap(),
        snippet: None,
    };
    let record = UserContentBlock::Attachment(attachment.clone());
    // The upload route stored each file: the message's medium references
    // its blob, and its bytes come along for the session to hold.
    let png_blob = blobs.put(png.clone()).await.unwrap();
    let mp4_blob = blobs.put(mp4.clone()).await.unwrap();
    let pdf_blob = blobs.put(pdf.clone()).await.unwrap();
    let held = |blob: &BlobRef, bytes: &B64Bytes| {
        let mut held = HeldMedia::default();
        held.hold(blob.clone(), bytes.clone());
        held
    };
    let by_reference = |blob: &BlobRef, media_type: &str| MediaSource::Ref {
        r#ref: blob.clone(),
        media_type: media_type.into(),
        width: None,
        height: None,
    };
    let uploaded_image = UserContentBlock::Image {
        source: by_reference(&png_blob, "image/png"),
    };
    let uploaded_video = UserContentBlock::Video {
        source: by_reference(&mp4_blob, "video/mp4"),
    };
    let uploaded_document = UserContentBlock::Document {
        source: DocumentSource::Ref {
            r#ref: pdf_blob.clone(),
            media_type: "application/pdf".into(),
            file_name: "paper.pdf".into(),
        },
    };
    files.upload(
        "upload-1",
        (vec![uploaded_image, record.clone()], held(&png_blob, &png)),
    );
    files.upload("upload-2", (vec![uploaded_video], held(&mp4_blob, &mp4)));
    files.upload("upload-3", (vec![uploaded_document], held(&pdf_blob, &pdf)));
    // Another message's image: the caller's own blob, which this message
    // does not hold.
    let elsewhere = blobs.put(B64Bytes::new(b"GIF89a".to_vec())).await.unwrap();
    let mut client =
        TestClient::connect_with(&fixture.server, &conversation(), files);
    client.open().await;
    client.received();
    let upload = |reference: &str, file_name: &str| ClientContent::Upload {
        r#ref: reference.into(),
        file_name: file_name.into(),
    };
    client
        .send(ClientFrame::Send {
            comments: Vec::new(),
            message_id: TurnId::try_from("m1").unwrap(),
            content: vec![
                ClientContent::Text {
                    text: "look".into(),
                },
                upload("upload-1", "chart.png"),
                upload("upload-2", "clip.mp4"),
                upload("upload-3", "paper.pdf"),
            ],
        })
        .await;
    client.until_idle().await;
    let target = user_block(&fixture, "m1");
    let version = session_of(&fixture).transcript().version;
    let image_ref = |blob: &BlobRef| ClientContent::Media {
        media: MediaRef::Image {
            r#ref: blob.clone(),
            media_type: "image/png".into(),
        },
    };

    let mut refusals = Vec::new();
    for (operation, unknown) in [
        (
            "op1",
            ClientContent::Attachment {
                id: "a9".into(),
            },
        ),
        ("op2", image_ref(&elsewhere)),
        (
            "op3",
            ClientContent::Media {
                media: MediaRef::Video {
                    r#ref: png_blob.clone(),
                    media_type: "video/mp4".into(),
                },
            },
        ),
    ] {
        client
            .send(edit(operation, &target, &version, vec![unknown]))
            .await;
        refusals.push(edit_outcome(&client.received()));
    }
    assert_eq!(
        refusals,
        [
            rejected(AgentCode::EditFileMissing, "The edited message holds no attachment a9"),
            rejected(AgentCode::EditFileMissing, &format!("The edited message holds no image {elsewhere}")),
            rejected(AgentCode::EditFileMissing, &format!("The edited message holds no video {png_blob}")),
        ]
    );
    let kept = vec![
        ClientContent::Text {
            text: "look again".into(),
        },
        image_ref(&png_blob),
        ClientContent::Media {
            media: MediaRef::Video {
                r#ref: mp4_blob.clone(),
                media_type: "video/mp4".into(),
            },
        },
        // The block keeps what the message holds, whatever else the
        // reference says.
        ClientContent::Media {
            media: MediaRef::Document {
                r#ref: pdf_blob.clone(),
                media_type: "text/plain".into(),
                file_name: "renamed.txt".into(),
            },
        },
        ClientContent::Attachment {
            id: "a1".into(),
        },
    ];
    client.send(edit("op4", &target, &version, kept)).await;
    let frames = client.until_idle().await;

    assert!(matches!(
        edit_outcome(&frames),
        EditOutcome::Accepted { .. }
    ));
    // The model reads the bytes of the message's own blocks; the store keeps
    // them by their blobs.
    let again = UserContentBlock::Text {
        text: "look again".into(),
    };
    assert_eq!(
        script.requests()[1].items.first(),
        Some(&InferenceItem::UserMessage {
            content: vec![
                UserPart::Text("look again".into()),
                image,
                video,
                document,
                UserPart::Text(attachment_tag(&attachment)),
            ]
        })
    );
    let stored = fixture.store.checkpoint(&conversation()).unwrap();
    let Block::User(replacement) = &stored.transcript[0] else {
        panic!("{:?}", stored.transcript)
    };
    assert_eq!(
        replacement.content,
        [
            again,
            UserContentBlock::Image {
                source: by_reference(&png_blob, "image/png"),
            },
            UserContentBlock::Video {
                source: by_reference(&mp4_blob, "video/mp4"),
            },
            UserContentBlock::Document {
                source: DocumentSource::Ref {
                    r#ref: pdf_blob,
                    media_type: "application/pdf".into(),
                    file_name: "paper.pdf".into(),
                },
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
                        content: demi_agent_store::testing::sent_text("A")
                    },
                    InferenceItem::AssistantText {
                        model_id: "test-model".into(),
                        text: "answer A".into()
                    },
                    InferenceItem::UserMessage {
                        content: demi_agent_store::testing::sent_text("U3")
                    },
                ]
            );
            vec![event::text("answer U3"), event::response(1, 1)]
        })),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    client.until_idle().await;
    client.send(send("m2", "B")).await;
    client.until_idle().await;
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
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;
    let cold = fixture
        .server
        .prepare_fork(&conversation(), &answer_a)
        .await
        .unwrap();
    let mut running = cold.clone();
    running.checkpoint.state.phase = SessionPhase::Running;
    let destination = NodeId::try_from("fork").unwrap();
    let invalid = fixture.server.initialize_fork(&destination, running).await;
    fixture
        .server
        .initialize_fork(&destination, live.clone())
        .await
        .unwrap();

    assert_eq!(live, cold);
    assert_eq!(live.checkpoint.transcript[..], blocks[..2]);
    assert_eq!(live.checkpoint.state.phase, SessionPhase::Idle);
    assert!(live.checkpoint.state.queue.is_empty() && live.checkpoint.state.edits.is_empty());
    assert_eq!(not_text, Err(ForkError::Target(CutError::NotCompletedText)));
    assert_eq!(invalid, Err(ForkError::InvalidSeed));
    let record = fixture.store.record(&destination).unwrap();
    assert_eq!((record.parent, record.closed), (None, None));

    // The destination opens idle with the prefix, and continues on its own.
    let mut forked = TestClient::connect(&fixture.server, &destination);
    forked.open().await;
    let handshake = forked.received();
    let Some(ServerFrame::TranscriptReset { blocks: seeded, .. }) = handshake.iter().find(|frame| matches!(frame, ServerFrame::TranscriptReset { node, .. } if node == &destination)) else {
        panic!("{handshake:#?}")
    };
    assert_eq!(seeded[..], blocks[..2]);
    forked
        .send(ClientFrame::Send {
            comments: Vec::new(),
            message_id: TurnId::try_from("m3").unwrap(),
            content: client_text("U3"),
        })
        .await;
    forked.until_idle().await;
    assert_eq!(script.remaining(), 0);
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
        TestProduct::default(),
        MemoryTreeStore::new(),
        ServerConfig::default(),
    );
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    client.until_idle().await;
    let target = user_block(&fixture, "m1");
    let root = conversation();
    let spawn = |prompt: &str| agent(&fixture.server, &root, "spawn", json!({ "prompt": prompt }));

    // A live child refuses the edit and keeps its record.
    let child = named_node(&fixture.store, &spawn("Read notes.md").await);
    let live = fixture.store.record(&child).unwrap();
    let version = session_of(&fixture).transcript().version;
    client
        .send(edit("op1", &target, &version, client_text("A2")))
        .await;
    assert_eq!(
        edit_outcome(&client.received()),
        rejected(AgentCode::FrameFailed, "Cannot edit while children or completion notifications are pending")
    );
    assert_eq!(fixture.store.record(&child).unwrap(), live);
    reading.open();
    client.until_idle().await;
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
    let frames = client.until_idle().await;
    let EditOutcome::Accepted { turn_id } = edit_outcome(&frames) else {
        panic!("{frames:#?}")
    };
    assert_eq!(fixture.store.record(&child).unwrap(), delivered);

    // While a child start takes its number, an edit is refused.
    let target = user_block(&fixture, turn_id.as_str());
    let version = session_of(&fixture).transcript().version;
    let gate = fixture.store.hold_numbers();
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
            rejected(AgentCode::FrameFailed, "Cannot edit while a child lifecycle operation is in progress")
        );
        gate.release();
        assert_eq!(starting.await.code, 0);
    }
    client.until_idle().await;
    assert!(model.is_done());
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_completion_whose_saves_failed_refuses_an_edit_until_a_later_save_delivers_it() {
    let model = Model::default();
    model.root([said("answer A"), said("answer B"), said("answer A2")]);
    let fixture = Fixture::with_model(
        &model,
        TestProduct::default(),
        MemoryTreeStore::new(),
        ServerConfig::default(),
    );
    let store = &fixture.store;
    let mut client = fixture.opened().await;
    client.send(send("m1", "A")).await;
    client.until_idle().await;
    client.send(ClientFrame::Close {}).await;
    // A child of the root closed before the root saved its completion, as a
    // process that died between the two leaves them.
    let child = NodeId::try_from("child").unwrap();
    store
        .create_node(
            child_record("child", 1, "conversation", None),
            checkpoint(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    let close = NodeClose {
        phase: ClosePhase::Completed,
        at: Timestamp::UNIX_EPOCH,
    };
    store.close_node(&child, close).await.unwrap();

    // The database refuses every save while the tree opens again: the save
    // of the completion the opening delivers fails, and so does the turn
    // the completion opens. The root ends idle, the completion undelivered.
    store.fail_saves(usize::MAX);
    let mut client = fixture.client();
    client.open().await;
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
        rejected(AgentCode::FrameFailed, "Cannot edit while children or completion notifications are pending")
    );
    assert_eq!(store.record(&child).unwrap(), undelivered);

    // The next save carries the completion; then the edit is accepted.
    client.send(send("m2", "B")).await;
    client.until_idle().await;
    assert!(store.record(&child).unwrap().delivered);
    let version = session_of(&fixture).transcript().version;
    client
        .send(edit("op2", &target, &version, client_text("A2")))
        .await;
    let frames = client.until_idle().await;
    assert!(matches!(
        edit_outcome(&frames),
        EditOutcome::Accepted { .. }
    ));
    assert!(model.is_done());
}

/// Two comments on the reply `reply`, one quoting two lines.
fn two_comments(reply: &BlockId) -> Vec<MessageComment> {
    let comment = |id: &str, quote: &str, text: &str| MessageComment {
        id: id.into(),
        block_id: reply.clone(),
        start: 0,
        at: 0,
        end: quote.chars().count() as u64,
        quote: quote.into(),
        picture: None,
        text: text.into(),
    };
    vec![
        comment("c1", "Upload in chunks\nof 4 MB.", "4 MB feels large on 2G."),
        comment("c2", "the same 30-second timeout", "Should uploads get their own?"),
    ]
}

/// The model's text for [`two_comments`]: each quote, then its comment.
const TWO_COMMENTS_READ: &str = "> Upload in chunks\n> of 4 MB.\n\n4 MB feels large on 2G.\n\n\
    > the same 30-second timeout\n\nShould uploads get their own?";

/// A steer carries its comments as a message does: the page's pending steer
/// shows them, the model reads them as quotes before the steer's text, and its
/// `steer` block keeps them.
#[tokio::test(flavor = "local")]
async fn a_steers_comments_reach_the_model_as_quotes_before_its_text() {
    let gate = Gate::new();
    let script = ScriptedRuntime::new([
        said("Uploads fail because"),
        held(&gate, vec![event::text("Checking the logs"), event::response(1, 1)]),
        said("Starting at 1 MB"),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "Why do uploads fail?")).await;
    client.until_idle().await;
    let reply = session_of(&fixture).transcript().blocks[1].id().clone();
    let comments = two_comments(&reply);
    client.send(send("m2", "Check the logs")).await;
    until(|| script.requests().len() == 2).await;
    let steer_id = BlockId::try_from("s1").unwrap();
    client
        .send(ClientFrame::Steer {
            steer_id: steer_id.clone(),
            content: client_text("Otherwise go ahead"),
            comments: comments.clone(),
        })
        .await;
    let answered = client
        .next_until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
        .await;
    let pending: Vec<_> = answered
        .iter()
        .filter_map(|frame| demi_agent_server::testing::root_entry(frame))
        .filter(|entry| !entry.pending_steers.is_empty())
        .last()
        .into_iter()
        .flat_map(|entry| entry.pending_steers.clone())
        .map(|steer| steer.comments)
        .collect();
    assert_eq!(pending, [comments.clone()]);
    gate.open();
    client.until_idle().await;

    let requests = script.requests();
    assert_eq!(
        requests[2].items.last(),
        Some(&InferenceItem::UserSteer {
            content: vec![UserPart::Text(TWO_COMMENTS_READ.into()), UserPart::Text("Otherwise go ahead".into())]
        })
    );
    let held = session_of(&fixture).transcript();
    let Some(Block::Steer(steer)) = held.blocks.iter().find(|block| block.id() == &steer_id) else {
        panic!("{:?}", held.blocks)
    };
    assert_eq!(steer.comments, comments);
}

/// A comment on a picture of the reply, sent with a text comment, as the model
/// reads it.
///
/// The model reads, in order:
///
/// 1. the text comment's quote and comment;
/// 2. the picture as an image the message carries, read from its blob;
/// 3. its comment;
/// 4. the message's own text.
///
/// A model that reads no images gets a note naming the picture in its place.
#[tokio::test(flavor = "local")]
async fn a_picture_comment_reaches_the_model_as_the_image_then_its_comment() {
    for reads_images in [true, false] {
        let script = ScriptedRuntime::new([said("Uploads fail because"), said("Starting at 1 MB")]);
        let blobs = MemoryBlobs::new();
        let store = MemoryTreeStore::with_blobs(blobs.clone());
        let fixture = Fixture::with(&script, store, ServerConfig::default());
        let reads: &[FileExtension] = if reads_images { &[FileExtension::Png] } else { &[] };
        fixture.resolver.select(model_reading("stub", "test-model", reads));
        let png = B64Bytes::new(vec![0x89, b'P', b'N', b'G']);
        let blob = blobs.put(png.clone()).await.unwrap();
        let mut client = fixture.opened().await;
        client.send(send("m1", "Why do uploads fail?")).await;
        client.until_idle().await;
        let reply = session_of(&fixture).transcript().blocks[1].id().clone();
        let [_, timeout] = two_comments(&reply).try_into().unwrap();
        let picture = MessageComment {
            id: "c3".into(),
            block_id: reply.clone(),
            start: 0,
            at: 120,
            end: 1,
            quote: "The upload cut off".into(),
            picture: Some(MediaSource::Ref {
                r#ref: blob.clone(),
                media_type: "image/png".into(),
                width: None,
                height: None,
            }),
            text: "Is this the 3G test?".into(),
        };
        client
            .send(ClientFrame::Send {
                message_id: crate::support::turn("m2"),
                content: client_text("Otherwise go ahead"),
                comments: vec![timeout, picture],
            })
            .await;
        client.until_idle().await;

        let requests = script.requests();
        let Some(InferenceItem::UserMessage { content }) = requests[1].items.get(2) else {
            panic!("{:#?}", requests[1].items)
        };
        assert_eq!(
            content[0],
            UserPart::Text("> the same 30-second timeout\n\nShould uploads get their own?".into())
        );
        if reads_images {
            assert_eq!(
                content[1],
                UserPart::Image(Medium::Bytes(MediaBytes { data: png, media_type: "image/png".into() }))
            );
        } else {
            assert!(
                matches!(&content[1], UserPart::Text(note) if note.starts_with("[image:") && note.contains("not sent")),
                "{content:#?}"
            );
        }
        assert_eq!(
            content[2..],
            [UserPart::Text("Is this the 3G test?".into()), UserPart::Text("Otherwise go ahead".into())]
        );
    }
}

/// Fork copies a message's comments with it: the fork's model reads them as the
/// source's did.
#[tokio::test(flavor = "local")]
async fn a_fork_keeps_a_messages_comments_and_its_model_reads_them() {
    let script = ScriptedRuntime::new([said("Uploads fail because"), said("Starting at 1 MB"), said("Done")]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "Why do uploads fail?")).await;
    client.until_idle().await;
    let reply = session_of(&fixture).transcript().blocks[1].id().clone();
    let comments = two_comments(&reply);
    client
        .send(ClientFrame::Send {
            message_id: crate::support::turn("m2"),
            content: client_text("Otherwise go ahead"),
            comments: comments.clone(),
        })
        .await;
    client.until_idle().await;
    let answer = session_of(&fixture).transcript().blocks[4].id().clone();

    let seed = fixture
        .server
        .prepare_fork(&conversation(), &answer)
        .await
        .unwrap();
    let destination = NodeId::try_from("fork").unwrap();
    fixture
        .server
        .initialize_fork(&destination, seed)
        .await
        .unwrap();
    let mut forked = TestClient::connect(&fixture.server, &destination);
    forked.open().await;
    forked.received();
    forked.send(send("m3", "And the retries?")).await;
    until(|| script.requests().len() == 3).await;

    let requests = script.requests();
    let Some(InferenceItem::UserMessage { content }) = requests[2].items.get(2) else {
        panic!("{:#?}", requests[2].items)
    };
    assert_eq!(
        content[..],
        [UserPart::Text(TWO_COMMENTS_READ.into()), UserPart::Text("Otherwise go ahead".into())]
    );
}

/// A message's comments: the model reads each quoted text as a `>` quote with
/// its comment after it, then the typed text.
///
/// The `user` block keeps them apart from its content, and an edit of the
/// message changes its text and keeps them.
#[tokio::test(flavor = "local")]
async fn a_messages_comments_reach_the_model_as_quotes_before_its_text_and_an_edit_keeps_them() {
    let script = ScriptedRuntime::new([said("Uploads fail because"), said("Starting smaller"), said("Starting at 1 MB")]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "Why do uploads fail?")).await;
    client.until_idle().await;
    let reply = session_of(&fixture).transcript().blocks[1].id().clone();
    let comments = two_comments(&reply);
    client
        .send(ClientFrame::Send {
            message_id: crate::support::turn("m2"),
            content: client_text("Otherwise go ahead"),
            comments: comments.clone(),
        })
        .await;
    client.until_idle().await;

    let quoted = TWO_COMMENTS_READ;
    let read = |request: &demi_provider_common::InferenceRequest| match request.items.get(2) {
        Some(InferenceItem::UserMessage { content }) => content.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        read(&script.requests()[1]),
        [UserPart::Text(quoted.into()), UserPart::Text("Otherwise go ahead".into())]
    );
    let target = user_block(&fixture, "m2");
    let held = session_of(&fixture).transcript();
    let Some(Block::User(sent)) = held.blocks.iter().find(|block| block.id() == &target) else {
        panic!("{:?}", held.blocks)
    };
    assert_eq!((sent.content.clone(), sent.comments.clone()), (demi_agent_store::testing::text("Otherwise go ahead"), comments.clone()));

    client
        .send(edit("op1", &target, &held.version, client_text("Start at 1 MB")))
        .await;
    client.until_idle().await;
    let edited = session_of(&fixture).transcript();
    let Some(Block::User(replacement)) = edited.blocks.get(3) else {
        panic!("{:?}", edited.blocks)
    };
    assert_eq!((replacement.content.clone(), replacement.comments.clone()), (demi_agent_store::testing::text("Start at 1 MB"), comments));
    assert_eq!(
        read(script.requests().last().unwrap()),
        [UserPart::Text(quoted.into()), UserPart::Text("Start at 1 MB".into())]
    );
}
