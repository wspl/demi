//! The user's port forwards on this runner
//! (`crates/backend-user-shard/src/shard/port_forwards.rs`), for as long as its
//! connection to the backend lasts: the backend tells it every forward it is an
//! end of as the connection begins and whenever one changes.
//!
//! - As a forward's Listen On, the runner listens on `127.0.0.1` only,
//!   announces whether it listens and its connections on each path as they
//!   change, and carries each connection direct, as a channel of the pair's
//!   peer, or through the relay, as a pair of pipes.
//! - As a Connect To, it opens the connection to the forward's address for each
//!   connection the other runner's channel or the backend's pipes bring.
//!
//! Each forward this runner is an end of has a generation: what ends its
//! listener and every connection it carries, here and on the other runner. A
//! forward turned off, removed, or changed where it listens or connects ends
//! its generation. Its open connections end, and new ones follow the change.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use demi_runner_direct::{Dial, Direct, ForwardChannel, PairPeer, Pairs, Path, PeerLink, connect_target};
use demi_runner_process::pipes::{PipeClient, report_pipe};
use demi_runner_protocol::direct::{DialRefusal, StunUrl};
use demi_runner_protocol::forward::{DEADLINE, Forward, ForwardRunState, PairReason};
use demi_runner_protocol::wire::{self, Inbound, Outbound, PipeRef};
use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

/// A forward this runner is an end of, as the backend last said, with what
/// ends its generation.
#[derive(Clone)]
pub struct Listed {
    pub forward: Forward,
    /// Which generation of the forward this is.
    generation: u64,
    pub ended: CancellationToken,
}

impl Listed {
    /// What, changed, makes the forward a new generation: where it listens
    /// and where it connects; a new name or the other end's presence or
    /// route do not.
    fn same_generation(&self, forward: &Forward) -> bool {
        self.forward.listen == forward.listen
            && self.forward.connect == forward.connect
            && self.forward.peer == forward.peer
    }
}

/// The forwards this runner is an end of, which its peers' channels read as
/// the Connect To.
pub type ForwardList = watch::Receiver<Arc<Vec<Listed>>>;

/// What the backend answers to a request of the runner's, by its id.
#[derive(Clone, Default)]
struct Replies(Arc<Mutex<HashMap<String, oneshot::Sender<Inbound>>>>);

impl Replies {
    fn wait(&self, id: &str) -> oneshot::Receiver<Inbound> {
        let (answer, answered) = oneshot::channel();
        self.0.lock().expect("the replies are whole").insert(id.to_owned(), answer);
        answered
    }

    /// Stops waiting for the reply to `id`.
    fn forget(&self, id: &str) {
        self.0.lock().expect("the replies are whole").remove(id);
    }

    /// Hands `reply` to what waits for it; one nobody waits for is passed
    /// over.
    fn answer(&self, id: &str, reply: Inbound) {
        if let Some(waiting) = self.0.lock().expect("the replies are whole").remove(id) {
            // The connection that asked may have gone.
            let _ = waiting.send(reply);
        }
    }
}

/// The connection's way to the backend.
#[derive(Clone)]
struct Backend {
    control: mpsc::Sender<wire::Frame>,
    closed: CancellationToken,
}

impl Backend {
    /// Sends `message`; false once the connection closed.
    async fn send(&self, message: &Outbound) -> bool {
        let frame = match wire::encode(message) {
            Ok(frame) => frame,
            Err(error) => {
                // The runner's own messages always encode; one that does
                // not is not sent, and its connection is refused.
                tracing::warn!("a forward's message could not be encoded: {error}");
                return true;
            }
        };
        tokio::select! {
            () = self.closed.cancelled() => false,
            sent = self.control.send(frame) => sent.is_ok(),
        }
    }
}

/// The runner's forwards on one connection; dropping it ends every
/// listener and connection.
pub struct Forwarding {
    events: mpsc::UnboundedSender<Event>,
    replies: Replies,
    _task: AbortOnDropHandle<()>,
}

