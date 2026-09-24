//! Turns over the conversation socket: a message runs to its response and
//! its patches rebuild the transcript; a failure, a stop and a close in the
//! middle of a turn; a reopen from the store after a close and after a
//! crash; model switches; the harness's texts.

use std::{rc::Rc, time::Duration};

use demi_agent::{
    ServerConfig, attachments,
    store::media::{BlobStore, externalize_frame},
    testing::{MemoryBlobs, MemoryTreeStore, TestClient, TestFiles, model_of, test_model},
};
use demi_agent_protocol::{
    AbortResult, AbortTarget, ClientContent, ClientFrame, ModelSwitchApply, ServerFrame,
    TranscriptPatch,
};
use demi_core::{B64Bytes, Block, FailureSource, MediaSource, SessionPhase, UserContentBlock};
use demi_provider::{
    ErrorCode, InferenceItem, ProviderEvent,
    testing::{ScriptedRuntime, Turn, event},
};
use serde_json::{Value, json};

use crate::support::{
    Fixture, Gate, conversation, held, is_idle, is_pending_steers, kinds, open, send, until,
};

#[tokio::test(flavor = "local")]
async fn a_message_runs_to_its_response_and_its_patches_rebuild_the_transcript() {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![
            ProviderEvent::ThinkingStart,
            event::thinking("Let me look."),
            ProviderEvent::ThinkingSignature("anthropic:sig-1".into()),
            event::text("Checking "),
            event::text("the files."),
            event::tool_call("call-1", "shell_exec", json!({ "script": "ls" })),
            event::response(12, 8),
        ]),
        Turn::Events(vec![
            event::text("There are two files."),
            event::response(30, 6),
        ]),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.client();
    client.send(open(test_model())).await;
    let handshake = client.next_until(is_pending_steers).await;
    client.send(send("m1", "List the files")).await;
    let turn = client.next_until(is_idle).await;

    let reset = handshake
        .into_iter()
        .find(|frame| matches!(frame, ServerFrame::TranscriptReset { .. }))
        .unwrap();
    let patches: Vec<ServerFrame> = turn
        .into_iter()
        .filter(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. }))
        .collect();
    let live = fixture
        .server
        .tree(&conversation())
        .unwrap()
        .root()
        .session()
        .transcript();
    assert_eq!(
        kinds(&live.blocks),
        [
            "user",
            "thinking",
            "text",
            "tool_call:error",
            "response",
            "text",
            "response"
        ]
    );
    // The patches, applied from the reset by `agent-client`'s one applier,
    // must give the live transcript: the fixture pins both.
    let recorded = json!({
        "name": "a turn with signed thinking, streamed text and a tool call",
        "reset": reset,
        "patches": patches,
        "transcript": live.blocks,
    });
    let fixture_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/agent/fixtures/transcript-patches.json"
    );
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/transcript-patches.json")).unwrap();
    if expected[0] != recorded {
        let actual = std::env::temp_dir().join("transcript-patches.actual.json");
        std::fs::write(
            &actual,
            serde_json::to_string_pretty(&json!([recorded])).unwrap(),
        )
        .unwrap();
        panic!(
            "the patches differ from {fixture_path}; this run's are in {}",
            actual.display()
        );
    }

    // The root is a node without a parent, open, beside its checkpoint.
    let record = fixture.store.record(&conversation()).unwrap();
    assert_eq!((record.parent, record.closed), (None, None));

    // Live equals cold: after a close, the stored tree reopens as it was.
    client.send(ClientFrame::Close {}).await;
    assert_eq!(
        client
            .next_until(|frame| *frame == ServerFrame::Closed)
            .await
            .last(),
        Some(&ServerFrame::Closed)
    );
    assert!(fixture.server.tree(&conversation()).is_none());
    assert_eq!(script.closes(), 1);
    let mut reopened = fixture.client();
    reopened.send(open(test_model())).await;
    let handshake = reopened.next_until(is_pending_steers).await;
    let ServerFrame::TranscriptReset {
        blocks, version, ..
    } = &handshake[1]
    else {
        panic!("{handshake:?}");
    };
    assert_eq!(*blocks, live.blocks);
    assert_ne!(version.epoch, live.version.epoch);
}

