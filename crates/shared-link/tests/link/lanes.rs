use std::time::Duration;

use demi_shared_link::testing;
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use futures_util::StreamExt as _;

use super::{HANG_GUARD, PING, exit, input, kill, next, output, request, wire_output};

/// The messages of one lane arrive in the order they were sent, however
/// many other lanes are busy meanwhile.
#[tokio::test(flavor = "local")]
async fn the_messages_of_one_lane_arrive_in_the_order_they_were_sent() {
    let (runner, mut backend, _network) = testing::pair(None);
    let jobs = ["a", "b", "c"];
    for offset in 0..60u64 {
        for job in jobs {
            runner.outgoing.send(output(job, offset)).await.unwrap();
        }
    }
    let mut seen = std::collections::HashMap::<String, u64>::new();
    for _ in 0..180 {
        let bytes = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
            .await
            .expect("a message within the hang guard")
            .unwrap()
            .unwrap();
        let Outbound::JobOutput { job_id, offset, .. } = wire::decode(&bytes).unwrap() else {
            panic!("expected output");
        };
        let next = seen.entry(job_id.clone()).or_default();
        assert_eq!(offset, *next, "job {job_id}");
        *next += 1;
    }
}

/// One command's messages arrive in the order they were sent when many
/// commands send at once from tasks that run on several threads: each command
/// is one stream, so no scheduling of tasks reorders it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_commands_messages_arrive_in_order_when_many_commands_send_on_many_threads() {
    let (runner, mut backend, _network) = testing::pair(None);
    let senders: Vec<_> = (0..8)
        .map(|job| {
            let outgoing = runner.outgoing.clone();
            tokio::spawn(async move {
                for offset in 0..300u64 {
                    outgoing.send(output(&format!("job-{job}"), offset)).await.unwrap();
                    if offset % 50 == 0 {
                        // Requests of other streams among them.
                        outgoing
                            .send(wire::encode(&Outbound::SyncDone { id: format!("r-{job}-{offset}"), error: None }).unwrap())
                            .await
                            .unwrap();
                    }
                }
            })
        })
        .collect();
    let mut seen = std::collections::HashMap::<String, u64>::new();
    let mut answers = 0;
    while answers + seen.values().sum::<u64>() < 8 * 300 + 8 * 6 {
        let bytes = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
            .await
            .expect("a message within the hang guard")
            .unwrap()
            .unwrap();
        match wire::decode::<Outbound>(&bytes).unwrap() {
            Outbound::JobOutput { job_id, offset, .. } => {
                let next = seen.entry(job_id.clone()).or_default();
                assert_eq!(offset, *next, "{job_id}");
                *next += 1;
            }
            Outbound::SyncDone { .. } => answers += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    for sender in senders {
        sender.await.unwrap();
    }
}

/// The reply goes back on the stream of its request, and a message of
/// either end that has no stream yet opens one.
#[tokio::test(flavor = "local")]
async fn a_request_is_answered_and_each_end_opens_lanes_of_its_own() {
    let (mut runner, mut backend, _network) = testing::pair(None);
    backend.outgoing.send(request("r1")).await.unwrap();
    let Inbound::Sync { id } = next(&mut runner.incoming).await else {
        panic!("expected a request");
    };
    runner
        .outgoing
        .send(wire::encode(&Outbound::SyncDone { id, error: None }).unwrap())
        .await
        .unwrap();
    let bytes = backend.incoming.next().await.unwrap().unwrap();
    assert!(matches!(
        wire::decode::<Outbound>(&bytes).unwrap(),
        Outbound::SyncDone { id, .. } if id == "r1"
    ));
    // The runner starts a lane of its own, which the backend answers on.
    runner
        .outgoing
        .send(wire::encode(&Outbound::RpcCancel { call_id: "c1".into() }).unwrap())
        .await
        .unwrap();
    backend.incoming.next().await.unwrap().unwrap();
    backend
        .outgoing
        .send(wire::encode(&Inbound::RpcExit { call_id: "c1".into(), exit_code: 0 }).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        next(&mut runner.incoming).await,
        Inbound::RpcExit { call_id, .. } if call_id == "c1"
    ));
}

