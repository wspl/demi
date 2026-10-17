//! Forwarded connections between two runners (`port-forwarding.md` § The
//! path of a connection): a real peer between two runners in process on
//! 127.0.0.1, one channel per TCP connection, and the pairs' schedule,
//! which runs on a paused clock with peers the test makes.

use std::time::Duration;

use demi_runner_direct::{Dial, PairPeer, Pairs, Path};
use demi_runner_protocol::direct::{Introduced, MAX_CHANNELS, MESSAGE_BYTES, QUEUE_BYTES};
use demi_runner_protocol::forward::{PEER_IDLE, PairReason, RETRY_WAITS};
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

use super::operations::Fake;
use super::runner;

/// A TCP server on 127.0.0.1 that sends back what it reads, and ends its
/// writing when its reader ends.
async fn echo() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (mut reader, mut writer) = socket.split();
        tokio::io::copy(&mut reader, &mut writer).await.unwrap();
        writer.shutdown().await.unwrap();
    });
    address
}

/// A connected TCP pair on 127.0.0.1: the program's end, and the end a
/// runner's listener accepted.
async fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let program = TcpStream::connect(listener.local_addr().unwrap());
    let (program, accepted) = tokio::join!(program, listener.accept());
    (program.unwrap(), accepted.unwrap().0)
}

/// Two runners in process, the second the Connect To that `connect` says,
/// whose peer the first offered as the backend introduces them, once it
/// connected: the runners, and the offering side's link.
async fn peered(connect: &Fake) -> (demi_runner_direct::Direct, demi_runner_direct::Direct, demi_runner_direct::PeerLink) {
    let listening = runner(&Fake::default());
    let connecting = runner(connect);
    let dialing = listening.dial("r1".into(), Vec::new()).await.expect("the runner offers");
    let answered = connecting
        .offer(
            "r1".into(),
            dialing.sdp,
            Introduced::Runner { device: "listen-device".into() },
            Vec::new(),
        )
        .await
        .expect("the other runner answers");
    listening.answered("r1", answered.sdp);
    let (mut offered, mut answering) = (dialing.candidates, answered.candidates);
    let (to_connecting, to_listening) = (connecting.clone(), listening.clone());
    tokio::spawn(async move {
        while let Some(candidate) = offered.recv().await {
            to_connecting.candidate("r1", candidate);
        }
    });
    tokio::spawn(async move {
        while let Some(candidate) = answering.recv().await {
            to_listening.candidate("r1", candidate);
        }
    });
    let link = dialing.link;
    assert_eq!(link.connected().await, Ok(()), "the two runners connect");
    (listening, connecting, link)
}

#[tokio::test]
async fn a_forwarded_connection_carries_bytes_both_ways_over_a_peer_between_two_runners() {
    let target = echo().await;
    let connect = Fake::default();
    connect
        .held
        .lock()
        .unwrap()
        .forwards
        .insert(("f1".into(), "listen-device".into()), target);
    let (_listening, _connecting, link) = peered(&connect).await;

    // Several windows' worth each way, so the sender waits for grants.
    let sent: Vec<u8> = (0..3_000_000u32).map(|index| (index % 251) as u8).collect();
    let (program, accepted) = tcp_pair().await;
    let channel = link.open("f1").await.expect("the peer opens a channel");
    let spliced = tokio::spawn(channel.splice(accepted));
    let (mut reader, mut writer) = program.into_split();
    let writing = {
        let sent = sent.clone();
        async move {
            writer.write_all(&sent).await.unwrap();
            writer.shutdown().await.unwrap();
        }
    };
    let reading = async move {
        let mut back = Vec::new();
        reader.read_to_end(&mut back).await.unwrap();
        back
    };
    let ((), back) = tokio::join!(writing, reading);
    assert_eq!(back.len(), sent.len(), "every byte came back, then the end");
    assert!(back == sent, "the bytes came back as they were sent");
    spliced.await.unwrap();

    // A forward the other runner does not connect is refused: the channel ends.
    let (mut program, accepted) = tcp_pair().await;
    let channel = link.open("unknown").await.expect("the peer opens a channel");
    let spliced = tokio::spawn(channel.splice(accepted));
    let mut nothing = Vec::new();
    program.read_to_end(&mut nothing).await.unwrap();
    assert!(nothing.is_empty(), "the program's connection ends with no bytes");
    spliced.await.unwrap();
}