#[tokio::test(flavor = "local")]
async fn a_provider_failure_is_reported_once_and_recorded_with_its_diagnostics() {
    let script = ScriptedRuntime::new([Turn::Events(vec![event::error(
        "auth failed",
        Some(ErrorCode::AuthExpired),
    )])]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;

    client.send(send("m1", "hi")).await;
    let frames = client.next_until(is_idle).await;

    let errors: Vec<&ServerFrame> = frames
        .iter()
        .filter(|frame| matches!(frame, ServerFrame::Error { .. }))
        .collect();
    assert_eq!(errors.len(), 1, "{frames:?}");
    let ServerFrame::Error {
        message,
        code,
        diagnostics,
    } = errors[0]
    else {
        unreachable!()
    };
    assert_eq!(
        (message.as_str(), code.as_deref()),
        ("auth failed", Some("auth_expired"))
    );
    let diagnostics = diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics.source, FailureSource::Unknown);
    assert_eq!(
        diagnostics.client_request_id.as_deref(),
        Some(script.requests()[0].request_id.as_str())
    );
    let checkpoint = fixture.store.checkpoint(&conversation()).unwrap();
    assert_eq!(kinds(&checkpoint.transcript), ["user", "error"]);
    let Block::Error(record) = &checkpoint.transcript[1] else {
        unreachable!()
    };
    assert_eq!(record.code.as_deref(), Some("auth_expired"));
    assert_eq!(checkpoint.state.phase, SessionPhase::Idle);
}

#[tokio::test(flavor = "local")]
async fn an_uploaded_image_reaches_the_model_inline_and_travels_and_rests_by_reference() {
    const PNG: [u8; 12] = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01,
    ];
    let image = UserContentBlock::Image {
        source: MediaSource::Binary {
            data: B64Bytes::new(PNG.to_vec()),
            media_type: "image/png".into(),
        },
    };
    let seen = |expected: Vec<UserContentBlock>, answer: &'static str| {
        Turn::Respond(Box::new(move |request| {
            let first = request.items.first().cloned();
            assert_eq!(
                first,
                Some(InferenceItem::UserMessage { content: expected })
            );
            vec![event::text(answer), event::response(1, 1)]
        }))
    };
    let blobs = MemoryBlobs::new();
    let uploaded = blobs.put(B64Bytes::new(PNG.to_vec())).await.unwrap();
    let path = "/home/demi/.demi/attachments/conversation/tiny.png";
    let blocks = attachments::upload_blocks(attachments::Upload {
        name: "tiny.png",
        path,
        media_type: "image/png",
        sha256: &uploaded,
        bytes: &PNG,
    });
    let record = blocks[1].clone();
    let text = UserContentBlock::Text {
        text: "describe this".into(),
    };
    let script = ScriptedRuntime::new([
        seen(
            vec![text.clone(), image.clone(), record.clone()],
            "a tiny png",
        ),
        seen(
            vec![text.clone(), image.clone(), record.clone()],
            "still a png",
        ),
        seen(
            vec![
                text.clone(),
                UserContentBlock::Text {
                    text: format!("[missing image blob {uploaded}]"),
                },
                record.clone(),
            ],
            "the image is gone",
        ),
    ]);
    let store = MemoryTreeStore::with_blobs(blobs.clone());
    let fixture = Fixture::with(&script, store, ServerConfig::default());
    let files = TestFiles::new();
    files.upload("upload-1", blocks);
    let mut client =
        TestClient::connect_with(&fixture.server, &conversation(), "/workspace", files);
    client.send(open(test_model())).await;
    client.next_until(is_pending_steers).await;
    client
        .send(ClientFrame::Send {
            message_id: crate::support::turn("m1"),
            content: vec![
                ClientContent::Text {
                    text: "describe this".into(),
                },
                ClientContent::Upload {
                    r#ref: "upload-1".into(),
                    file_name: "tiny.png".into(),
                },
            ],
        })
        .await;
    let frames = client.next_until(is_idle).await;

    // At rest and on the wire, the image is its reference.
    let by_reference = UserContentBlock::Image {
        source: MediaSource::Ref {
            r#ref: uploaded.clone(),
            media_type: "image/png".into(),
        },
    };
    let stored = fixture.store.checkpoint(&conversation()).unwrap();
    let Block::User(user) = &stored.transcript[0] else {
        panic!("{:?}", stored.transcript)
    };
    assert_eq!(
        user.content,
        [text.clone(), by_reference.clone(), record.clone()]
    );
    let mut added = frames
        .into_iter()
        .find(|frame| matches!(frame, ServerFrame::TranscriptPatch { patches, .. }
            if patches.iter().any(|patch| matches!(patch, TranscriptPatch::Add { value: Block::User(_), .. }))))
        .unwrap();
    externalize_frame(&mut added, &*blobs).await.unwrap();
    let ServerFrame::TranscriptPatch { patches, .. } = &added else {
        unreachable!()
    };
    let Some(TranscriptPatch::Add {
        value: Block::User(sent),
        ..
    }) = patches.first()
    else {
        panic!("{patches:?}")
    };
    assert_eq!(sent.content, [text.clone(), by_reference, record.clone()]);

    // Loaded again, the model reads the bytes; a blob that is gone is named.
    for message in ["m2", "m3"] {
        client.send(ClientFrame::Close {}).await;
        client
            .next_until(|frame| *frame == ServerFrame::Closed)
            .await;
        if message == "m3" {
            blobs.forget(&uploaded);
        }
        client.send(open(test_model())).await;
        client.next_until(is_pending_steers).await;
        client.send(send(message, "and now?")).await;
        client.next_until(is_idle).await;
    }
    assert_eq!(script.remaining(), 0);
}