/// A large reply is on its way, its sender and its stream busy, while a job
/// prints on another lane: the output is not held until the reply is whole.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_large_reply_in_flight_does_not_hold_a_jobs_output() {
    let (runner, mut backend, network) = testing::pair(None);
    // A network that passes a message in a moment, as a real one does.
    network.slow(std::time::Duration::from_millis(5));
    let large = wire::encode(&Outbound::PackageMessage {
        package: "demi.example".into(),
        conversation_id: "c".into(),
        message: serde_json::Value::String("x".repeat(2 * 1024 * 1024)),
    })
    .unwrap();
    runner.outgoing.send(large).await.unwrap();
    let outgoing = runner.outgoing.clone();
    let printing = tokio::task::spawn_local(async move {
        for offset in 0..200u64 {
            outgoing.send(output("j", offset)).await.unwrap();
        }
    });
    // The first output arrives while the reply is still crossing, which takes
    // far longer than one message.
    let first = tokio::time::timeout(std::time::Duration::from_secs(1), backend.incoming.next())
        .await
        .expect("the output does not wait for the reply")
        .unwrap()
        .unwrap();
    assert!(matches!(wire::decode::<Outbound>(&first).unwrap(), Outbound::JobOutput { .. }));
    let mut outputs = 1;
    let mut large_seen = false;
    while outputs < 200 || !large_seen {
        let bytes = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
            .await
            .expect("every message arrives")
            .unwrap()
            .unwrap();
        match wire::decode::<Outbound>(&bytes).unwrap() {
            Outbound::JobOutput { .. } => outputs += 1,
            Outbound::PackageMessage { .. } => large_seen = true,
            other => panic!("unexpected {other:?}"),
        }
    }
    printing.await.unwrap();
}

/// A message of 4 MiB, the most the wire allows, arrives whole.
#[tokio::test(flavor = "local")]
async fn the_largest_message_arrives_whole() {
    let (runner, mut backend, _network) = testing::pair(None);
    let size = wire::MAX_MESSAGE_BYTES - 1024;
    let message = wire::encode(&Outbound::PackageMessage {
        package: "demi.example".into(),
        conversation_id: "c".into(),
        message: serde_json::Value::String("x".repeat(size)),
    });
    let message = message.unwrap();
    let expected = message.encoded_len();
    runner.outgoing.send(message).await.unwrap();
    let bytes = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(bytes.len(), expected);
}

/// A reply larger than a stream's window arrives whole, though its request
/// already ended its end of the stream: the stream goes on reading, so the
/// window opens again.
#[tokio::test(flavor = "local")]
async fn a_reply_larger_than_the_window_answers_a_request_that_ended_its_end() {
    let (mut runner, mut backend, _network) = testing::pair(None);
    let ask = wire::encode(&Inbound::LogRead {
        id: "r1".into(),
        since: None,
        limit: 10,
        source: None,
    })
    .unwrap();
    backend.outgoing.send(ask).await.unwrap();
    let Inbound::LogRead { id, .. } = next(&mut runner.incoming).await else {
        panic!("expected a request");
    };
    let line = wire::LogLine {
        at: wire::Timestamp(1_700_000_000_000),
        source: "runner".into(),
        conversation_id: None,
        text: "x".repeat(4000),
    };
    let reply = wire::encode(&Outbound::LogLines {
        id,
        lines: vec![line; 500],
        next: 500,
    })
    .unwrap();
    let expected = reply.encoded_len();
    runner.outgoing.send(reply).await.unwrap();
    let bytes = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("the reply within the hang guard")
        .unwrap()
        .unwrap();
    assert_eq!(bytes.len(), expected);
}

