//! This runner's pairs with the other runners of the user, for as long as its
//! connection to the backend lasts (`crates/runner-direct/src/pairs.rs`): the
//! one peer per pair, whichever runner offered it, which the user's forwards
//! (`crate::forward`) and the pipes between two devices
//! (`crates/runner-process/src/pipes/moves.rs`) open channels on.
//!
//! - This runner offers a peer through the backend, which introduces it to the
//!   other runner ([`RunnerDial`]).
//! - An offer of the other runner's, which the connection's owner answers,
//!   becomes the pair's peer ([`Pairing::answers`], [`Pairing::adopt`]).
//! - A pipe channel the other runner opens waits here for the pipe's reader,
//!   for as long as a peer takes to connect, and closes when none takes it.
//! - A peer this runner measures probes the relay through the backend
//!   (`pair_ping`), whose answers come back here by the peer's id
//!   (`pair_pong`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use bytes::Bytes;
use demi_runner_direct::measure::{self, ProbedPath};
use demi_runner_direct::{Dial, Direct, PairChannel, PairPeer, Pairs, PathGuard, PeerLink, Received};
use demi_runner_process::pipes::{Carried, PairPlace, PipeChannel, PipePairs};
use demi_runner_protocol::direct::{CONNECT_TIMEOUT, DialRefusal, StunUrl};
use demi_runner_protocol::pairs::{PairHeader, PairReason, PairRoute};
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

/// What the backend answers to a request of the runner's, by its id.
#[derive(Clone, Default)]
pub(crate) struct Replies(Arc<Mutex<HashMap<String, oneshot::Sender<Inbound>>>>);

impl Replies {
    pub(crate) fn wait(&self, id: &str) -> oneshot::Receiver<Inbound> {
        let (answer, answered) = oneshot::channel();
        self.0.lock().expect("the replies are whole").insert(id.to_owned(), answer);
        answered
    }

    /// Stops waiting for the reply to `id`.
    pub(crate) fn forget(&self, id: &str) {
        self.0.lock().expect("the replies are whole").remove(id);
    }

    /// Hands `reply` to what waits for it; one nobody waits for is passed
    /// over.
    pub(crate) fn answer(&self, id: &str, reply: Inbound) {
        if let Some(waiting) = self.0.lock().expect("the replies are whole").remove(id) {
            // The connection that asked may have gone.
            let _ = waiting.send(reply);
        }
    }
}

/// The answers to this runner's probes of the relay, by the peer each
/// measures.
#[derive(Clone, Default)]
pub(crate) struct Pongs(Arc<Mutex<HashMap<String, mpsc::UnboundedSender<u32>>>>);

impl Pongs {
    /// The answers of peer `peer`'s relay probes from now on, until the
    /// listening is dropped.
    fn listen(&self, peer: &str) -> Listening {
        let (answers, answered) = mpsc::unbounded_channel();
        self.0.lock().expect("the pongs are whole").insert(peer.to_owned(), answers);
        Listening {
            pongs: self.clone(),
            peer: peer.to_owned(),
            answers: answered,
        }
    }

    fn answer(&self, peer: &str, id: u32) {
        if let Some(answers) = self.0.lock().expect("the pongs are whole").get(peer) {
            // A measurement that ended meanwhile takes no more.
            let _ = answers.send(id);
        }
    }
}

/// The answers to one peer's relay probes, which stop when it is dropped.
struct Listening {
    pongs: Pongs,
    peer: String,
    answers: mpsc::UnboundedReceiver<u32>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.pongs.0.lock().expect("the pongs are whole").remove(&self.peer);
    }
}

/// The relay as the path a measurement probes: each probe goes to the
/// other runner of peer `peer` through the backend.
struct RelayProbes {
    backend: Backend,
    peer: String,
}

impl ProbedPath for RelayProbes {
    fn send(&self, id: u32) {
        // A connection that ended answers nothing, which the relay's
        // figures then lack.
        self.backend.send(&Outbound::PairPing { peer: self.peer.clone(), id });
    }
}