enum Event {
    Forwards { forwards: Vec<Forward>, stun: Vec<StunUrl> },
    Retry(String),
    Connect { forward: String, input: PipeRef, output: PipeRef },
    Counted { forward: String, direct: bool, opened: bool },
}

/// Where the backend's list of forwards goes, and where the peers'
/// channels read it.
pub fn forward_list() -> (watch::Sender<Arc<Vec<Listed>>>, ForwardList) {
    watch::channel(Arc::new(Vec::new()))
}

impl Forwarding {
    /// The forwards of the connection whose messages go to `control` until
    /// `closed`, with `direct`'s peers and `pipes` for the relay; what the
    /// backend lists goes to `listed`.
    pub fn new(
        listed: watch::Sender<Arc<Vec<Listed>>>,
        direct: Direct,
        pipes: PipeClient,
        control: mpsc::Sender<wire::Frame>,
        closed: CancellationToken,
    ) -> Self {
        let (events, received) = mpsc::unbounded_channel();
        let replies = Replies::default();
        let backend = Backend { control, closed };
        let (stun, stunned) = watch::channel(Vec::new());
        let pairs = Pairs::new(RunnerDial {
            direct,
            backend: backend.clone(),
            replies: replies.clone(),
            stun: stunned,
        });
        let serving = Serving {
            events: received,
            sender: events.clone(),
            listed,
            stun,
            listeners: HashMap::new(),
            pairs: Arc::new(pairs),
            backend,
            replies: replies.clone(),
            pipes,
            tasks: JoinSet::new(),
            generations: 0,
        };
        Self {
            events,
            replies,
            _task: AbortOnDropHandle::new(tokio::spawn(serving.run())),
        }
    }

    /// Takes a message of the backend's about forwards: whether it was one.
    pub fn handle(&self, message: Inbound) -> Result<(), Inbound> {
        let event = match message {
            Inbound::Forwards { forwards, stun } => Event::Forwards { forwards, stun },
            Inbound::ForwardRetry { forward } => Event::Retry(forward),
            Inbound::ForwardConnect {
                forward,
                input,
                output,
            } => Event::Connect {
                forward,
                input,
                output,
            },
            Inbound::ForwardOpened { ref id, .. }
            | Inbound::ForwardRefused { ref id, .. }
            | Inbound::DirectDialed { ref id, .. }
            | Inbound::DirectDialRefused { ref id, .. } => {
                let id = id.clone();
                self.replies.answer(&id, message);
                return Ok(());
            }
            other => return Err(other),
        };
        // A forwarding that ended takes nothing more.
        let _ = self.events.send(event);
        Ok(())
    }
}

/// A forward this runner listens for.
struct Listener {
    port: u16,
    /// Its forward's generation, which it belongs to, and what ends it.
    generation: u64,
    ended: CancellationToken,
    /// Ends its listening and its connections: a child of its generation.
    cancel: CancellationToken,
    /// What it announced last; none while it does not listen.
    state: Option<ForwardRunState>,
    direct: u32,
    relay: u32,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct Serving {
    events: mpsc::UnboundedReceiver<Event>,
    sender: mpsc::UnboundedSender<Event>,
    listed: watch::Sender<Arc<Vec<Listed>>>,
    stun: watch::Sender<Vec<StunUrl>>,
    listeners: HashMap<String, Listener>,
    pairs: Arc<Pairs<RunnerDial>>,
    backend: Backend,
    replies: Replies,
    pipes: PipeClient,
    /// The relayed connections this runner carries as a Connect To, which
    /// end with it.
    tasks: JoinSet<()>,
    /// The last generation given to a forward.
    generations: u64,
}

impl Drop for Serving {
    /// The connection to the backend ended: every forward's generation
    /// ends with it, its connections on every path included.
    fn drop(&mut self) {
        for listed in self.listed.borrow().iter() {
            listed.ended.cancel();
        }
    }
}

impl Serving {
    async fn run(mut self) {
        let mut reasons = self.pairs.reasons();
        loop {
            tokio::select! {
                event = self.events.recv() => match event {
                    Some(event) => self.handle(event).await,
                    None => break,
                },
                Ok(()) = reasons.changed() => self.announce_all().await,
                Some(_) = self.tasks.join_next() => {}
            }
        }
    }