/// A stream that is read as fast as it fills grows its window with the round
/// trip, so a command's output crosses a link of 100 ms of round trip at the
/// rate the network allows, not at 256 KiB a round trip: more than a few
/// windows are on the way at once.
#[tokio::test(flavor = "local")]
async fn a_stream_read_as_fast_as_it_fills_grows_its_window() {
    let (runner, mut backend, network) = testing::pair(None);
    network.latency(Duration::from_millis(25));
    let chunk = wire::encode(&Outbound::JobOutput {
        job_id: "j".into(),
        stream: wire::OutputStream::Stdout,
        offset: 0,
        bytes: wire::WireBytes(vec![b'z'; 60 * 1024]),
    })
    .unwrap();
    let (bytes, route) = chunk.into_parts();
    let sending = tokio::task::spawn_local({
        let outgoing = runner.outgoing.clone();
        async move {
            // 48 MiB, far more than a window that stays at 256 KiB would
            // carry in the time the window has to grow.
            for _ in 0..800 {
                let frame = wire::Frame::from_parts(bytes.clone(), route.clone());
                outgoing.send(frame).await.unwrap();
            }
        }
    });
    for _ in 0..800 {
        tokio::time::timeout(HANG_GUARD, backend.incoming.next())
            .await
            .expect("every message arrives")
            .unwrap()
            .unwrap();
    }
    sending.await.unwrap();
    let most = network.most_in_flight();
    assert!(most > 2 * 1024 * 1024, "{most} bytes at most were on the way");
}

/// A command ends with its stream, at both ends: what a long-lived link keeps
/// of a command goes when the command's exit has been sent and read, and a
/// message that comes after is dropped instead of opening a stream.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_command_that_ended_leaves_no_record_and_takes_no_more_messages() {
    let (mut runner, mut backend, _network) = testing::pair(None);
    backend.outgoing.send(input("j", 10)).await.unwrap();
    next(&mut runner.incoming).await;
    let exit = exit("j");
    runner.outgoing.send(exit).await.unwrap();
    let ended = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("the exit arrives")
        .unwrap()
        .unwrap();
    assert!(matches!(wire::decode(&ended).unwrap(), Outbound::JobExit { .. }));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(backend.health.records(), 0);
    assert_eq!(runner.health.records(), 0);
    // Input of the ended command goes nowhere, and holds nobody.
    backend.outgoing.send(input("j", 10)).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), runner.incoming.next())
            .await
            .is_err()
    );
    assert_eq!(backend.health.records(), 0);
}

/// A command that reads no input, whatever is sent it, stalls only itself: the
/// link reads that stream no further, its window fills, and its sender waits,
/// for as long as it takes, while the link stays up with its heartbeat and a
/// stop, another command's input and a request go on. What the application
/// lets go of lets its stream go on.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_command_that_reads_no_input_stalls_only_itself() {
    let (mut runner, backend, _network) = testing::pair(Some(PING));
    let sent = std::rc::Rc::new(std::cell::Cell::new(0usize));
    tokio::task::spawn_local({
        let outgoing = backend.outgoing.clone();
        let sent = sent.clone();
        async move {
            while outgoing.send(input("stuck", 64 * 1024)).await.is_ok() {
                sent.set(sent.get() + 1);
            }
        }
    });
    // The application takes what arrives of it and keeps it.
    let mut kept = Vec::new();
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(Duration::from_millis(100), runner.incoming.next_held()).await
    {
        kept.push(message);
    }
    let stalled_at = sent.get();
    // The stream's window of 16 MiB holds 256 of the messages, the
    // application 8, and the lane's queue 8 more.
    assert!(
        kept.len() <= 8 && stalled_at < 288,
        "{} messages held, {stalled_at} sent",
        kept.len()
    );
    // Many times the heartbeat's deadline later the link is up, nothing more
    // was sent, and others pass.
    tokio::time::sleep(PING.timeout * 20).await;
    assert_eq!(sent.get(), stalled_at);
    backend.outgoing.send(kill("stuck")).await.unwrap();
    backend.outgoing.send(input("other", 10)).await.unwrap();
    backend.outgoing.send(request("r")).await.unwrap();
    let mut passed = Vec::new();
    for _ in 0..3 {
        passed.push(next(&mut runner.incoming).await);
    }
    assert!(passed.iter().any(|message| matches!(message, Inbound::JobKill { .. })));
    assert!(passed.iter().any(|message| matches!(message, Inbound::JobStdin { job_id, .. } if job_id == "other")));
    assert!(passed.iter().any(|message| matches!(message, Inbound::Sync { .. })));
    // What the application lets go of lets its stream go on: the window is
    // renewed once half of it has been read.
    drop(kept);
    for _ in 0..stalled_at {
        assert!(matches!(
            next(&mut runner.incoming).await,
            Inbound::JobStdin { job_id, .. } if job_id == "stuck"
        ));
    }
    assert!(sent.get() > stalled_at);
}

