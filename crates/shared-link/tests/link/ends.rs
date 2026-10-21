//! How a link ends, and what it keeps while it lives.

use std::time::Duration;

use demi_shared_link::{SocketEnd, testing};
use demi_runner_protocol::wire::{self, Outbound};
use futures_util::StreamExt as _;

use super::{HANG_GUARD, PING, exit, input, output, request};

/// A sender that waits for room on a lane returns once the link is lost,
/// with its message, instead of waiting for room that never comes.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_sender_on_a_full_lane_returns_once_the_link_is_lost() {
    let (_runner, mut backend, network) = testing::pair(Some(PING));
    network.stall();
    let outgoing = backend.outgoing.clone();
    let writer = tokio::task::spawn_local(async move {
        for _ in 0..256 {
            if outgoing.send(input("stuck", 64 * 1024)).await.is_err() {
                return "returned an error";
            }
        }
        "sent everything"
    });
    let ended = tokio::time::timeout(Duration::from_secs(5), backend.incoming.next())
        .await
        .expect("the link is lost");
    assert_eq!(ended, Some(Err(SocketEnd::Unresponsive)));
    let finished = tokio::time::timeout(Duration::from_secs(60), writer)
        .await
        .expect("the sender returns after the link was lost")
        .unwrap();
    assert_eq!(finished, "returned an error");
}

/// Everything sent before a close reaches the peer before the close does, on
/// every lane.
#[tokio::test(flavor = "local")]
async fn what_was_sent_before_a_close_arrives_before_it() {
    let (runner, mut backend, _network) = testing::pair(None);
    for offset in 0..100u64 {
        for job in ["a", "b", "c", "d"] {
            runner.outgoing.send(output(job, offset)).await.unwrap();
        }
    }
    for id in 0..30 {
        runner
            .outgoing
            .send(wire::encode(&Outbound::SyncDone { id: format!("r{id}"), error: None }).unwrap())
            .await
            .unwrap();
    }
    runner.outgoing.close(None).await;
    let mut count = 0;
    loop {
        let next = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
            .await
            .expect("the link ends within the hang guard");
        match next {
            Some(Ok(_)) => count += 1,
            None => break,
            Some(Err(error)) => panic!("the link failed: {error:?}"),
        }
    }
    assert_eq!(count, 430);
}

/// A link that carries many commands and requests over its life keeps no
/// record of the ones that ended.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_long_lived_link_keeps_no_record_of_commands_that_ended() {
    let (mut runner, mut backend, _network) = testing::pair(None);
    // Each end takes what the other sends, as the application does, and the
    // runner answers the requests.
    let answers = runner.outgoing.clone();
    let taking = tokio::task::spawn_local(async move {
        let mut taken = 0;
        while let Some(Ok(bytes)) = runner.incoming.next().await {
            taken += 1;
            if let Ok(wire::Inbound::Sync { id }) = wire::decode(&bytes) {
                let done = wire::encode(&Outbound::SyncDone { id, error: None }).unwrap();
                answers.send(done).await.unwrap();
            }
        }
        taken
    });
    let backend_out = backend.outgoing.clone();
    let counting = tokio::task::spawn_local(async move {
        let mut taken = 0;
        while let Some(Ok(_)) = backend.incoming.next().await {
            taken += 1;
            if taken == 900 {
                break;
            }
        }
        backend
    });
    for round in 0..300 {
        backend_out.send(request(&format!("r{round}"))).await.unwrap();
        backend_out.send(input(&format!("j{round}"), 100)).await.unwrap();
        runner
            .outgoing
            .send(output(&format!("j{round}"), 0))
            .await
            .unwrap();
        runner.outgoing.send(exit(&format!("j{round}"))).await.unwrap();
    }
    let backend = tokio::time::timeout(HANG_GUARD, counting)
        .await
        .expect("the output arrives")
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(runner.health.records(), 0);
    assert_eq!(backend.health.records(), 0);
    taking.abort();
}

/// A side may open 1,024 streams; the next opens once one ends, and nothing
/// is given up for it or dropped. Its sender is not held for it: its message
/// waits in its lane.
#[tokio::test(flavor = "local", start_paused = true)]
async fn the_command_past_the_limit_waits_until_one_ends() {
    let (runner, mut backend, _network) = testing::pair(None);
    for command in 0..1024 {
        runner.outgoing.send(output(&format!("c{command}"), 0)).await.unwrap();
    }
    let outgoing = runner.outgoing.clone();
    let waiting = tokio::task::spawn_local(async move {
        outgoing.send(output("c1024", 0)).await.unwrap();
    });
    let mut taken = Vec::new();
    while let Ok(Some(Ok(bytes))) =
        tokio::time::timeout(Duration::from_secs(1), backend.incoming.next()).await
    {
        taken.push(bytes);
    }
    assert_eq!(taken.len(), 1024);
    runner.outgoing.send(exit("c0")).await.unwrap();
    let rest = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("the exit arrives")
        .unwrap()
        .unwrap();
    assert!(matches!(wire::decode(&rest).unwrap(), Outbound::JobExit { .. }));
    let last = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("the command past the limit starts")
        .unwrap()
        .unwrap();
    assert!(matches!(wire::decode(&last).unwrap(), Outbound::JobOutput { job_id, .. } if job_id == "c1024"));
    waiting.await.unwrap();
}