    async fn handle(&mut self, event: Event) {
        match event {
            Event::Forwards { forwards, stun } => self.forwards(forwards, stun).await,
            Event::Retry(forward) => {
                let in_use = self
                    .listeners
                    .get(&forward)
                    .is_some_and(|listener| listener.state == Some(ForwardRunState::PortInUse));
                if in_use {
                    self.listen(&forward).await;
                }
            }
            Event::Connect {
                forward,
                input,
                output,
            } => {
                let target = self
                    .listed
                    .borrow()
                    .iter()
                    .find(|listed| listed.forward.id == forward)
                    .and_then(|listed| Some((listed.forward.connect.clone()?, listed.ended.clone())));
                let pipes = self.pipes.clone();
                let backend = self.backend.clone();
                self.tasks.spawn(connect_relayed(target, input, output, pipes, backend));
            }
            Event::Counted {
                forward,
                direct,
                opened,
            } => {
                let Some(listener) = self.listeners.get_mut(&forward) else {
                    return;
                };
                let count = if direct {
                    &mut listener.direct
                } else {
                    &mut listener.relay
                };
                *count = if opened { *count + 1 } else { count.saturating_sub(1) };
                self.announce(&forward).await;
            }
        }
    }

    /// The backend's new list: a forward that went, or changed where it
    /// listens or connects, ends its generation, with its listener and its
    /// connections; a new one, or a new generation, listens; and the pairs
    /// whose other runner came online try a peer again.
    async fn forwards(&mut self, forwards: Vec<Forward>, stun: Vec<StunUrl>) {
        self.stun.send_replace(stun);
        let before = self.listed.borrow().clone();
        let mut listed = Vec::with_capacity(forwards.len());
        for forward in forwards {
            let was = before.iter().find(|listed| listed.forward.id == forward.id);
            if was.is_some_and(|was| !was.forward.peer_online) && forward.peer_online {
                self.pairs.online(&forward.peer);
            }
            let (generation, ended) = match was {
                Some(was) if was.same_generation(&forward) => (was.generation, was.ended.clone()),
                _ => {
                    self.generations += 1;
                    (self.generations, CancellationToken::new())
                }
            };
            listed.push(Listed { forward, generation, ended });
        }
        for was in before.iter() {
            let kept = listed
                .iter()
                .any(|listed| listed.forward.id == was.forward.id && listed.generation == was.generation);
            if !kept {
                was.ended.cancel();
            }
        }
        self.listed.send_replace(Arc::new(listed.clone()));
        let stopped: Vec<String> = self
            .listeners
            .iter()
            .filter(|(id, listener)| {
                !listed
                    .iter()
                    .any(|listed| listed.forward.id == **id && listed.generation == listener.generation)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stopped {
            if let Some(listener) = self.listeners.remove(&id) {
                listener.cancel.cancel();
                if listener.state.is_some() {
                    self.backend
                        .send(&Outbound::ForwardState {
                            forward: id,
                            state: None,
                        })
                        .await;
                }
            }
        }
        for entry in &listed {
            let Some(port) = entry.forward.listen else {
                continue;
            };
            if !self.listeners.contains_key(&entry.forward.id) {
                self.listeners.insert(
                    entry.forward.id.clone(),
                    Listener {
                        port,
                        generation: entry.generation,
                        ended: entry.ended.clone(),
                        cancel: CancellationToken::new(),
                        state: None,
                        direct: 0,
                        relay: 0,
                    },
                );
                self.listen(&entry.forward.id).await;
            }
        }
        // A pair's route or the presence of its other end may have changed
        // what is said of it.
        self.announce_all().await;
    }

    /// Listens for `forward` on `127.0.0.1` at its port, or says the port
    /// is in use.
    async fn listen(&mut self, forward: &str) {
        let Some(listener) = self.listeners.get_mut(forward) else {
            return;
        };
        listener.cancel.cancel();
        listener.cancel = listener.ended.child_token();
        let bound = TcpListener::bind((Ipv4Addr::LOCALHOST, listener.port)).await;
        match bound {
            Ok(bound) => {
                let accepting = Accepting {
                    forward: forward.to_owned(),
                    listed: self.listed.subscribe(),
                    pairs: self.pairs.clone(),
                    backend: self.backend.clone(),
                    replies: self.replies.clone(),
                    pipes: self.pipes.clone(),
                    counted: self.sender.clone(),
                    cancel: listener.cancel.clone(),
                };
                tokio::spawn(accepting.run(bound));
                listener.state = Some(listening(0, 0, None));
            }
            Err(error) => {
                tracing::info!("forward {forward} cannot listen on port {}: {error}", listener.port);
                listener.state = Some(ForwardRunState::PortInUse);
            }
        }
        let state = listener.state.clone();
        self.backend
            .send(&Outbound::ForwardState {
                forward: forward.to_owned(),
                state,
            })
            .await;
    }

    /// Announces each listening forward's state that changed, such as a
    /// pair's reason.
    async fn announce_all(&mut self) {
        let ids: Vec<String> = self.listeners.keys().cloned().collect();
        for id in ids {
            self.announce(&id).await;
        }
    }

    /// Announces `forward`'s state when it changed.
    async fn announce(&mut self, forward: &str) {
        let reasons = self.pairs.reasons().borrow().clone();
        let peer = self
            .listed
            .borrow()
            .iter()
            .find(|listed| listed.forward.id == forward)
            .map(|listed| listed.forward.peer.clone());
        let Some(listener) = self.listeners.get_mut(forward) else {
            return;
        };
        if !matches!(listener.state, Some(ForwardRunState::Listening { .. })) {
            return;
        }
        let reason = peer.and_then(|peer| reasons.get(&peer).copied());
        let state = Some(listening(listener.direct, listener.relay, reason));
        if listener.state == state {
            return;
        }
        listener.state = state.clone();
        self.backend
            .send(&Outbound::ForwardState {
                forward: forward.to_owned(),
                state,
            })
            .await;
    }
}

fn listening(direct: u32, relay: u32, reason: Option<PairReason>) -> ForwardRunState {
    ForwardRunState::Listening {
        direct,
        relay,
        reason,
    }
}

/// One forward's listener and the connections it accepts.
struct Accepting {
    forward: String,
    listed: ForwardList,
    pairs: Arc<Pairs<RunnerDial>>,
    backend: Backend,
    replies: Replies,
    pipes: PipeClient,
    counted: mpsc::UnboundedSender<Event>,
    cancel: CancellationToken,
}

/// Counts a connection open on one path until dropped.
struct Counted {
    counted: mpsc::UnboundedSender<Event>,
    forward: String,
    direct: bool,
}

impl Counted {
    fn new(accepting: &Accepting, direct: bool) -> Self {
        // A forwarding that ended counts nothing.
        let _ = accepting.counted.send(Event::Counted {
            forward: accepting.forward.clone(),
            direct,
            opened: true,
        });
        Self {
            counted: accepting.counted.clone(),
            forward: accepting.forward.clone(),
            direct,
        }
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        // A forwarding that ended counts nothing.
        let _ = self.counted.send(Event::Counted {
            forward: std::mem::take(&mut self.forward),
            direct: self.direct,
            opened: false,
        });
    }
}

impl Accepting {
    /// Accepts connections until its listener's end, which ends every
    /// connection it carries.
    async fn run(self, listener: TcpListener) {
        let accepting = Arc::new(self);
        let mut connections = JoinSet::new();
        loop {
            let accepted = tokio::select! {
                () = accepting.cancel.cancelled() => return,
                Some(_) = connections.join_next() => continue,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((tcp, _)) => {
                    let connection = accepting.clone();
                    connections.spawn(async move {
                        let cancel = connection.cancel.clone();
                        tokio::select! {
                            () = cancel.cancelled() => {}
                            () = connection.carry(tcp) => {}
                        }
                    });
                }
                // A connection that went before it was taken, or a lack of
                // descriptors: the listener goes on.
                Err(error) => tracing::debug!("forward {} did not accept: {error}", accepting.forward),
            }
        }
    }

    /// Carries one connection on the path it takes as it opens, until it
    /// ends; dropped, it ends the connection on every path.
    async fn carry(&self, tcp: TcpStream) {
        let listed = self
            .listed
            .borrow()
            .iter()
            .find(|listed| listed.forward.id == self.forward)
            .cloned();
        let Some(Listed { forward, ended, .. }) = listed else {
            return;
        };
        // A forward between a Host and itself joins its connections here.
        if let Some(address) = &forward.connect {
            let Ok(mut target) = connect_target(address, &ended).await else {
                return;
            };
            let _counted = Counted::new(self, true);
            let mut tcp = tcp;
            // Either side's end or failure ends the connection.
            let _ = tokio::io::copy_bidirectional(&mut tcp, &mut target).await;
            return;
        }
        // An offline or stopped Connect To refuses at once; its Cloud is
        // not woken.
        if !forward.peer_online {
            return;
        }
        match self.pairs.path(&forward.peer, &forward.id, forward.direct).await {
            Path::Direct(channel, _reserved) => {
                let _counted = Counted::new(self, true);
                channel.splice(tcp).await;
            }
            Path::Relay(_reserved) => {
                let id = uuid::Uuid::new_v4().simple().to_string();
                let answered = self.replies.wait(&id);
                let open = Outbound::ForwardOpen {
                    id: id.clone(),
                    forward: forward.id.clone(),
                };
                if !self.backend.send(&open).await {
                    return;
                }
                let opened = tokio::time::timeout(DEADLINE, answered).await;
                self.replies.forget(&id);
                let Ok(Ok(Inbound::ForwardOpened { input, output, .. })) = opened else {
                    // Refused, or not answered in time: the connection
                    // closes at once.
                    return;
                };
                let _counted = Counted::new(self, false);
                relay(tcp, input, output, &self.pipes, &self.backend, &self.cancel).await;
            }
        }
    }
}

/// Carries `tcp` through the relay: what it reads goes into `output`, what
/// `input` carries is written to it. Either side's clean end ends only its
/// own direction, as TCP's half-close does; a failure of either, or `ended`,
/// ends both at once. Each pipe's end is reported as every pipe's is.
async fn relay(
    tcp: TcpStream,
    input: PipeRef,
    output: PipeRef,
    pipes: &PipeClient,
    backend: &Backend,
    ended: &CancellationToken,
) {
    let (reader, mut writer) = tcp.into_split();
    let failed = ended.child_token();
    let gone = backend.closed.clone();
    let stopped = async {
        tokio::select! {
            () = failed.cancelled() => {}
            () = gone.cancelled() => {}
        }
    };
    let sending = async {
        let body = tokio_util::io::ReaderStream::new(reader);
        let sent = tokio::select! {
            () = stopped => Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "the connection ended")),
            sent = pipes.stream(&output.url, body, &failed) => sent,
        };
        if sent.is_err() {
            failed.cancel();
        }
        sent
    };
    let receiving = async {
        let received = async {
            let mut body = pipes.get(&input.url, failed.clone()).await?;
            while let Some(chunk) = body.next().await {
                writer.write_all(&chunk?).await?;
            }
            writer.shutdown().await
        };
        // A write the program does not take ends with the connection too.
        let received = tokio::select! {
            () = failed.cancelled() => Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "the connection ended")),
            () = gone.cancelled() => Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "the connection ended")),
            received = received => received,
        };
        if received.is_err() {
            failed.cancel();
        }
        received
    };
    let (sent, received) = tokio::join!(sending, receiving);
    report_pipe(&backend.control, output.id, sent, &backend.closed).await;
    report_pipe(&backend.control, input.id, received, &backend.closed).await;
}