/// Both ends send on the same command at the same moment, each opening a
/// stream for it: neither end's messages wait for the other's stream.
#[tokio::test(flavor = "local", start_paused = true)]
async fn ends_that_open_a_command_at_once_do_not_hold_each_others_messages() {
    let (mut runner, mut backend, _network) = testing::pair(None);
    for round in 0..16 {
        let id = format!("both-{round}");
        let (sent_by_backend, sent_by_runner) = tokio::join!(
            backend.outgoing.send(input(&id, 10)),
            runner.outgoing.send(output(&id, 0)),
        );
        sent_by_backend.unwrap();
        sent_by_runner.unwrap();
        let (at_runner, at_backend) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(2), runner.incoming.next()),
            tokio::time::timeout(Duration::from_secs(2), backend.incoming.next()),
        );
        assert!(at_runner.is_ok(), "round {round}: the runner heard nothing");
        assert!(at_backend.is_ok(), "round {round}: the backend heard nothing");
    }
}

/// A command's first megabytes of output cross in a round trip: the end that
/// opens a stream announces the window it gives the other's output in the
/// frame that opens it, so the first window is not 256 KiB, which at 100 ms of
/// round trip would take a dozen round trips for 3 MiB.
#[tokio::test(flavor = "local")]
async fn a_new_streams_first_window_carries_a_commands_first_output_in_a_round_trip() {
    let (mut runner, mut backend, network) = testing::pair(None);
    network.latency(Duration::from_millis(25));
    // The backend opens the command's stream with its input.
    backend.outgoing.send(input("j", 10)).await.unwrap();
    next(&mut runner.incoming).await;
    let (bytes, route) = wire_output(60 * 1024).into_parts();
    let sending = tokio::task::spawn_local({
        let outgoing = runner.outgoing.clone();
        async move {
            // 3 MiB, within the window the opened stream announced.
            for _ in 0..50 {
                let frame = wire::Frame::from_parts(bytes.clone(), route.clone());
                outgoing.send(frame).await.unwrap();
            }
        }
    });
    for _ in 0..50 {
        tokio::time::timeout(HANG_GUARD, backend.incoming.next())
            .await
            .expect("every message arrives")
            .unwrap()
            .unwrap();
    }
    sending.await.unwrap();
    let most = network.most_in_flight();
    assert!(most > 2 * 1024 * 1024, "{most} bytes at most were on the way");
}

/// A command's input beyond the first 256 KiB crosses once the end that
/// accepted the stream has announced its window, which it did as it accepted
/// the stream: the input does not wait for the window to grow over round
/// trips.
#[tokio::test(flavor = "local")]
async fn a_new_streams_first_window_carries_a_commands_input_once_it_is_accepted() {
    let (mut runner, backend, network) = testing::pair(None);
    network.latency(Duration::from_millis(25));
    let (bytes, route) = input("j", 60 * 1024).into_parts();
    let sending = tokio::task::spawn_local({
        let outgoing = backend.outgoing.clone();
        async move {
            // 3 MiB, of which the first 256 KiB go at once.
            for _ in 0..50 {
                let frame = wire::Frame::from_parts(bytes.clone(), route.clone());
                outgoing.send(frame).await.unwrap();
            }
        }
    });
    for _ in 0..50 {
        next(&mut runner.incoming).await;
    }
    sending.await.unwrap();
    let most = network.most_in_flight();
    assert!(most > 2 * 1024 * 1024, "{most} bytes at most were on the way");
}