/// The connection's way to the backend.
#[derive(Clone)]
pub(crate) struct Backend {
    pub(crate) control: mpsc::UnboundedSender<wire::Frame>,
    pub(crate) closed: CancellationToken,
}

impl Backend {
    /// Sends `message`; false once the connection's writer ended.
    pub(crate) fn send(&self, message: &Outbound) -> bool {
        let frame = match wire::encode(message) {
            Ok(frame) => frame,
            Err(error) => {
                // The runner's own messages always encode; one that does
                // not is not sent, and its connection is refused.
                tracing::warn!(event = "reply.encode_failed", what = "pair", %error, "a pair's message was not encoded");
                return true;
            }
        };
        self.control.send(frame).is_ok()
    }
}

/// The pipe channels the other runners opened, waiting for their readers.
#[derive(Clone, Default)]
pub struct InboundPipes(Arc<Mutex<Meetings>>);

#[derive(Default)]
struct Meetings {
    /// Each pipe's channel or reader, by the source's device and the pipe.
    waiting: HashMap<(String, String), Meeting>,
    /// The number the next waiting channel takes.
    next: u64,
}

/// Where a pipe's channel and its reader meet.
enum Meeting {
    /// The reader waits for the channel from the pipe's byte `at` on.
    Reader { at: u64, reader: oneshot::Sender<RunnerPipeChannel> },
    /// The channel waits, under its number.
    Channel(RunnerPipeChannel, u64),
}

impl InboundPipes {
    /// The channel the runner of `from` opened for pipe `pipe` from byte
    /// `at`: its reader takes it, or it waits for one for as long as a peer
    /// takes to connect, and then closes.
    pub fn arrived(&self, from: String, pipe: String, at: u64, channel: PairChannel, runtime: &tokio::runtime::Handle) {
        let channel = RunnerPipeChannel::new(channel, at);
        let key = (from, pipe);
        let mut meetings = self.0.lock().expect("the inbound pipes are whole");
        let channel = match meetings.waiting.remove(&key) {
            Some(Meeting::Reader { at: wanted, reader }) if wanted == at => match reader.send(channel) {
                Ok(()) => return,
                Err(channel) => channel,
            },
            // A channel from before the reader's move, which its source
            // left, closes as it is dropped.
            Some(Meeting::Reader { at: wanted, reader }) if at < wanted => {
                meetings.waiting.insert(key, Meeting::Reader { at: wanted, reader });
                return;
            }
            // A later channel for one pipe replaces what waited, which its
            // source or its reader left.
            Some(_) | None => channel,
        };
        meetings.next += 1;
        let number = meetings.next;
        meetings.waiting.insert(key.clone(), Meeting::Channel(channel, number));
        let inbound = self.clone();
        runtime.spawn(async move {
            tokio::time::sleep(CONNECT_TIMEOUT).await;
            let mut meetings = inbound.0.lock().expect("the inbound pipes are whole");
            if matches!(meetings.waiting.get(&key), Some(Meeting::Channel(_, waiting)) if *waiting == number) {
                // Its channel closes as it is dropped.
                meetings.waiting.remove(&key);
            }
        });
    }

    /// The channel the runner of `from` opens for pipe `pipe`'s bytes from
    /// `at` on, once it arrives.
    fn accept(&self, from: String, pipe: String, at: u64) -> oneshot::Receiver<RunnerPipeChannel> {
        let (reader, accepted) = oneshot::channel();
        let key = (from, pipe);
        let mut meetings = self.0.lock().expect("the inbound pipes are whole");
        match meetings.waiting.remove(&key) {
            Some(Meeting::Channel(channel, _)) if channel.at == at => {
                // The reader waits on `accepted`, which it holds.
                let _ = reader.send(channel);
            }
            // A channel from an earlier move closes as it is dropped.
            Some(_) | None => {
                meetings.waiting.insert(key, Meeting::Reader { at, reader });
            }
        }
        accepted
    }

