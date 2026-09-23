//! Connections to a conversation's live tree: the open handshake, takeover
//! by a second connection, two opens at once, the refusals of frames that
//! need a session, a client that stops reading, and the eviction of a tree
//! left detached and quiescent.

use std::time::Duration;

use demi_agent::{
    Outgoing, ServerConfig,
    testing::{MemoryTreeStore, model_of, test_model},
};
use demi_agent_protocol::{ClientFrame, ClientFrameKind, ServerFrame, SteerOutcome};
use demi_core::{BlockId, NodeId, SessionPhase};
use demi_provider::testing::{ScriptedRuntime, Turn, event};

use crate::support::{
    Fixture, conversation, frame_type, is_idle, is_pending_steers, kinds, open, send, turn, until,
};

fn rejected(command: ClientFrameKind, reason: &str) -> ServerFrame {
    ServerFrame::Rejected {
        command,
        reason: reason.to_owned(),
    }
}

#[tokio::test(flavor = "local")]
async fn the_open_handshake_is_one_step_and_each_patch_is_one_revision_past_the_last() {
    let script = ScriptedRuntime::new([Turn::Events(vec![
        event::text("a"),
        event::text("b"),
        event::response(1, 1),
    ])]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.client();

    client.send(open(test_model())).await;
    let handshake = client.received();
    client.send(send("m1", "hi")).await;
    let turn = client.next_until(is_idle).await;

    let types: Vec<String> = handshake.iter().map(frame_type).collect();
    assert_eq!(
        types,
        [
            "opened",
            "transcript_reset",
            "phase",
            "queue",
            "pending_steers"
        ]
    );
    let ServerFrame::TranscriptReset { version, .. } = &handshake[1] else {
        unreachable!()
    };
    let mut revision = version.revision;
    for frame in &turn {
        if let ServerFrame::TranscriptPatch { revision: next, .. } = frame {
            assert_eq!(*next, revision + 1);
            revision = *next;
        }
    }
    assert!(revision > version.revision);
}

#[tokio::test(flavor = "local")]
async fn a_second_connection_takes_the_tree_over_and_the_first_can_take_it_back() {
    let script = ScriptedRuntime::new([Turn::Events(vec![
        event::text("hello"),
        event::response(1, 1),
    ])]);
    let fixture = Fixture::new(&script);
    let mut first = fixture.opened().await;
    first.send(send("m1", "hi")).await;
    first.next_until(is_idle).await;

    let mut second = fixture.opened().await;

    assert_eq!(first.next().await, Some(ServerFrame::Closed));
    first.send(send("m2", "again")).await;
    assert_eq!(
        first.next().await,
        Some(rejected(ClientFrameKind::Send, "No session is open"))
    );
    // One tree: the second connection adopted it, nothing was resolved again.
    assert_eq!(fixture.resolver.calls.borrow().len(), 1);
    let tree = fixture.server.tree(&conversation()).unwrap();
    assert_eq!(tree.root().session().transcript().blocks.len(), 3);

    first.send(open(test_model())).await;
    assert_eq!(
        first.next_until(is_pending_steers).await.first(),
        Some(&ServerFrame::Opened)
    );
    assert_eq!(second.next().await, Some(ServerFrame::Closed));
}

#[tokio::test(flavor = "local")]
async fn two_opens_of_one_conversation_at_once_build_one_tree() {
    let script = ScriptedRuntime::new(Vec::new());
    let fixture = Fixture::new(&script);
    let mut first = fixture.client();
    let mut second = fixture.client();

    tokio::join!(
        first.send(open(test_model())),
        second.send(open(test_model()))
    );

    assert_eq!(fixture.resolver.calls.borrow().len(), 1);
    assert_eq!(fixture.store.saves().len(), 1);
    let first_frames = first.received();
    let second_frames = second.received();
    assert_eq!(first_frames.first(), Some(&ServerFrame::Opened));
    assert_eq!(second_frames.first(), Some(&ServerFrame::Opened));
    // The one that opened first was taken over.
    let closed = [&first_frames, &second_frames]
        .iter()
        .filter(|frames| frames.last() == Some(&ServerFrame::Closed))
        .count();
    assert_eq!(closed, 1);
}

#[tokio::test(flavor = "local")]
async fn frames_that_need_a_session_are_refused_without_one_and_while_it_is_busy() {
    let script = ScriptedRuntime::new([Turn::pending()]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.client();
    let steer_id = BlockId::try_from("s1").unwrap();

    for frame in [
        send("m1", "hi"),
        ClientFrame::Abort {},
        ClientFrame::SyncTranscript {},
        ClientFrame::DequeueMessage {
            message_id: turn("m1"),
        },
        ClientFrame::Steer {
            steer_id: steer_id.clone(),
            content: demi_agent::testing::text("steer"),
        },
        ClientFrame::SetProvider {
            model: test_model(),
            apply: None,
        },
        ClientFrame::CancelPendingSteer {
            steer_id: steer_id.clone(),
        },
        ClientFrame::AbortSubagents {},
        ClientFrame::AbortSubagent {
            subagent_id: NodeId::try_from("child").unwrap(),
        },
    ] {
        client.send(frame).await;
    }

    assert_eq!(
        client.received(),
        [
            rejected(ClientFrameKind::Send, "No session is open"),
            rejected(ClientFrameKind::Abort, "No session is open"),
            rejected(ClientFrameKind::SyncTranscript, "No session is open"),
            rejected(ClientFrameKind::DequeueMessage, "No session is open"),
            ServerFrame::SteerResult {
                steer_id,
                outcome: SteerOutcome::Rejected {
                    reason: "No session is open on this connection".into(),
                },
            },
            rejected(
                ClientFrameKind::SetProvider,
                "No session is open on this connection"
            ),
        ]
    );
    client.send(ClientFrame::Close {}).await;
    assert_eq!(client.received(), [ServerFrame::Closed]);

    client.send(open(test_model())).await;
    client.next_until(is_pending_steers).await;
    client.send(open(test_model())).await;
    assert_eq!(
        client.next().await,
        Some(rejected(
            ClientFrameKind::Open,
            "A session is already open on this connection"
        ))
    );
    client.send(send("m1", "hang")).await;
    client
        .next_until(|frame| {
            matches!(
                frame,
                ServerFrame::Phase {
                    phase: SessionPhase::Running
                }
            )
        })
        .await;
    for frame in [
        ClientFrame::Retry {},
        ClientFrame::Resume {},
        ClientFrame::Compact {},
    ] {
        let kind = frame.kind();
        client.send(frame).await;
        assert_eq!(
            client.next().await,
            Some(rejected(kind, "Session is busy (running)"))
        );
    }
}

#[tokio::test(flavor = "local")]
async fn an_unknown_provider_leaves_the_connection_unattached() {
    let script = ScriptedRuntime::new(Vec::new());
    let fixture = Fixture::new(&script);
    let mut client = fixture.client();

    client.send(open(model_of("missing", "some-model"))).await;
    client.send(send("m1", "hi")).await;

    assert_eq!(
        client.received(),
        [
            ServerFrame::Error {
                message: "Provider \"missing\" is not available".into(),
                code: None,
                diagnostics: None,
            },
            rejected(ClientFrameKind::Send, "No session is open"),
        ]
    );
    assert!(fixture.server.tree(&conversation()).is_none());
    assert!(fixture.store.record(&conversation()).is_none());
}

#[tokio::test(flavor = "local")]
async fn a_client_that_stops_reading_is_closed_as_lagging_and_a_reopen_adopts_the_running_tree() {
    let deltas = (0..40)
        .map(|index| event::text(&format!("{index} ")))
        .collect::<Vec<_>>();
    let script =
        ScriptedRuntime::new([Turn::Events([deltas, vec![event::response(1, 1)]].concat())]);
    let config = ServerConfig {
        outbox_frames: 16,
        ..ServerConfig::default()
    };
    let fixture = Fixture::with(&script, MemoryTreeStore::new(), config);
    let mut slow = fixture.opened().await;

    slow.send(send("m1", "talk")).await;
    let tree = fixture.server.tree(&conversation()).unwrap();
    tree.root().session().settled().await;

    assert_eq!(slow.outgoing().await, Outgoing::Lagged);
    assert!(!tree.is_attached());
    let mut again = fixture.client();
    again.send(open(test_model())).await;
    let handshake = again.received();
    let ServerFrame::TranscriptReset { blocks, .. } = &handshake[1] else {
        panic!("{handshake:?}");
    };
    let serialized = serde_json::to_string(&blocks[1]).unwrap();
    assert!(serialized.contains("39 "), "{serialized}");
    assert_eq!(fixture.resolver.calls.borrow().len(), 1);

    // A connection whose socket is gone detaches, and its outbox ends.
    let (connection, mut frames) = fixture.server.connect(conversation(), "/workspace".into());
    connection.handle(open(test_model())).await;
    drop(connection);
    let mut drained = Vec::new();
    loop {
        match frames.recv().await {
            Outgoing::Frame(frame) => drained.push(frame),
            outgoing => {
                assert_eq!(outgoing, Outgoing::Closed);
                break;
            }
        }
    }
    assert_eq!(drained.len(), 5);
    assert!(!tree.is_attached());
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_tree_left_detached_and_quiescent_is_disposed_after_its_idle_time() {
    let script = ScriptedRuntime::new([Turn::Events(vec![
        event::text("done"),
        event::response(1, 1),
    ])]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "hi")).await;
    client.next_until(is_idle).await;
    drop(client);

    // An open inside the idle time keeps the tree; leaving starts it again.
    tokio::time::sleep(Duration::from_secs(300)).await;
    let back = fixture.opened().await;
    drop(back);
    tokio::time::sleep(Duration::from_secs(540)).await;
    assert!(fixture.server.tree(&conversation()).is_some());

    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(fixture.server.tree(&conversation()).is_none());
    let checkpoint = fixture.store.checkpoint(&conversation()).unwrap();
    assert_eq!(checkpoint.state.phase, SessionPhase::Idle);
    assert_eq!(checkpoint.transcript.len(), 3);
    assert_eq!(script.closes(), 1);
}

#[tokio::test(flavor = "local")]
async fn shutdown_disposes_every_live_tree_with_its_final_checkpoint() {
    let script = ScriptedRuntime::new([Turn::pending()]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    client.send(send("m1", "hang")).await;
    until(|| script.requests().len() == 1).await;

    fixture.server.shutdown().await;

    assert!(fixture.server.tree(&conversation()).is_none());
    let checkpoint = fixture.store.checkpoint(&conversation()).unwrap();
    assert_eq!(checkpoint.state.phase, SessionPhase::Running);
    assert_eq!(kinds(&checkpoint.transcript), ["user", "error"]);
    assert_eq!(script.closes(), 1);
    // The attached client saw the interruption record before the tree went.
    let frames = client.received();
    let recorded = frames.iter().any(|frame| {
        matches!(frame, ServerFrame::TranscriptPatch { patches, .. }
            if serde_json::to_string(patches).unwrap().contains("\"code\":\"interrupted\""))
    });
    assert!(recorded, "{frames:?}");
}
