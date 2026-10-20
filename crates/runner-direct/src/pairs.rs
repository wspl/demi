//! The peers between this runner and the runners at the other end of its
//! forwards, one per pair
//! of runners, and the path each connection takes as it opens:
//!
//! - The first connection of any forward between a pair has the backend
//!   introduce the two runners, this one offering, and goes through the
//!   relay itself, since nothing waits for the peer.
//! - While the peer is connected, a connection goes direct as a data
//!   channel of it, up to [`MAX_CHANNELS`]; more go through the relay.
//! - The peer closes [`PEER_IDLE`] after the last connection between the
//!   pair ended.
//! - A peer that did not connect, or failed, sends the pair's connections
//!   through the relay, and while the pair carries connections a new peer
//!   is tried after [`RETRY_WAITS`], 1, 2 and 5 minutes and then every 10,
//!   or at once when the other runner connects again; with no connection
//!   left, the next one tries.
//! - A pair whose runners may not talk directly, a Cloud's or one whose
//!   route is Relay Only, always takes the relay.

use std::collections::HashMap;
use std::sync::Arc;

use demi_runner_protocol::direct::MAX_CHANNELS;
use demi_runner_protocol::forward::{PEER_IDLE, PairReason, RETRY_WAITS};
use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;

/// What makes peers with other runners for [`Pairs`].
pub trait Dial: Send + Sync + 'static {
    type Peer: PairPeer;

    /// Makes a peer with the runner of `device`: it, once it connected, or
    /// why it did not.
    fn dial(&self, device: String) -> BoxFuture<'static, Result<Self::Peer, Option<PairReason>>>;
}

/// A peer with another runner, as [`Pairs`] uses it.
pub trait PairPeer: Clone + Send + Sync + 'static {
    type Channel: Send + 'static;

    /// A channel for a connection of `forward`; none once the peer ended.
    fn open(&self, forward: String) -> BoxFuture<'static, Option<Self::Channel>>;
    /// Resolves once the peer ended, with why, unless it was closed.
    fn ended(&self) -> BoxFuture<'static, Option<PairReason>>;
    /// Closes the peer.
    fn close(&self);
}

/// The path a connection took as it opened, with its reservation, which
/// counts it as open between its pair until it is dropped.
pub enum Path<C> {
    Direct(C, PathGuard),
    Relay(PathGuard),
}

/// A connection's place between its pair, from the moment it asks for a
/// path until it is dropped, whatever path it ends on: one reservation,
/// which a direct one that could not open moves to the relay.
pub struct PathGuard {
    device: String,
    id: u64,
    events: mpsc::UnboundedSender<Command>,
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        // Pairs that ended count nothing.
        let _ = self.events.send(Command::Ended {
            device: std::mem::take(&mut self.device),
            id: self.id,
        });
    }
}

/// What a connection that opened direct carries.
pub type Opened<D> = Path<<<D as Dial>::Peer as PairPeer>::Channel>;

/// The pairs' state, served by a task of its own until dropped.
pub struct Pairs<D: Dial> {
    commands: mpsc::UnboundedSender<Event<D::Peer>>,
    /// Where a reservation says it ended or moved to the relay, in order
    /// with the asks that made it.
    reservations: mpsc::UnboundedSender<Command>,
    next: std::sync::atomic::AtomicU64,
    reasons: watch::Receiver<Arc<HashMap<String, PairReason>>>,
    _task: AbortOnDropHandle<()>,
}

/// What a reservation says of itself.
pub(crate) enum Command {
    /// It asks for a path; the answer is a peer when it goes direct.
    Path { device: String, id: u64, allowed: bool },
    /// Its direct channel did not open: it is on the relay now.
    Relayed { device: String, id: u64 },
    /// Its connection ended, or it went before it had a path.
    Ended { device: String, id: u64 },
}