/// Connects a connection that came through the relay to the forward's
/// address, and carries it until it or its forward's generation ends; one
/// that cannot connect, in time or at all, fails both pipes at once, so the
/// Listen On runner closes its connection.
async fn connect_relayed(
    target: Option<(String, CancellationToken)>,
    input: PipeRef,
    output: PipeRef,
    pipes: PipeClient,
    backend: Backend,
) {
    let (connected, ended) = match target {
        Some((address, ended)) => (connect_target(&address, &ended).await, ended),
        None => (
            Err(std::io::Error::other("this Host connects no such forward")),
            CancellationToken::new(),
        ),
    };
    match connected {
        Ok(tcp) => relay(tcp, input, output, &pipes, &backend, &ended).await,
        Err(error) => {
            tracing::debug!("a forwarded connection could not connect: {error}");
            let failed = futures_util::stream::once(async move { Err::<Bytes, _>(error) });
            let cancel = backend.closed.child_token();
            let sent = pipes.stream(&output.url, failed, &cancel).await;
            report_pipe(&backend.control, output.id, sent, &backend.closed).await;
            // Taken and dropped: the Listen On runner's bytes go nowhere.
            let received = pipes.get(&input.url, cancel).await.map(drop);
            report_pipe(&backend.control, input.id, received, &backend.closed).await;
        }
    }
}