#[tokio::test(flavor = "local")]
async fn stop_during_a_stream_answers_after_the_stopped_marker_and_the_queued_message_runs_next() {
    let script = ScriptedRuntime::new([
        Turn::pending(),
        Turn::Events(vec![event::text("second answer"), event::response(1, 1)]),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "first")).await;
    client.send(send("m2", "second")).await;
    until(|| script.requests().len() == 1).await;

    client.send(ClientFrame::Abort {}).await;
    let frames = client
        .next_until(|frame| matches!(frame, ServerFrame::AbortResult { .. }))
        .await;

    // The stopped marker is in the transcript before the answer.
    let marker = frames.iter().position(|frame| {
        matches!(frame, ServerFrame::TranscriptPatch { patches, .. }
            if serde_json::to_string(patches).unwrap().contains("\"type\":\"abort\""))
    });
    assert!(marker.is_some(), "{frames:?}");
    assert_eq!(
        frames.last(),
        Some(&ServerFrame::AbortResult {
            result: AbortResult {
                target: Some(AbortTarget::ActiveProviderStream),
                can_abort_again: true,
            },
        })
    );
    let tree = fixture.server.tree(&conversation()).unwrap();
    tree.root().session().settled().await;
    assert_eq!(
        kinds(&tree.root().session().transcript().blocks),
        ["user", "abort", "user", "text", "response"]
    );
}

#[tokio::test(flavor = "local")]
async fn an_interrupt_stops_the_running_turn_and_holds_the_queued_message_until_it_is_let_go() {
    let script = ScriptedRuntime::new([
        Turn::pending(),
        Turn::Events(vec![event::text("second answer"), event::response(1, 1)]),
    ]);
    let fixture = Fixture::new(&script);
    let client = fixture.opened().await;
    client.send(send("m1", "first")).await;
    client.send(send("m2", "second")).await;
    until(|| script.requests().len() == 1).await;

    let tree = fixture.server.tree(&conversation()).unwrap();
    let held = tree.interrupt().await;
    // The running turn stopped as a Stop stops it; the queued message waits
    // while the tree is held, and runs once it is let go.
    assert_eq!(
        kinds(&tree.root().session().transcript().blocks),
        ["user", "abort"]
    );
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    assert_eq!(script.requests().len(), 1);
    drop(held);
    until(|| script.requests().len() == 2).await;
    tree.root().session().settled().await;
    assert_eq!(
        kinds(&tree.root().session().transcript().blocks),
        ["user", "abort", "user", "text", "response"]
    );
}