enum Event<P> {
    /// The answer to the reservation `id`'s ask.
    Answer { id: u64, answer: oneshot::Sender<Option<P>> },
    Online {
        device: String,
    },
    Dialed {
        device: String,
        result: Result<P, Option<PairReason>>,
    },
    PeerEnded {
        device: String,
        generation: u64,
        reason: Option<PairReason>,
    },
}

/// Where a pair's peer is.
enum PeerState<P> {
    None,
    Dialing,
    Connected { peer: P, generation: u64 },
}

struct Pair<P> {
    peer: PeerState<P>,
    /// The pair's reservations now, each with whether it goes direct.
    open: HashMap<u64, bool>,
    /// When the connected peer closes, with no connection left.
    idle_at: Option<Instant>,
    /// The failed attempts since the last connected peer.
    failures: usize,
    /// When the next attempt is due, while the pair is on the relay.
    retry_at: Option<Instant>,
}

impl<P> Pair<P> {
    fn new() -> Self {
        Self {
            peer: PeerState::None,
            open: HashMap::new(),
            idle_at: None,
            failures: 0,
            retry_at: None,
        }
    }

    fn direct(&self) -> usize {
        self.open.values().filter(|direct| **direct).count()
    }
}

impl<D: Dial> Pairs<D> {
    /// Serves the pairs of this runner with peers that `dial` makes.
    pub fn new(dial: D) -> Self {
        let (commands, events) = mpsc::unbounded_channel();
        let (reservations, reserved) = mpsc::unbounded_channel();
        let (reasons, watching) = watch::channel(Arc::new(HashMap::new()));
        let task = Serving {
            dial: Arc::new(dial),
            pairs: HashMap::new(),
            events,
            reserved,
            answers: HashMap::new(),
            sender: commands.clone(),
            reasons,
            generation: 0,
            dials: JoinSet::new(),
        };
        Self {
            commands,
            reservations,
            next: std::sync::atomic::AtomicU64::new(0),
            reasons: watching,
            _task: AbortOnDropHandle::new(tokio::spawn(task.run())),
        }
    }

    /// The path a new connection of `forward` with the runner of `device`
    /// takes, direct when `allowed` and the pair's peer is connected and
    /// has room. The connection counts from the ask on, so a caller that
    /// goes while it waits leaves nothing counted.
    pub async fn path(&self, device: &str, forward: &str, allowed: bool) -> Opened<D> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let guard = PathGuard {
            device: device.to_owned(),
            id,
            events: self.reservations.clone(),
        };
        let (answer, answered) = oneshot::channel();
        // The ask goes before the answer is asked for, on the channel the
        // guard's end goes on, so the task counts it before its end.
        let asked = self
            .reservations
            .send(Command::Path {
                device: device.to_owned(),
                id,
                allowed,
            })
            .is_ok()
            && self.commands.send(Event::Answer { id, answer }).is_ok();
        let peer = if asked { answered.await.ok().flatten() } else { None };
        let Some(peer) = peer else {
            return Path::Relay(guard);
        };
        match peer.open(forward.to_owned()).await {
            Some(channel) => Path::Direct(channel, guard),
            None => {
                // Pairs that ended count nothing.
                let _ = self.reservations.send(Command::Relayed {
                    device: device.to_owned(),
                    id,
                });
                Path::Relay(guard)
            }
        }
    }

    /// The runner of `device` connected again: a pair on the relay tries a
    /// peer again at once.
    pub fn online(&self, device: &str) {
        // Pairs that ended try nothing.
        let _ = self.commands.send(Event::Online {
            device: device.to_owned(),
        });
    }

    /// Why each pair on the relay is there, when a direct path failed.
    pub fn reasons(&self) -> watch::Receiver<Arc<HashMap<String, PairReason>>> {
        self.reasons.clone()
    }
}