    /// Drops every waiting channel and reader: the connection ended.
    fn clear(&self) {
        self.0.lock().expect("the inbound pipes are whole").waiting.clear();
    }
}

/// This connection's pairs: the peers, the STUN servers their offers ask,
/// the backend's answers and the pipe channels waiting for their readers.
pub struct Pairing {
    pairs: Arc<Pairs<RunnerDial>>,
    stun: watch::Sender<Vec<StunUrl>>,
    pub(crate) replies: Replies,
    pub(crate) backend: Backend,
    inbound: InboundPipes,
    direct: Direct,
    pongs: Pongs,
}

impl Pairing {
    /// The pairs of the connection whose messages go to `control` until
    /// `closed`, with `direct`'s peers, this runner's device as `own` names
    /// it once the backend said it, and `inbound` for the pipe channels the
    /// other runners open.
    pub fn new(
        direct: Direct,
        own: Arc<OnceLock<String>>,
        inbound: InboundPipes,
        control: mpsc::UnboundedSender<wire::Frame>,
        closed: CancellationToken,
    ) -> Arc<Self> {
        let replies = Replies::default();
        let backend = Backend { control, closed };
        let (stun, stunned) = watch::channel(Vec::new());
        let pongs = Pongs::default();
        let pairs = Pairs::new(
            RunnerDial {
                direct: direct.clone(),
                backend: backend.clone(),
                replies: replies.clone(),
                stun: stunned,
                pongs: pongs.clone(),
            },
            own,
        );
        Arc::new(Self {
            pairs: Arc::new(pairs),
            stun,
            replies,
            backend,
            inbound,
            direct,
            pongs,
        })
    }

    pub(crate) fn pairs(&self) -> &Arc<Pairs<RunnerDial>> {
        &self.pairs
    }

    /// The STUN servers the backend names, which this runner's offers ask.
    pub fn stun(&self, stun: Vec<StunUrl>) {
        self.stun.send_replace(stun);
    }

    /// Whether this runner answers the runner of `device`'s offer of a peer.
    pub async fn answers(&self, device: &str) -> bool {
        self.pairs.answers(device).await
    }

    /// Takes the peer `peer`, which this runner answered for the runner of
    /// `device`, as the pair's.
    pub fn adopt(&self, device: &str, route: PairRoute, peer: String, link: PeerLink) {
        self.pairs.adopt(
            device,
            route,
            RunnerPeer {
                link,
                peer,
                direct: self.direct.clone(),
                backend: self.backend.clone(),
                pongs: self.pongs.clone(),
            },
        );
    }

    /// A message of the backend's for this runner's offers: whether it was
    /// one.
    pub fn handle(&self, message: Inbound) -> Result<(), Inbound> {
        match message {
            Inbound::DirectDialed { ref id, .. } | Inbound::DirectDialRefused { ref id, .. } => {
                let id = id.clone();
                self.replies.answer(&id, message);
                Ok(())
            }
            Inbound::PairPong { peer, id } => {
                self.pongs.answer(&peer, id);
                Ok(())
            }
            other => Err(other),
        }
    }
}

impl Drop for Pairing {
    fn drop(&mut self) {
        self.inbound.clear();
    }
}

impl PipePairs for Pairing {
    fn relay(&self, device: String, route: PairRoute) -> BoxFuture<'static, Box<dyn PairPlace>> {
        let pairs = self.pairs.clone();
        Box::pin(async move {
            let guard = pairs.relay(&device, route).await;
            Box::new(RunnerPlace(Arc::new(Place {
                pairs,
                guard,
                peer: Mutex::new(None),
            }))) as Box<dyn PairPlace>
        })
    }

    fn accept(&self, device: String, pipe: String, at: u64) -> BoxFuture<'static, Option<Box<dyn PipeChannel>>> {
        let accepted = self.inbound.accept(device, pipe, at);
        Box::pin(async move {
            accepted
                .await
                .ok()
                .map(|channel| Box::new(channel) as Box<dyn PipeChannel>)
        })
    }
}