// Under a second: two runners over loopback.
#[tokio::test]
async fn a_connection_that_ends_closes_its_channel_at_once_though_its_bytes_wait_for_the_other_sides_window() {
    // The Connect To never reaches its target, so it takes none of the
    // channel's bytes and grants nothing back.
    let connect = Fake::default();
    let mut hung_up = connect.hung_up.subscribe();
    let (_listening, _connecting, link) = peered(&connect).await;

    // Once the program wrote more than its sockets' buffers and the window
    // together hold, the listening side holds bytes the window does not let
    // it send; it reads at most the window and its queue's four messages
    // more, so the program gets that far.
    let listener = tokio::net::TcpSocket::new_v4().unwrap();
    listener.set_recv_buffer_size(16 * 1024).unwrap();
    listener.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let program = tokio::net::TcpSocket::new_v4().unwrap();
    program.set_send_buffer_size(16 * 1024).unwrap();
    let buffered = listener.recv_buffer_size().unwrap() as usize + program.send_buffer_size().unwrap() as usize;
    let beyond = QUEUE_BYTES + buffered + 2 * MESSAGE_BYTES;
    let listener = listener.listen(1).unwrap();
    let (program, accepted) = tokio::join!(program.connect(listener.local_addr().unwrap()), listener.accept());
    let (program, accepted) = (program.unwrap(), accepted.unwrap().0);
    let channel = link.open("hang").await.expect("the peer opens a channel");
    let spliced = tokio::spawn(channel.splice(accepted));
    let (written, mut wrote) = tokio::sync::watch::channel(0usize);
    let (_program_reader, mut program_writer) = program.into_split();
    let writing = tokio::spawn(async move {
        let chunk = vec![1u8; 16 * 1024];
        while program_writer.write_all(&chunk).await.is_ok() {
            written.send_modify(|total| *total += chunk.len());
        }
    });
    wrote.wait_for(|total| *total >= beyond).await.unwrap();

    // Its connection ends, as its forward turned off: the channel closes,
    // and the Connect To gives its connection up.
    spliced.abort();
    let given_up = tokio::time::timeout(std::time::Duration::from_secs(20), hung_up.wait_for(|count| *count == 1)).await;
    assert!(given_up.is_ok(), "the channel closed though its bytes waited for a grant");
    writing.abort();
}

/// What the pairs asked of the test's dialer: the device, and where its
/// peer, or why there is none, goes.
type Asked = (String, oneshot::Sender<Result<TestPeer, Option<PairReason>>>);

struct TestDial(mpsc::UnboundedSender<Asked>);

impl Dial for TestDial {
    type Peer = TestPeer;

    fn dial(&self, device: String) -> BoxFuture<'static, Result<TestPeer, Option<PairReason>>> {
        let (answer, answered) = oneshot::channel();
        self.0.send((device, answer)).unwrap();
        answered.map(|answer| answer.unwrap_or(Err(None))).boxed()
    }
}

/// A peer the test made: it says when the pairs watch it and when they
/// close it.
#[derive(Clone)]
struct TestPeer {
    watched: mpsc::UnboundedSender<()>,
    closed: mpsc::UnboundedSender<Instant>,
    ended: watch::Receiver<Option<Option<PairReason>>>,
}

impl PairPeer for TestPeer {
    type Channel = ();

