//! Connections to a conversation's live tree: the open handshake, two opens
//! at once, several connections of one tree with their events, replies and
//! lag, connections acting at once, the refusals of frames that need a
//! session, a connection dropped with its socket, and the eviction of a tree
//! left detached and quiescent. The backend's scenarios show two sockets of
//! one conversation and a client that falls behind over the real socket.

use std::time::Duration;

use demi_agent_server::{
    Outgoing, ServerConfig,
    testing::{TestFiles, client_text},
};
use demi_agent_store::testing::{MemoryTreeStore, model_of};
use demi_conversation_socket_protocol::{
    ClientFrame, ClientFrameKind, EditOutcome, ServerFrame, SteerOutcome,
};
use demi_provider_common::{
    ProviderEvent,
    testing::{ScriptedRuntime, Turn, event},
};
use demi_shared_types::{Block, BlockId, CommandId, NodeId, SessionPhase};
use futures_util::{StreamExt as _, stream};

use crate::{
    editing::{edit, edit_outcome, said, user_block},
    support::{
        Fixture, Gate, conversation, frame_type, held, is_context_usage, is_idle,
        is_pending_steers, kinds, open, send, session_of, turn, until,
    },
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

    client.send(open()).await;
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
            "pending_steers",
            "context_usage"
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
async fn two_opens_of_one_conversation_at_once_build_one_tree() {
    let script = ScriptedRuntime::new(Vec::new());
    let fixture = Fixture::new(&script);
    let mut first = fixture.client();
    let mut second = fixture.client();

    tokio::join!(first.send(open()), second.send(open()));

    assert_eq!(fixture.resolver.calls.borrow().len(), 1);
    assert_eq!(fixture.store.saves().len(), 1);
    let first_frames = first.received();
    let second_frames = second.received();
    assert_eq!(first_frames.first(), Some(&ServerFrame::Opened));
    assert_eq!(second_frames.first(), Some(&ServerFrame::Opened));
    // Neither open took the tree from the other.
    assert!(
        !first_frames
            .iter()
            .chain(&second_frames)
            .any(|frame| *frame == ServerFrame::Closed)
    );
}

/// The queue's messages by id.
fn queued(frame: &ServerFrame) -> Option<Vec<String>> {
    match frame {
        ServerFrame::Queue { queue } => {
            Some(queue.iter().map(|message| message.id.to_string()).collect())
        }
        _ => None,
    }
}

// Two tabs of one conversation: every event of the tree reaches both, a
// reply only the tab that asked, and a close disposes the tree for both.
#[tokio::test(flavor = "local")]
async fn two_connections_of_one_tree_receive_its_events_and_each_its_own_replies() {
    let script = ScriptedRuntime::new([Turn::Events(vec![
        event::text("a"),
        event::text("b"),
        event::response(1, 1),
    ])]);
    let fixture = Fixture::new(&script);
    let mut first = fixture.opened().await;
    let mut second = fixture.opened().await;

    first.send(send("m1", "hi")).await;
    let seen_by_first = first.next_until(is_idle).await;
    let seen_by_second = second.next_until(is_idle).await;
    assert_eq!(seen_by_first, seen_by_second);
    assert!(
        seen_by_second
            .iter()
            .any(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. })),
        "{seen_by_second:?}"
    );

    second
        .send(ClientFrame::Steer {
            steer_id: BlockId::try_from("s1").unwrap(),
            content: client_text("too late"),
        })
        .await;
    second.send(ClientFrame::Abort {}).await;
    second.send(ClientFrame::SyncTranscript {}).await;
    second
        .send(ClientFrame::ShellWrite {
            command_id: CommandId::try_from("no-such-command").unwrap(),
            stdin: "y\n".into(),
        })
        .await;
    let replies: Vec<String> = second.received().iter().map(frame_type).collect();
    assert_eq!(
        replies,
        ["steer_result", "abort_result", "transcript_reset", "error"]
    );
    assert_eq!(first.received(), []);

    first.send(ClientFrame::Close {}).await;
    let closed = |frames: &[ServerFrame]| {
        frames
            .iter()
            .filter(|frame| **frame == ServerFrame::Closed)
            .count()
    };
    let first_end = first.received();
    let second_end = second.received();
    assert_eq!(first_end.last(), Some(&ServerFrame::Closed));
    assert_eq!(closed(&first_end), 1, "{first_end:?}");
    assert_eq!(second_end.last(), Some(&ServerFrame::Closed));
    assert!(fixture.server.tree(&conversation()).is_none());
}

// A tab that stops reading is closed as lagging; the other tab of the
// conversation receives the whole turn. The run yields after each delta, as
// a vendor's stream does, so the reading tab empties its outbox as the turn
// streams.
#[tokio::test(flavor = "local")]
async fn a_connection_that_falls_behind_is_closed_alone_and_the_other_receives_the_turn() {
    let deltas: Vec<ProviderEvent> = (0..40)
        .map(|index| event::text(&format!("{index} ")))
        .chain([event::response(1, 1)])
        .collect();
    let script = ScriptedRuntime::new([Turn::Stream(Box::new(move |_| {
        stream::iter(deltas)
            .then(|event| async move {
                tokio::task::yield_now().await;
                event
            })
            .boxed_local()
    }))]);
    let config = ServerConfig {
        outbox_frames: 16,
        ..ServerConfig::default()
    };
    let fixture = Fixture::with(&script, MemoryTreeStore::new(), config);
    let mut reading = fixture.opened().await;
    let mut stalled = fixture.opened().await;

    reading.send(send("m1", "count")).await;
    let turn = reading.next_until(is_idle).await;

    let patches = turn
        .iter()
        .filter(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. }))
        .count();
    assert!(
        patches > 16,
        "the turn outgrew one outbox: {patches} patches"
    );
    let mut held = 0;
    let end = loop {
        match stalled.outgoing().await {
            Outgoing::Frame(_) => held += 1,
            end => break end,
        }
    };
    assert_eq!(end, Outgoing::Lagged);
    assert!(held <= 16, "{held}");
    let text: String = session_of(&fixture)
        .transcript()
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect();
    assert!(text.ends_with("39 "), "{text}");
    assert!(fixture.server.tree(&conversation()).unwrap().is_attached());
}