/// A pipe's place among its pair's connections.
struct RunnerPlace(Arc<Place>);

struct Place {
    pairs: Arc<Pairs<RunnerDial>>,
    guard: PathGuard,
    /// The peer the pipe's next channel opens on, once it had room.
    peer: Mutex<Option<RunnerPeer>>,
}

impl PairPlace for RunnerPlace {
    fn ready(&self) -> BoxFuture<'static, bool> {
        let place = self.0.clone();
        Box::pin(async move {
            let Some(peer) = place.pairs.direct(&place.guard).await else {
                return false;
            };
            *place.peer.lock().expect("the place is whole") = Some(peer);
            true
        })
    }

    fn open(&self, pipe: String, at: u64) -> BoxFuture<'_, Option<Box<dyn PipeChannel>>> {
        Box::pin(async move {
            let peer = self.0.peer.lock().expect("the place is whole").take();
            let opening = async { peer?.open(PairHeader::Pipe { pipe, at }).await };
            // A peer that stopped answering holds the pipe's bytes no longer
            // than a peer takes to connect.
            match tokio::time::timeout(CONNECT_TIMEOUT, opening).await {
                Ok(Some(channel)) => Some(Box::new(RunnerPipeChannel::new(channel, at)) as Box<dyn PipeChannel>),
                _ => {
                    self.0.guard.relayed();
                    None
                }
            }
        })
    }

    fn relayed(&self) {
        self.0.guard.relayed();
    }
}

/// A pair's channel that carries a pipe's bytes.
struct RunnerPipeChannel {
    channel: PairChannel,
    at: u64,
}

impl RunnerPipeChannel {
    fn new(channel: PairChannel, at: u64) -> Self {
        Self { channel, at }
    }
}

impl PipeChannel for RunnerPipeChannel {
    fn at(&self) -> u64 {
        self.at
    }

    fn send(&self, bytes: Bytes) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.channel.send(bytes).await.is_ok() })
    }

    fn end(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.channel.end().await.is_ok() })
    }

    fn granted(&self) -> watch::Receiver<u64> {
        self.channel.granted()
    }

    fn gone(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.channel.gone())
    }

    fn receive(&mut self) -> BoxFuture<'_, Option<Carried>> {
        Box::pin(async move {
            match self.channel.receive().await? {
                Received::Bytes(bytes) => Some(Carried::Bytes(bytes)),
                Received::End => Some(Carried::End),
            }
        })
    }

    fn took(&self, bytes: usize) {
        self.channel.took(bytes);
    }
}

/// Makes peers with the user's other runners, introduced by the backend,
/// this runner offering.
pub(crate) struct RunnerDial {
    direct: Direct,
    backend: Backend,
    replies: Replies,
    stun: watch::Receiver<Vec<StunUrl>>,
    pongs: Pongs,
}

/// A peer with another runner, whichever offered it.
#[derive(Clone)]
pub(crate) struct RunnerPeer {
    link: PeerLink,
    peer: String,
    direct: Direct,
    backend: Backend,
    pongs: Pongs,
}

impl PairPeer for RunnerPeer {
    type Channel = PairChannel;