/// The address the runner connects to for forward `forward` whose other
/// end is `from`, as its Connect To, and what ends the forward's
/// generation.
pub fn connect_target_of(list: &ForwardList, forward: &str, from: &str) -> Option<(String, CancellationToken)> {
    list.borrow()
        .iter()
        .find(|listed| listed.forward.id == forward && listed.forward.peer == from)
        .and_then(|listed| Some((listed.forward.connect.clone()?, listed.ended.clone())))
}

/// Makes peers with the runners at the other end of this runner's forwards,
/// introduced by the backend, this runner offering.
struct RunnerDial {
    direct: Direct,
    backend: Backend,
    replies: Replies,
    stun: watch::Receiver<Vec<StunUrl>>,
}

/// A connected peer with another runner.
#[derive(Clone)]
struct RunnerPeer {
    link: PeerLink,
    peer: String,
    direct: Direct,
}

impl PairPeer for RunnerPeer {
    type Channel = ForwardChannel;

    fn open(&self, forward: String) -> BoxFuture<'static, Option<ForwardChannel>> {
        let link = self.link.clone();
        Box::pin(async move { link.open(&forward).await })
    }

    fn ended(&self) -> BoxFuture<'static, Option<PairReason>> {
        let link = self.link.clone();
        Box::pin(async move { link.ended().await })
    }

    fn close(&self) {
        self.direct.close(&self.peer);
    }
}