struct Serving<D: Dial> {
    dial: Arc<D>,
    pairs: HashMap<String, Pair<D::Peer>>,
    events: mpsc::UnboundedReceiver<Event<D::Peer>>,
    reserved: mpsc::UnboundedReceiver<Command>,
    /// Each reservation's answer until its asker waits for it: the peer it
    /// goes direct on, or none.
    answers: HashMap<u64, Option<D::Peer>>,
    sender: mpsc::UnboundedSender<Event<D::Peer>>,
    reasons: watch::Sender<Arc<HashMap<String, PairReason>>>,
    generation: u64,
    /// Each dial's and each peer's watch, which end with the pairs.
    dials: JoinSet<()>,
}

impl<D: Dial> Serving<D> {
    async fn run(mut self) {
        loop {
            let next = self.next_timer();
            let event = tokio::select! {
                event = self.events.recv() => match event {
                    Some(event) => event,
                    None => break,
                },
                () = async {
                    match next {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.timers();
                    continue;
                }
                Some(command) = self.reserved.recv() => {
                    self.reservation(command);
                    continue;
                }
                Some(_) = self.dials.join_next() => continue,
            };
            self.handle(event);
        }
        for pair in self.pairs.values() {
            if let PeerState::Connected { peer, .. } = &pair.peer {
                peer.close();
            }
        }
    }

    /// The earliest idle close or due attempt.
    fn next_timer(&self) -> Option<Instant> {
        self.pairs
            .values()
            .flat_map(|pair| {
                let retry = pair.retry_at.filter(|_| !pair.open.is_empty() && matches!(pair.peer, PeerState::None));
                [pair.idle_at, retry]
            })
            .flatten()
            .min()
    }

    fn timers(&mut self) {
        let now = Instant::now();
        let mut due = Vec::new();
        for (device, pair) in &mut self.pairs {
            if pair.idle_at.is_some_and(|at| at <= now) {
                pair.idle_at = None;
                if let PeerState::Connected { peer, .. } = std::mem::replace(&mut pair.peer, PeerState::None) {
                    peer.close();
                }
            }
            let retry = pair.retry_at.is_some_and(|at| at <= now)
                && !pair.open.is_empty()
                && matches!(pair.peer, PeerState::None);
            if retry {
                due.push(device.clone());
            }
        }
        for device in due {
            self.start_dial(&device);
        }
    }

    fn handle(&mut self, event: Event<D::Peer>) {
        match event {
            Event::Answer { id, answer } => {
                // Its ask went before it on the reservations' channel, so
                // the asks and ends sent so far are taken first: the
                // reservation is decided, or it ended and has no answer.
                while let Ok(command) = self.reserved.try_recv() {
                    self.reservation(command);
                }
                if let Some(peer) = self.answers.remove(&id) {
                    // The asker may have gone; its guard counts it out.
                    let _ = answer.send(peer);
                }
            }
            Event::Online { device } => {
                let Some(pair) = self.pairs.get_mut(&device) else {
                    return;
                };
                if pair.retry_at.is_some() && matches!(pair.peer, PeerState::None) {
                    pair.retry_at = Some(Instant::now());
                    if !pair.open.is_empty() {
                        self.start_dial(&device);
                    }
                }
            }
            Event::Dialed { device, result } => {
                let Some(pair) = self.pairs.get_mut(&device) else {
                    return;
                };
                match result {
                    Ok(peer) => {
                        self.generation += 1;
                        let generation = self.generation;
                        pair.failures = 0;
                        pair.retry_at = None;
                        if pair.open.is_empty() {
                            pair.idle_at = Some(Instant::now() + PEER_IDLE);
                        }
                        let ended = peer.ended();
                        pair.peer = PeerState::Connected { peer, generation };
                        let sender = self.sender.clone();
                        self.dials.spawn(async move {
                            let reason = ended.await;
                            // Pairs that ended hear nothing.
                            let _ = sender.send(Event::PeerEnded {
                                device,
                                generation,
                                reason,
                            });
                        });
                        self.set_reason(None);
                    }
                    Err(reason) => {
                        pair.peer = PeerState::None;
                        Self::failed(pair);
                        let device_reason = reason.map(|reason| (device, reason));
                        self.set_reason(device_reason);
                    }
                }
            }
            Event::PeerEnded {
                device,
                generation,
                reason,
            } => {
                let Some(pair) = self.pairs.get_mut(&device) else {
                    return;
                };
                let current = matches!(&pair.peer, PeerState::Connected { generation: now, .. } if *now == generation);
                if !current {
                    return;
                }
                pair.peer = PeerState::None;
                pair.idle_at = None;
                // Its channels went with it; their connections end.
                for direct in pair.open.values_mut() {
                    *direct = false;
                }
                if let Some(reason) = reason {
                    Self::failed(pair);
                    self.set_reason(Some((device, reason)));
                }
            }
        }
    }

    /// What a reservation says: its ask, counted at once and answered
    /// with the pair's peer while it is connected and has room; its move
    /// to the relay; its end, after which, with none left, the peer closes
    /// a minute later.
    fn reservation(&mut self, command: Command) {
        match command {
            Command::Path { device, id, allowed } => {
                let pair = self.pairs.entry(device.clone()).or_insert_with(Pair::new);
                pair.idle_at = None;
                let peer = match &pair.peer {
                    PeerState::Connected { peer, .. } if allowed && pair.direct() < MAX_CHANNELS => Some(peer.clone()),
                    _ => None,
                };
                pair.open.insert(id, peer.is_some());
                let dial = allowed
                    && matches!(pair.peer, PeerState::None)
                    && pair.retry_at.is_none_or(|at| at <= Instant::now());
                self.answers.insert(id, peer);
                if dial {
                    self.start_dial(&device);
                }
            }
            Command::Relayed { device, id } => {
                if let Some(direct) = self.pairs.get_mut(&device).and_then(|pair| pair.open.get_mut(&id)) {
                    *direct = false;
                }
            }
            Command::Ended { device, id } => {
                self.answers.remove(&id);
                let Some(pair) = self.pairs.get_mut(&device) else {
                    return;
                };
                pair.open.remove(&id);
                if pair.open.is_empty() && matches!(pair.peer, PeerState::Connected { .. }) {
                    pair.idle_at = Some(Instant::now() + PEER_IDLE);
                }
            }
        }
    }

    /// A pair whose attempt failed waits for its next.
    fn failed(pair: &mut Pair<D::Peer>) {
        let wait = RETRY_WAITS[pair.failures.min(RETRY_WAITS.len() - 1)];
        pair.failures += 1;
        pair.retry_at = Some(Instant::now() + wait);
    }

    /// Records why a pair is on the relay, or clears every reason of the
    /// pairs whose peer connected.
    fn set_reason(&mut self, reason: Option<(String, PairReason)>) {
        let pairs = &self.pairs;
        self.reasons.send_if_modified(|reasons| {
            let mut next: HashMap<String, PairReason> = reasons
                .iter()
                .filter(|(device, _)| {
                    pairs
                        .get(*device)
                        .is_some_and(|pair| !matches!(pair.peer, PeerState::Connected { .. }))
                })
                .map(|(device, reason)| (device.clone(), *reason))
                .collect();
            if let Some((device, reason)) = reason {
                next.insert(device, reason);
            }
            if next == **reasons {
                return false;
            }
            *reasons = Arc::new(next);
            true
        });
    }

    fn start_dial(&mut self, device: &str) {
        let Some(pair) = self.pairs.get_mut(device) else {
            return;
        };
        if !matches!(pair.peer, PeerState::None) {
            return;
        }
        pair.peer = PeerState::Dialing;
        pair.retry_at = None;
        let dialing = self.dial.dial(device.to_owned());
        let sender = self.sender.clone();
        let device = device.to_owned();
        self.dials.spawn(async move {
            let result = dialing.await;
            // Pairs that ended hear nothing.
            let _ = sender.send(Event::Dialed { device, result });
        });
    }
}