#[tokio::test(flavor = "local")]
async fn close_during_a_turn_saves_the_interruption_and_the_queue_and_a_reopen_runs_the_queue() {
    let script = ScriptedRuntime::new([
        Turn::pending(),
        Turn::Events(vec![event::text("later"), event::response(1, 1)]),
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "first")).await;
    client.send(send("m2", "second")).await;
    until(|| script.requests().len() == 1).await;

    client.send(ClientFrame::Close {}).await;
    assert_eq!(
        client
            .next_until(|frame| *frame == ServerFrame::Closed)
            .await
            .last(),
        Some(&ServerFrame::Closed)
    );

    let checkpoint = fixture.store.checkpoint(&conversation()).unwrap();
    assert_eq!(checkpoint.state.phase, SessionPhase::Running);
    assert_eq!(kinds(&checkpoint.transcript), ["user", "error"]);
    let queue: Vec<&str> = checkpoint
        .state
        .queue
        .iter()
        .map(|message| message.id.as_str())
        .collect();
    assert_eq!(queue, ["m2"]);
    assert_eq!(script.closes(), 1);

    let mut reopened = fixture.client();
    reopened.send(open(test_model())).await;
    let frames = reopened.next_until(is_idle).await;
    let ServerFrame::TranscriptReset { blocks, .. } = &frames[1] else {
        panic!("{frames:?}");
    };
    assert_eq!(kinds(blocks), ["user", "error"]);
    let tree = fixture.server.tree(&conversation()).unwrap();
    tree.root().session().settled().await;
    // The interruption was recorded once; the queued message ran.
    assert_eq!(
        kinds(&tree.root().session().transcript().blocks),
        ["user", "error", "user", "text", "response"]
    );
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_crash_during_a_turn_is_recorded_as_an_interruption_when_the_conversation_reopens() {
    let script = ScriptedRuntime::new([Turn::pending()]);
    let fixture = Fixture::new(&script);
    let client = fixture.opened().await;
    client.send(send("m1", "first")).await;
    // The throttled save writes the user block with the turn running.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let crashed = fixture.store.copy();
    assert_eq!(
        crashed.checkpoint(&conversation()).unwrap().state.phase,
        SessionPhase::Running
    );

    let after = ScriptedRuntime::new(Vec::new());
    let restarted = Fixture::with(&after, crashed.clone(), ServerConfig::default());
    let mut reopened = restarted.client();
    reopened.send(open(test_model())).await;
    let frames = reopened.received();

    assert_eq!(frames[0], ServerFrame::Opened);
    let ServerFrame::TranscriptReset { blocks, .. } = &frames[1] else {
        panic!("{frames:?}");
    };
    assert_eq!(kinds(blocks), ["user"]);
    // The record follows the handshake, and it is saved at once.
    assert!(
        frames[5..]
            .iter()
            .any(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. }))
    );
    let checkpoint = crashed.checkpoint(&conversation()).unwrap();
    assert_eq!(kinds(&checkpoint.transcript), ["user", "error"]);
    assert_eq!(checkpoint.state.phase, SessionPhase::Idle);
}

#[tokio::test(flavor = "local")]
async fn an_immediate_switch_lands_inside_the_running_turn_and_a_next_turn_switch_waits() {
    for (apply, expected) in [
        (
            Some(ModelSwitchApply::Immediate),
            ["test-model", "model-b", "model-b"],
        ),
        (None, ["test-model", "test-model", "model-b"]),
    ] {
        let release = Gate::new();
        let first = held(
            &release,
            vec![
                event::tool_call("call-1", "shell_exec", json!({})),
                event::response(1, 1),
            ],
        );
        let script = ScriptedRuntime::new([
            first,
            Turn::Events(vec![event::text("one"), event::response(1, 1)]),
            Turn::Events(vec![event::text("two"), event::response(1, 1)]),
        ]);
        let fixture = Fixture::new(&script);
        let mut client = fixture.opened().await;
        client.send(send("m1", "first")).await;
        until(|| script.requests().len() == 1).await;
        client
            .send(ClientFrame::SetProvider {
                model: model_of("stub", "model-b"),
                apply,
            })
            .await;
        release.open();
        client.next_until(is_idle).await;
        client.send(send("m2", "second")).await;
        client.next_until(is_idle).await;

        let models: Vec<String> = script
            .requests()
            .iter()
            .map(|request| request.model_id.clone())
            .collect();
        assert_eq!(models, expected, "{apply:?}");
        // The same provider serves both models: nothing was resolved again.
        assert_eq!(fixture.resolver.calls.borrow().len(), 1);
    }
}

#[tokio::test(flavor = "local")]
async fn a_switch_to_another_provider_builds_its_runtime_and_closes_the_old_one_at_the_next_turn() {
    let stub = ScriptedRuntime::new([Turn::Events(vec![
        event::text("from stub"),
        event::response(1, 1),
    ])]);
    let other = ScriptedRuntime::new([Turn::Events(vec![
        event::text("from other"),
        event::response(1, 1),
    ])]);
    let fixture = Fixture::new(&stub);
    fixture.resolver.provide("other", &other);
    let mut client = fixture.opened().await;
    client.send(send("m1", "first")).await;
    client.next_until(is_idle).await;

    client
        .send(ClientFrame::SetProvider {
            model: model_of("other", "other-model"),
            apply: None,
        })
        .await;
    client.send(send("m2", "second")).await;
    client.next_until(is_idle).await;

    let calls: Vec<String> = fixture
        .resolver
        .calls
        .borrow()
        .iter()
        .map(|(root, provider)| format!("{root} {provider}"))
        .collect();
    assert_eq!(calls, ["conversation stub", "conversation other"]);
    assert_eq!((stub.requests().len(), stub.closes()), (1, 1));
    assert_eq!(other.requests()[0].model_id, "other-model");
    let checkpoint = fixture.store.checkpoint(&conversation()).unwrap();
    assert_eq!(checkpoint.state.model.provider_id, "other");
}