// Two tabs acting at once need no rule of their own: two sends queue in the
// order they arrive, a send while the other tab's edit is prepared is
// refused, and so is an edit while the other tab's send runs.
#[tokio::test(flavor = "local")]
async fn connections_acting_at_once_are_decided_by_the_rules_of_one() {
    let first_turn = Gate::new();
    let busy_turn = Gate::new();
    let script = ScriptedRuntime::new([
        held(&first_turn, vec![event::text("one"), event::response(1, 1)]),
        said("two"),
        said("three"),
        said("one again"),
        held(&busy_turn, vec![event::text("busy"), event::response(1, 1)]),
    ]);
    let fixture = Fixture::new(&script);
    let mut first = fixture.opened().await;
    let mut second = fixture.opened().await;

    first.send(send("m1", "one")).await;
    second.send(send("m2", "two")).await;
    first.send(send("m3", "three")).await;
    first_turn.open();
    let seen_by_first = first.next_until(is_idle).await;
    let seen_by_second = second.next_until(is_idle).await;
    let queues: Vec<Vec<String>> = seen_by_second.iter().filter_map(queued).collect();
    assert!(
        queues.contains(&vec!["m2".to_owned(), "m3".to_owned()]),
        "{queues:?}"
    );
    assert_eq!(seen_by_first, seen_by_second);
    let turns: Vec<String> = session_of(&fixture)
        .transcript()
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::User(user) => Some(user.turn_id.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(turns, ["m1", "m2", "m3"]);

    // The second tab sends while the first tab's edit waits for its save.
    let target = user_block(&fixture, "m1");
    let version = session_of(&fixture).transcript().version;
    let saves = fixture.store.hold_saves();
    {
        let (connection, _) = first.split();
        let editing = connection.handle(edit("op1", &target, &version, client_text("one again")));
        tokio::pin!(editing);
        while saves.waiting() == 0 {
            assert!(futures_util::poll!(editing.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        second.send(send("m4", "during the edit")).await;
        saves.release();
        editing.await;
    }
    let refused = second.next_until(is_idle).await;
    assert!(
        refused.contains(&ServerFrame::Rejected {
            command: ClientFrameKind::Send,
            reason: "A message edit is being prepared".into(),
        }),
        "{refused:?}"
    );
    let edited = first.next_until(is_idle).await;
    let EditOutcome::Accepted { turn_id } = edit_outcome(&edited) else {
        panic!("{edited:?}");
    };

    // The first tab edits while the second tab's send runs.
    let target = user_block(&fixture, turn_id.as_str());
    second.send(send("m5", "busy")).await;
    until(|| script.requests().len() == 5).await;
    let version = session_of(&fixture).transcript().version;
    first
        .send(edit("op2", &target, &version, client_text("not now")))
        .await;
    let answer = first.received();
    busy_turn.open();
    assert_eq!(
        edit_outcome(&answer),
        EditOutcome::Rejected {
            reason: "Message editing requires a settled session with no pending work".into(),
        }
    );
    second.next_until(is_idle).await;
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
            content: demi_agent_server::testing::client_text("steer"),
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
        ]
    );
    client.send(ClientFrame::Close {}).await;
    assert_eq!(client.received(), [ServerFrame::Closed]);

    client.send(open()).await;
    // The handshake, then the usage that follows it.
    client.next_until(is_context_usage).await;
    client.send(open()).await;
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

    fixture.resolver.select(model_of("missing", "some-model"));
    client.send(open()).await;
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

/// The backend drops a connection when its socket is gone: the connection
/// detaches from its tree, and its outbox ends once the frames it holds are
/// read, so the socket's writer ends too.
#[tokio::test(flavor = "local")]
async fn a_connection_dropped_with_its_socket_detaches_and_its_outbox_ends() {
    let script = ScriptedRuntime::new(Vec::new());
    let fixture = Fixture::new(&script);
    let (connection, mut frames) =
        fixture
            .server
            .connect(conversation(), "/workspace".into(), TestFiles::new());
    connection.handle(open()).await;
    let tree = fixture.server.tree(&conversation()).unwrap();
    assert!(tree.is_attached());

    drop(connection);

    assert!(!tree.is_attached());
    let read = tokio::time::timeout(Duration::from_secs(10), async {
        let mut drained = Vec::new();
        loop {
            match frames.recv().await {
                Outgoing::Frame(frame) => drained.push(frame),
                outgoing => return (drained, outgoing),
            }
        }
    })
    .await;
    let (drained, end) = read.expect("the outbox ends");
    assert_eq!(end, Outgoing::Closed);
    let handshake: Vec<String> = drained.iter().map(frame_type).collect();
    assert_eq!(
        handshake,
        [
            "opened",
            "transcript_reset",
            "phase",
            "queue",
            "pending_steers",
            "context_usage"
        ]
    );
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