    fn open(&self, forward: String) -> BoxFuture<'static, Option<()>> {
        // A channel of forward `fails` does not open, as one of a peer that
        // just failed does not.
        async move { (forward != "fails").then_some(()) }.boxed()
    }

    fn ended(&self) -> BoxFuture<'static, Option<PairReason>> {
        self.watched.send(()).unwrap();
        let mut ended = self.ended.clone();
        async move {
            let seen = ended.wait_for(Option::is_some).await.map(|reason| reason.flatten());
            match seen {
                Ok(reason) => reason,
                // The test ended the peers' maker: nothing ends them now.
                Err(_) => std::future::pending().await,
            }
        }
        .boxed()
    }

    fn close(&self) {
        self.closed.send(Instant::now()).unwrap();
    }
}

struct TestPeers {
    watched: mpsc::UnboundedReceiver<()>,
    closed: mpsc::UnboundedReceiver<Instant>,
    /// Ends the peers it made, while the test holds it.
    _end: watch::Sender<Option<Option<PairReason>>>,
    peer: TestPeer,
}

fn test_peers() -> TestPeers {
    let (watched_sender, watched) = mpsc::unbounded_channel();
    let (closed_sender, closed) = mpsc::unbounded_channel();
    let (end, ended) = watch::channel(None);
    TestPeers {
        watched,
        closed,
        _end: end,
        peer: TestPeer { watched: watched_sender, closed: closed_sender, ended },
    }
}

#[tokio::test(start_paused = true)]
async fn a_pairs_peer_carries_up_to_the_channel_limit_and_closes_a_minute_after_its_last_connection() {
    let (asks, mut asked) = mpsc::unbounded_channel();
    let pairs = Pairs::new(TestDial(asks));
    let mut peers = test_peers();

    let first = pairs.path("b", "f1", true).await;
    assert!(matches!(first, Path::Relay(_)), "the first connection takes the relay, as nothing waits for the peer");
    let (device, answer) = asked.recv().await.unwrap();
    assert_eq!(device, "b", "it has the backend introduce the pair");
    answer.send(Ok(peers.peer.clone())).ok().unwrap();
    peers.watched.recv().await.unwrap();

    let mut direct = Vec::new();
    for _ in 0..MAX_CHANNELS {
        match pairs.path("b", "f1", true).await {
            Path::Direct((), guard) => direct.push(guard),
            Path::Relay(_) => panic!("a connection within the limit goes direct"),
        }
    }
    assert!(matches!(pairs.path("b", "f1", true).await, Path::Relay(_)), "past the limit, the relay");
    assert!(
        matches!(pairs.path("b", "f2", false).await, Path::Relay(_)),
        "a pair that may not talk directly takes the relay"
    );
    drop(first);

    // An hour with connections open closes nothing.
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert!(peers.closed.try_recv().is_err(), "the peer stays while it carries connections");
    let last = direct.pop().unwrap();
    drop(direct);
    let ended = Instant::now();
    drop(last);
    let closed = peers.closed.recv().await.unwrap();
    assert_eq!(closed - ended, PEER_IDLE, "the peer closes a minute after the last connection");
    assert!(asked.try_recv().is_err(), "no peer is tried while the pair is not on the relay");
}

#[tokio::test(start_paused = true)]
async fn a_pair_on_the_relay_tries_a_peer_again_on_the_schedule_and_when_the_runner_connects_again() {
    let (asks, mut asked) = mpsc::unbounded_channel();
    let pairs = Pairs::new(TestDial(asks));
    let mut reasons = pairs.reasons();

    let held = pairs.path("b", "f1", true).await;
    assert!(matches!(held, Path::Relay(_)));
    let (_, answer) = asked.recv().await.unwrap();
    let failed = Instant::now();
    answer.send(Err(Some(PairReason::Unreachable))).ok().unwrap();
    let reason = reasons.wait_for(|reasons| !reasons.is_empty()).await.unwrap().get("b").copied();
    assert_eq!(reason, Some(PairReason::Unreachable), "the pair says why it is on the relay");

    let mut waits = Vec::new();
    let mut last = failed;
    for attempt in 0..=RETRY_WAITS.len() {
        let (_, answer) = asked.recv().await.unwrap();
        waits.push((Instant::now() - last).as_secs() / 60);
        last = Instant::now();
        // The last fails for another reason, which says when it was taken.
        let reason = if attempt == RETRY_WAITS.len() { PairReason::Busy } else { PairReason::Unreachable };
        answer.send(Err(Some(reason))).ok().unwrap();
    }
    assert_eq!(waits, [1, 2, 5, 10, 10], "after 1, 2, 5 and then every 10 minutes");
    reasons.wait_for(|reasons| reasons.get("b") == Some(&PairReason::Busy)).await.unwrap();

    let before = Instant::now();
    pairs.online("b");
    let (device, _answer) = asked.recv().await.unwrap();
    assert_eq!(device, "b");
    assert_eq!(Instant::now(), before, "a runner that connects again is tried at once");
    drop(held);
}