impl Dial for RunnerDial {
    type Peer = RunnerPeer;

    fn dial(&self, device: String) -> BoxFuture<'static, Result<RunnerPeer, Option<PairReason>>> {
        let direct = self.direct.clone();
        let backend = self.backend.clone();
        let replies = self.replies.clone();
        let stun = self.stun.borrow().clone();
        Box::pin(async move {
            let peer = uuid::Uuid::new_v4().simple().to_string();
            let dialing = direct
                .dial(peer.clone(), stun)
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
            if !backend.send(&offer).await {
                direct.close(&peer);
                return Err(None);
            }
            let reason = match answered.await {
                Ok(Inbound::DirectDialed { sdp, .. }) => {
                    direct.answered(&peer, sdp);
                    None
                }
                Ok(Inbound::DirectDialRefused { code, message, .. }) => {
                    tracing::debug!("the backend did not introduce a pair: {message}");
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
                                if !told.send(&found).await {
                                    return;
                                }
                            }
                            None => break,
                        },
                        _ = ended.ended() => break,
                    }
                }
                ended.ended().await;
                told.send(&Outbound::DirectClosed { peer: ending }).await;
            });
            match link.connected().await {
                Ok(()) => Ok(RunnerPeer { link, peer, direct }),
                Err(reason) => {
                    direct.close(&peer);
                    Err(reason)
                }
            }
        })
    }
}