    fn open(&self, header: PairHeader) -> BoxFuture<'static, Option<PairChannel>> {
        let link = self.link.clone();
        Box::pin(async move { link.open(&header).await })
    }

    fn connected(&self) -> BoxFuture<'static, Result<(), Option<PairReason>>> {
        let link = self.link.clone();
        Box::pin(async move { link.connected().await })
    }

    fn ended(&self) -> BoxFuture<'static, Option<PairReason>> {
        let link = self.link.clone();
        Box::pin(async move { link.ended().await })
    }

    fn carries(&self) -> bool {
        self.link.carries()
    }

    fn close(&self) {
        self.direct.close(&self.peer);
    }

    fn measure(&self) -> BoxFuture<'static, bool> {
        let link = self.link.clone();
        let relay = RelayProbes {
            backend: self.backend.clone(),
            peer: self.peer.clone(),
        };
        let mut relay_answers = self.pongs.listen(&self.peer);
        Box::pin(async move {
            let (direct, mut direct_answers) = link.probes();
            let (direct, relay) = measure::measure(&direct, &mut direct_answers, &relay, &mut relay_answers.answers).await;
            let slower = measure::direct_worse(direct, relay);
            tracing::info!(
                event = "direct_path.measured",
                direct_lost = direct.lost,
                direct_ms = direct.latency.map(|latency| latency.as_millis() as u64),
                relay_ms = relay.latency.map(|latency| latency.as_millis() as u64),
                slower,
                "a pair's peer was measured against the relay"
            );
            slower
        })
    }

    fn tell(&self, slower: bool) -> BoxFuture<'static, ()> {
        let link = self.link.clone();
        Box::pin(async move { link.tell(slower).await })
    }

    fn verdict(&self) -> BoxFuture<'static, Option<bool>> {
        let link = self.link.clone();
        Box::pin(async move { link.verdict().await })
    }
}

impl Dial for RunnerDial {
    type Peer = RunnerPeer;

    fn dial(&self, device: String) -> BoxFuture<'static, Result<RunnerPeer, Option<PairReason>>> {
        let direct = self.direct.clone();
        let backend = self.backend.clone();
        let replies = self.replies.clone();
        let stun = self.stun.borrow().clone();
        let pongs = self.pongs.clone();
        Box::pin(async move {
            let peer = uuid::Uuid::new_v4().simple().to_string();
            let dialing = direct
                .dial(peer.clone(), device.clone(), stun)
                .await
                .map_err(|_| Some(PairReason::Busy))?;
            let id = uuid::Uuid::new_v4().simple().to_string();
            let answered = replies.wait(&id);
            let offer = Outbound::DirectDial {
                id,
                peer: peer.clone(),
                device,
                sdp: dialing.sdp,
            };
            if !backend.send(&offer) {
                direct.close(&peer);
                return Err(None);
            }
            let reason = match answered.await {
                Ok(Inbound::DirectDialed { sdp, .. }) => {
                    direct.answered(&peer, sdp);
                    None
                }
                Ok(Inbound::DirectDialRefused { code, message, .. }) => {
                    tracing::debug!(event = "pair.introduction_refused", code = ?code, reason = %message, "the backend did not introduce a pair");
                    Some(match code {
                        DialRefusal::Busy => Some(PairReason::Busy),
                        DialRefusal::InvalidOffer | DialRefusal::Unavailable => None,
                    })
                }
                _ => Some(None),
            };
            if let Some(reason) = reason {
                direct.close(&peer);
                return Err(reason);
            }
            // The candidates this side finds after its offer go to the
            // other runner, and the peer's end to the backend, which
            // closes the other side.
            let link = dialing.link;
            let mut candidates = dialing.candidates;
            let told = backend.clone();
            let ended = link.clone();
            let ending = peer.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        found = candidates.recv() => match found {
                            Some(candidate) => {
                                let found = Outbound::DirectCandidate { peer: ending.clone(), candidate };
                                if !told.send(&found) {
                                    return;
                                }
                            }
                            None => break,
                        },
                        _ = ended.ended() => break,
                    }
                }
                ended.ended().await;
                told.send(&Outbound::DirectClosed { peer: ending });
            });
            match link.connected().await {
                Ok(()) => Ok(RunnerPeer {
                    link,
                    peer,
                    direct,
                    backend,
                    pongs,
                }),
                Err(reason) => {
                    direct.close(&peer);
                    Err(reason)
                }
            }
        })
    }
}