/// The pairs with a connected peer to `b`, which one connection on the
/// relay made: the pairs, the test's peers, and that connection.
async fn connected_pair() -> (Pairs<TestDial>, TestPeers, Path<()>, mpsc::UnboundedReceiver<Asked>) {
    let (asks, mut asked) = mpsc::unbounded_channel();
    let pairs = Pairs::new(TestDial(asks));
    let mut peers = test_peers();
    let first = pairs.path("b", "f1", true).await;
    let (_, answer) = asked.recv().await.unwrap();
    answer.send(Ok(peers.peer.clone())).ok().unwrap();
    peers.watched.recv().await.unwrap();
    (pairs, peers, first, asked)
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_goes_while_it_waits_for_its_path_is_not_counted() {
    let (pairs, mut peers, first, _asked) = connected_pair().await;
    // Asked once and dropped before its answer, as a connection that ended
    // while its path was chosen.
    assert!(pairs.path("b", "f1", true).now_or_never().is_none(), "the answer comes later");
    let ended = Instant::now();
    drop(first);
    let closed = tokio::time::timeout(PEER_IDLE * 2, peers.closed.recv()).await;
    let closed = closed.expect("the peer closes once the last connection ended").unwrap();
    assert_eq!(closed - ended, PEER_IDLE);
}

#[tokio::test(start_paused = true)]
async fn a_connection_whose_channel_did_not_open_is_counted_once_on_the_relay() {
    let (pairs, mut peers, first, _asked) = connected_pair().await;
    let open = match pairs.path("b", "f1", true).await {
        Path::Direct((), reserved) => reserved,
        Path::Relay(_) => panic!("the connected peer carries it"),
    };
    let failed = pairs.path("b", "fails", true).await;
    assert!(matches!(failed, Path::Relay(_)), "a channel that did not open goes to the relay");
    drop((failed, first));
    // One connection is still open: an hour closes nothing.
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert!(peers.closed.try_recv().is_err(), "the peer stays while a connection is open");
    let ended = Instant::now();
    drop(open);
    let closed = peers.closed.recv().await.unwrap();
    assert_eq!(closed - ended, PEER_IDLE);
}

// No time: the clock is paused, and nothing answers 192.0.2.1, an address
// kept for documentation, which no network routes back.
#[tokio::test(start_paused = true)]
async fn a_target_that_does_not_accept_in_ten_seconds_or_whose_forward_ended_is_given_up() {
    let ended = tokio_util::sync::CancellationToken::new();
    let started = Instant::now();
    let refused = demi_runner_direct::connect_target("192.0.2.1:9", &ended).await.unwrap_err();
    if refused.kind() == std::io::ErrorKind::TimedOut {
        assert_eq!(started.elapsed(), demi_runner_protocol::forward::DEADLINE);
    } else {
        // A computer with no route at all refuses at once.
        assert_eq!(started.elapsed(), Duration::ZERO, "{refused}");
    }

    let connecting = tokio::spawn({
        let ended = ended.clone();
        async move { demi_runner_direct::connect_target("192.0.2.1:9", &ended).await }
    });
    tokio::task::yield_now().await;
    let cancelled = Instant::now();
    ended.cancel();
    let refused = connecting.await.unwrap().unwrap_err();
    assert_eq!(cancelled.elapsed(), Duration::ZERO, "{refused}");
}
