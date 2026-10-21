//! Two ends of a link joined by memory, over a network a test can stall.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt as _, StreamExt as _, stream};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use tokio_util::sync::PollSender;

use crate::{Connection, Liveness, Received, Side, Wire};

/// The messages the network holds in each direction before the sender waits,
/// as a socket's buffers do.
const NETWORK_MESSAGES: usize = 4;

/// The messages that crossed the network a latency makes wait, in flight at
/// once.
const IN_FLIGHT_MESSAGES: usize = 4096;

/// The bytes on the way between two ends, which a latency holds.
#[derive(Default)]
struct Flight {
    now: AtomicUsize,
    most: AtomicUsize,
}

/// The network between two ends.
pub struct Network {
    stalled: watch::Sender<bool>,
    delay: watch::Sender<Duration>,
    latency: watch::Sender<Duration>,
    flight: Arc<Flight>,
}

impl Network {
    /// Stops passing messages in both directions while keeping both sockets
    /// open, as a network that fails without telling either end.
    pub fn stall(&self) {
        self.stalled.send_replace(true);
    }

    /// Passes messages again.
    pub fn resume(&self) {
        self.stalled.send_replace(false);
    }

    /// Makes every message take `delay` to cross, in both directions, as a
    /// slow link does: the messages keep coming, though far apart.
    pub fn slow(&self, delay: Duration) {
        self.delay.send_replace(delay);
    }

    /// Makes every message arrive `latency` after it was sent, in both
    /// directions, as a long link does: messages follow each other closely
    /// and arrive late, so what a window holds is what crosses in a round
    /// trip. Set before the traffic it is for.
    pub fn latency(&self, latency: Duration) {
        self.latency.send_replace(latency);
    }

    /// The most bytes that were on the way at once through a latency.
    pub fn most_in_flight(&self) -> usize {
        self.flight.most.load(Ordering::Relaxed)
    }
}

/// A runner end and a backend end of one link, and the network between.
pub fn pair(liveness: Option<Liveness>) -> (Connection, Connection, Network) {
    let (stalled, watching) = watch::channel(false);
    let (delay, delaying) = watch::channel(Duration::ZERO);
    let (latency, lagging) = watch::channel(Duration::ZERO);
    let flight = Arc::new(Flight::default());
    let (runner_in, backend_out) = half(
        watching.clone(),
        delaying.clone(),
        lagging.clone(),
        flight.clone(),
    );
    let (backend_in, runner_out) = half(watching, delaying, lagging, flight.clone());
    let runner = Connection::open(runner_in, runner_out, Side::Runner, liveness);
    let backend = Connection::open(backend_in, backend_out, Side::Backend, liveness);
    (
        runner,
        backend,
        Network {
            stalled,
            delay,
            latency,
            flight,
        },
    )
}

/// One direction: the sink the sender writes and the stream the receiver
/// reads, with a hop between that holds everything while the network is
/// stalled.
fn half(
    stalled: watch::Receiver<bool>,
    delay: watch::Receiver<Duration>,
    latency: watch::Receiver<Duration>,
    flight: Arc<Flight>,
) -> (
    impl futures_util::Stream<Item = Result<Received, String>> + Unpin + Send + 'static,
    impl futures_util::Sink<Wire, Error = String> + Unpin + Send + 'static,
) {
    let (sent, mut sending) = mpsc::channel::<Wire>(NETWORK_MESSAGES);
    let (passed, received) = mpsc::channel::<Wire>(NETWORK_MESSAGES);
    let (late, mut arriving) = mpsc::channel::<(Instant, Wire)>(IN_FLIGHT_MESSAGES);
    let landing = flight.clone();
    let lands = passed.clone();
    tokio::spawn(async move {
        while let Some((at, message)) = arriving.recv().await {
            tokio::time::sleep_until(at).await;
            if let Wire::Binary(bytes) = &message {
                landing.now.fetch_sub(bytes.len(), Ordering::Relaxed);
            }
            if lands.send(message).await.is_err() {
                return;
            }
        }
    });
    tokio::spawn(async move {
        let mut stalled = stalled;
        while let Some(message) = sending.recv().await {
            if stalled.wait_for(|stalled| !stalled).await.is_err() {
                return;
            }
            let delay = *delay.borrow();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let latency = *latency.borrow();
            if latency.is_zero() {
                if passed.send(message).await.is_err() {
                    return;
                }
                continue;
            }
            if let Wire::Binary(bytes) = &message {
                let now = flight.now.fetch_add(bytes.len(), Ordering::Relaxed) + bytes.len();
                flight.most.fetch_max(now, Ordering::Relaxed);
            }
            if late.send((Instant::now() + latency, message)).await.is_err() {
                return;
            }
        }
    });
    let reading = stream::unfold(received, |mut received| async move {
        let message = received.recv().await?;
        let message = match message {
            Wire::Binary(bytes) => Received::Binary(bytes),
            Wire::Close(Some(reason)) => Received::Close(Some(crate::Refusal(reason))),
            Wire::Close(None) => Received::Close(None),
        };
        Some((Ok(message), received))
    })
    .boxed();
    let writing = PollSender::new(sent).sink_map_err(|error| error.to_string());
    (reading, writing)
}
