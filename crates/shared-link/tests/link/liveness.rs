use std::time::Duration;

use demi_shared_link::{Liveness, SocketEnd, testing};
use futures_util::StreamExt as _;

use super::{HANG_GUARD, PING, input, request};

/// A network that stops passing data while its sockets stay open is lost
/// within the ping's deadline, though a stream is full of input that cannot
/// leave. Before, the ping waited in the queue behind that input and never
/// went.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_network_that_passes_nothing_is_lost_within_the_ping_deadline() {
    let (_runner, mut backend, network) = testing::pair(Some(PING));
    network.stall();
    let outgoing = backend.outgoing.clone();
    // Input that fills what the network and the lane hold.
    tokio::task::spawn_local(async move {
        for _ in 0..256 {
            if outgoing.send(input("stuck", 64 * 1024)).await.is_err() {
                return;
            }
        }
    });
    let ended = tokio::time::timeout(
        PING.interval + PING.timeout * 2,
        backend.incoming.next(),
    )
    .await
    .expect("the link ends within the deadline");
    assert_eq!(ended, Some(Err(SocketEnd::Unresponsive)));
    assert!(backend.outgoing.send(request("late")).await.is_err());
}

/// A paused link counts no ping unanswered, and a resumed one is checked
/// again.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_paused_link_is_not_lost_until_it_resumes() {
    let (runner, mut backend, network) = testing::pair(Some(PING));
    backend.health.pause();
    runner.health.pause();
    network.stall();
    tokio::time::sleep(PING.timeout * 10).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1), backend.incoming.next())
            .await
            .is_err(),
        "a paused link stays"
    );
    backend.health.resume();
    runner.health.resume();
    let ended = tokio::time::timeout(PING.interval + PING.timeout * 2, backend.incoming.next())
        .await
        .expect("the resumed link is checked again");
    assert_eq!(ended, Some(Err(SocketEnd::Unresponsive)));
}

/// A ping on demand answers while the other end does, and not once it stops.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_ping_on_demand_is_answered_while_the_peer_answers() {
    let (_runner, backend, network) = testing::pair(Some(Liveness {
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(60),
    }));
    assert!(backend.health.answers(Duration::from_secs(5)).await);
    network.stall();
    assert!(!backend.health.answers(Duration::from_secs(5)).await);
    // A paused link answers: its silence is no death.
    backend.health.pause();
    assert!(backend.health.answers(Duration::from_secs(5)).await);
}

/// A peer that refuses a message ends the link with the reason it gave.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_close_carries_the_refusal_it_names() {
    let (mut runner, backend, _network) = testing::pair(None);
    backend
        .outgoing
        .close(Some("cannot decode job_start: unknown field `shell`".into()))
        .await;
    assert_eq!(
        runner.incoming.next().await,
        Some(Err(SocketEnd::Refused(
            "cannot decode job_start: unknown field `shell`".into()
        )))
    );
}

/// A link ends for its peer once its last user is dropped, as a socket does,
/// so a connection nobody uses is not held open; one user left keeps it.
#[tokio::test(flavor = "local")]
async fn a_link_ends_for_the_peer_once_its_last_user_is_dropped() {
    let (runner, mut backend, _network) = testing::pair(None);
    // Without the messages, the sender still reaches its peer.
    let outgoing = runner.outgoing.clone();
    drop(runner);
    outgoing.send(request("last")).await.unwrap();
    let message = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("the message within the hang guard");
    assert!(matches!(message, Some(Ok(_))), "{message:?}");
    drop(outgoing);
    let ended = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("the peer's link ends within the hang guard");
    assert!(!matches!(ended, Some(Ok(_))), "{ended:?}");
}

/// A slow link whose messages keep coming is not lost, though one message
/// takes far longer than the deadline to cross: the peer is there as long as
/// anything arrives, and a ping stuck behind data does not say it is not.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_slow_link_that_keeps_delivering_is_not_lost() {
    let (runner, mut backend, network) = testing::pair(Some(PING));
    // A message of the link crosses in 100 ms; the 4 MiB message below is
    // some 130 of them, a dozen seconds against a deadline of half a second.
    network.slow(Duration::from_millis(100));
    let message = demi_runner_protocol::wire::encode(&demi_runner_protocol::wire::Outbound::PackageMessage {
        package: "demi.example".into(),
        conversation_id: "c".into(),
        message: serde_json::Value::String("x".repeat(4 * 1024 * 1024 - 1024)),
    })
    .unwrap();
    runner.outgoing.send(message).await.unwrap();
    let arrived = tokio::time::timeout(Duration::from_secs(60), backend.incoming.next())
        .await
        .expect("the message crosses");
    assert!(matches!(arrived, Some(Ok(_))), "{arrived:?}");
}

/// An end that did not run for longer than the deadline, as a machine copied
/// or asleep does, did not hear nothing: its check runs after it reads what
/// arrived, and counts the time it was away as no silence.
#[tokio::test(flavor = "local", start_paused = true)]
async fn an_end_that_did_not_run_does_not_count_the_time_as_silence() {
    let (mut runner, backend, _network) = testing::pair(Some(PING));
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Every timer of both ends is due at once, far past its deadline.
    tokio::time::advance(Duration::from_secs(100)).await;
    tokio::task::yield_now().await;
    backend.outgoing.send(request("after")).await.unwrap();
    let bytes = tokio::time::timeout(HANG_GUARD, runner.incoming.next())
        .await
        .expect("a message within the hang guard")
        .expect("the link is open")
        .expect("the link has not failed");
    assert!(matches!(
        demi_runner_protocol::wire::decode::<demi_runner_protocol::wire::Inbound>(&bytes).unwrap(),
        demi_runner_protocol::wire::Inbound::Sync { .. }
    ));
}