#[tokio::test(flavor = "local")]
async fn the_system_prompt_has_the_command_help_and_a_context_change_is_saved_before_the_request() {
    let store = MemoryTreeStore::new();
    let stored_at_request = Rc::new(std::cell::RefCell::new(Vec::new()));
    let script = ScriptedRuntime::new([Turn::Respond(Box::new({
        let store = store.clone();
        let stored_at_request = stored_at_request.clone();
        move |request| {
            *stored_at_request.borrow_mut() =
                kinds(&store.checkpoint(&conversation()).unwrap().transcript);
            let texts: Vec<String> = request
                .items
                .iter()
                .map(|item| match item {
                    InferenceItem::UserMessage { content } => {
                        serde_json::to_string(content).unwrap()
                    }
                    other => format!("{other:?}"),
                })
                .collect();
            vec![event::text(&texts.join(" | ")), event::response(1, 1)]
        }
    })), Turn::Events(vec![event::text("again"), event::response(1, 1)])]);
    let fixture = Fixture::with(&script, store, ServerConfig::default());
    fixture
        .harness
        .context
        .borrow_mut()
        .push_back("The conversation now runs on the Cloud.".into());
    let mut client = fixture.opened().await;

    client.send(send("m1", "hi")).await;
    client.next_until(is_idle).await;

    assert_eq!(*stored_at_request.borrow(), ["user", "context"]);
    let request = &script.requests()[0];
    assert_eq!(request.system_prompt, "system prompt");
    assert_eq!(request.items.len(), 2);
    let prompts = fixture.harness.prompts.borrow();
    assert!(
        prompts[0].contains("greet: Greets the caller."),
        "{prompts:?}"
    );
    assert!(prompts[0].contains("greet hello"), "{prompts:?}");
    drop(prompts);
    let blocks = fixture
        .server
        .tree(&conversation())
        .unwrap()
        .root()
        .session()
        .transcript()
        .blocks;
    assert_eq!(kinds(&blocks), ["user", "context", "text", "response"]);

    // The next request's hook is shown what the node saw.
    client.send(send("m2", "again")).await;
    client.next_until(is_idle).await;
    let seen = fixture.harness.seen.borrow();
    assert_eq!(*seen, [vec![], vec!["The conversation now runs on the Cloud.".to_owned()]]);
}

#[tokio::test(flavor = "local")]
async fn messages_sent_during_a_turn_wait_in_the_queue_which_the_client_reorders_and_empties() {
    let (release, release_third) = (Gate::new(), Gate::new());
    let first = held(&release, vec![event::text("one"), event::response(1, 1)]);
    let third = held(
        &release_third,
        vec![event::text("two"), event::response(1, 1)],
    );
    let script = ScriptedRuntime::new([
        first,
        Turn::Events(vec![event::text("three"), event::response(1, 1)]),
        third,
    ]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "one")).await;
    for (id, text) in [
        ("m2", "two"),
        ("m3", "three"),
        ("m4", "four"),
        ("m5", "five"),
    ] {
        client.send(send(id, text)).await;
    }
    let queue_ids = |frame: &ServerFrame| match frame {
        ServerFrame::Queue { queue } => Some(
            queue
                .iter()
                .map(|message| message.id.to_string())
                .collect::<Vec<_>>(),
        ),
        _ => None,
    };

    client
        .send(ClientFrame::SendQueuedMessage {
            message_id: crate::support::turn("m3"),
        })
        .await;
    client
        .send(ClientFrame::DequeueMessage {
            message_id: crate::support::turn("m4"),
        })
        .await;
    let queues: Vec<Vec<String>> = client.received().iter().filter_map(queue_ids).collect();
    assert_eq!(
        queues,
        [
            vec!["m2"],
            vec!["m2", "m3"],
            vec!["m2", "m3", "m4"],
            vec!["m2", "m3", "m4", "m5"],
            vec!["m3", "m2", "m4", "m5"],
            vec!["m3", "m2", "m5"],
        ]
    );
    // m1 and m3 run; while m2 runs, the queue that is left is cleared.
    release.open();
    until(|| script.requests().len() == 3).await;
    client.send(ClientFrame::ClearMessageQueue {}).await;
    release_third.open();
    let tree = fixture.server.tree(&conversation()).unwrap();
    tree.root().session().settled().await;
    let users: Vec<String> = tree
        .root()
        .session()
        .transcript()
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::User(user) => Some(user.turn_id.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(users, ["m1", "m3", "m2"]);
    assert!(tree.root().session().queued_messages().is_empty());
}
