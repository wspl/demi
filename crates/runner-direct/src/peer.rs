//! One peer: a page's WebRTC connection to the runner (`direct-channel.md`
//! § Making the channel). It owns its UDP sockets, its str0m connection and
//! its channels, each an operation; it gives up when it has not connected
//! within [`CONNECT_TIMEOUT`], and ends when its connection fails or the
//! runner closes it. Its end closes its sockets and ends its operations.

use std::collections::HashMap;
use std::future::poll_fn;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::task::Poll;

use bytes::Bytes;
use demi_runner_protocol::direct::{
    CONNECT_TIMEOUT, ChannelError, ChannelErrorCode, ChannelHeader, Introduction, MAX_CHANNELS,
    QUEUE_BYTES, WriteEnd,
};
use demi_runner_protocol::files::FileWatchRequest;
use garde::Validate;
use str0m::change::SdpOffer;
use str0m::channel::{ChannelData, ChannelId};
use str0m::crypto::CryptoProvider;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output as RtcOutput, Rtc, RtcConfig};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::operation::{self, Input as PageInput, OUTPUT_MESSAGES, Operations, Output, refusal};

/// The most bytes str0m queues on all of a peer's channels together: room
/// for a full queue on several channels at once. Each channel keeps its own
/// at [`QUEUE_BYTES`], since larger queues measured slower.
const SCTP_BUFFER: usize = 4 * QUEUE_BYTES;
/// The most bytes of one datagram; a data channel's are well under it.
const DATAGRAM_BYTES: usize = 2048;
/// Datagrams received that wait for the peer's loop.
const DATAGRAMS: usize = 1024;

/// The time str0m reads, which is tokio's clock, so a paused clock pauses
/// a peer too.
fn now() -> std::time::Instant {
    Instant::now().into_std()
}

/// A peer made from a page's offer, before it serves.
pub(crate) struct Peer {
    rtc: Rtc,
    sockets: Vec<Arc<UdpSocket>>,
    operations: Arc<dyn Operations>,
    introduction: Arc<Introduction>,
}

/// Why an offer made no peer.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OfferError {
    #[error("the offer is not one the runner can answer: {0}")]
    Invalid(String),
    #[error("no socket could be bound: {0}")]
    Sockets(std::io::Error),
}

impl Peer {
    /// Binds one socket on each of `addresses`, on a port the system picks,
    /// and answers `offer` with them as its candidates.
    pub(crate) async fn answer(
        offer: &str,
        addresses: Vec<Ipv4Addr>,
        provider: Arc<CryptoProvider>,
        operations: Arc<dyn Operations>,
        introduction: Arc<Introduction>,
    ) -> Result<(Self, String), OfferError> {
        let offer =
            SdpOffer::from_sdp_string(offer).map_err(|error| OfferError::Invalid(error.to_string()))?;
        let mut rtc = RtcConfig::new()
            .set_crypto_provider(provider)
            .set_sctp_max_buffered_amount(SCTP_BUFFER)
            .build(now());
        let mut sockets = Vec::new();
        let mut unbound = None;
        for address in addresses {
            match UdpSocket::bind(SocketAddr::from((address, 0))).await {
                Ok(socket) => {
                    let local = socket.local_addr().map_err(OfferError::Sockets)?;
                    match Candidate::host(local, "udp") {
                        Ok(candidate) => {
                            rtc.add_local_candidate(candidate);
                            sockets.push(Arc::new(socket));
                        }
                        // An address ICE does not take, such as a
                        // link-local one, is not offered.
                        Err(error) => tracing::debug!("{local} is not offered: {error}"),
                    }
                }
                // An address that went away meanwhile is not offered.
                Err(error) => unbound = Some(error),
            }
        }
        if sockets.is_empty() {
            let error = unbound.unwrap_or_else(|| std::io::Error::other("no address to bind"));
            return Err(OfferError::Sockets(error));
        }
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|error| OfferError::Invalid(error.to_string()))?;
        let peer = Self {
            rtc,
            sockets,
            operations,
            introduction,
        };
        Ok((peer, answer.to_sdp_string()))
    }

    /// Serves the peer until it gives up, fails or `cancel` closes it.
    pub(crate) async fn serve(self, cancel: CancellationToken) {
        let Self {
            mut rtc,
            sockets,
            operations,
            introduction,
        } = self;
        let (datagrams, mut received) = mpsc::channel(DATAGRAMS);
        let _readers: Vec<AbortOnDropHandle<()>> = sockets
            .iter()
            .map(|socket| AbortOnDropHandle::new(tokio::spawn(read(socket.clone(), datagrams.clone()))))
            .collect();
        drop(datagrams);
        let mut served = Served {
            channels: HashMap::new(),
            tasks: JoinSet::new(),
            operations,
            introduction,
            connected: false,
        };
        let give_up = Instant::now() + CONNECT_TIMEOUT;
        let end = loop {
            let timeout = match served.drain(&mut rtc, &sockets).await {
                Ok(Some(timeout)) => timeout,
                Ok(None) => break "the connection closed",
                Err(error) => {
                    tracing::debug!("a direct peer failed: {error}");
                    break "the connection failed";
                }
            };
            if served.pump(&mut rtc) {
                // What was written goes out now, not at str0m's next timer.
                if let Err(error) = rtc.handle_input(Input::Timeout(now())) {
                    tracing::debug!("a direct peer failed: {error}");
                    break "the connection failed";
                }
                continue;
            }
            let woke = tokio::select! {
                biased;
                () = cancel.cancelled() => break "the runner closed it",
                () = tokio::time::sleep_until(give_up), if !served.connected => {
                    break "it did not connect in time";
                }
                datagram = received.recv() => match datagram {
                    Some(datagram) => Wake::Datagram(datagram),
                    None => break "its sockets closed",
                },
                () = tokio::time::sleep_until(Instant::from_std(timeout)) => Wake::Timeout,
                () = served.output_ready() => Wake::Output,
            };
            let input = match &woke {
                Wake::Datagram(datagram) => {
                    let Ok(contents) = datagram.contents.as_slice().try_into() else {
                        // Not a datagram of the connection's.
                        continue;
                    };
                    Input::Receive(
                        now(),
                        Receive {
                            proto: Protocol::Udp,
                            source: datagram.source,
                            destination: datagram.destination,
                            contents,
                        },
                    )
                }
                Wake::Timeout => Input::Timeout(now()),
                Wake::Output => continue,
            };
            let received = matches!(input, Input::Receive(..));
            if let Err(error) = rtc.handle_input(input) {
                tracing::debug!("a direct peer failed: {error}");
                break "the connection failed";
            }
            // What the datagram calls for, such as the acknowledgement of a
            // channel's opening, goes out now, not at str0m's next timer.
            if received && let Err(error) = rtc.handle_input(Input::Timeout(now())) {
                tracing::debug!("a direct peer failed: {error}");
                break "the connection failed";
            }
        };
        tracing::debug!("a direct peer ended: {end}");
        // The page hears the close at once, rather than when its checks of
        // a peer gone quiet give up.
        if rtc.is_alive() && rtc.close().is_ok() {
            // What could not be sent no longer matters.
            let _ = served.drain(&mut rtc, &sockets).await;
        }
        rtc.disconnect();
        // The operations end with their channels: their tasks abort, and
        // each channel's drop ends what it holds.
        drop(served);
    }
}

/// What woke a peer's loop.
enum Wake {
    Datagram(Datagram),
    Timeout,
    Output,
}

struct Datagram {
    source: SocketAddr,
    destination: SocketAddr,
    contents: Vec<u8>,
}

/// Hands the peer each datagram its socket receives, until the peer ends.
async fn read(socket: Arc<UdpSocket>, datagrams: mpsc::Sender<Datagram>) {
    let Ok(destination) = socket.local_addr() else {
        return;
    };
    let mut buffer = vec![0; DATAGRAM_BYTES];
    loop {
        let (length, source) = match socket.recv_from(&mut buffer).await {
            Ok(received) => received,
            // An ICMP error a send provoked, as some systems report on the
            // next receive: the socket goes on.
            Err(error) => {
                tracing::debug!("a direct peer's socket could not receive: {error}");
                continue;
            }
        };
        let datagram = Datagram {
            source,
            destination,
            contents: buffer[..length].to_vec(),
        };
        if datagrams.send(datagram).await.is_err() {
            return;
        }
    }
}

/// A peer's channels and the operations they carry.
struct Served {
    channels: HashMap<ChannelId, Channel>,
    /// Each operation's task, which aborts with the peer.
    tasks: JoinSet<()>,
    operations: Arc<dyn Operations>,
    introduction: Arc<Introduction>,
    connected: bool,
}

/// One channel, from its open to its close.
struct Channel {
    phase: Phase,
    /// The message for the page that waits for room.
    pending: Option<Output>,
    /// The operation's messages; none once it ended, or for a channel
    /// refused at once.
    output: Option<mpsc::Receiver<Output>>,
    /// The channel's queue has room for another of the operation's
    /// messages.
    room: bool,
    /// Ends the operation: cancelled when the channel goes.
    cancel: CancellationToken,
}

/// Where a channel is.
enum Phase {
    /// Waiting for the page's header.
    Header,
    /// Carrying out the operation; the page's later messages go to it.
    Running {
        kind: Kind,
        input: mpsc::UnboundedSender<PageInput>,
    },
    /// The operation ended, or the channel was refused: it closes once its
    /// messages are out.
    Closing,
}

/// What a running operation takes after its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Bytes, then `{ end: true }`.
    Write,
    /// Bytes only.
    Stream,
    /// `paths` messages only.
    Watch,
    /// Nothing more.
    Request,
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Channel {
    fn closing(message: Output) -> Self {
        Self {
            phase: Phase::Closing,
            pending: Some(message),
            output: None,
            room: false,
            cancel: CancellationToken::new(),
        }
    }
}

impl Served {
    /// Sends what str0m has to send and takes its events; the time it next
    /// wants to be woken at, or none once the connection is closed.
    async fn drain(
        &mut self,
        rtc: &mut Rtc,
        sockets: &[Arc<UdpSocket>],
    ) -> Result<Option<std::time::Instant>, str0m::RtcError> {
        loop {
            if !rtc.is_alive() {
                return Ok(None);
            }
            match rtc.poll_output()? {
                RtcOutput::Timeout(timeout) => return Ok(Some(timeout)),
                RtcOutput::Transmit(transmit) => {
                    let socket = sockets
                        .iter()
                        .find(|socket| socket.local_addr().ok() == Some(transmit.source));
                    if let Some(socket) = socket
                        && let Err(error) = socket.send_to(&transmit.contents, transmit.destination).await
                    {
                        // A pair that cannot route, such as loopback to a
                        // network address, is one ICE does not pick.
                        tracing::trace!("a direct peer's datagram to {} failed: {error}", transmit.destination);
                    }
                }
                RtcOutput::Event(event) => {
                    if !self.event(rtc, event) {
                        return Ok(None);
                    }
                }
            }
        }
    }

    /// Takes one event; false once the connection is over.
    fn event(&mut self, rtc: &mut Rtc, event: Event) -> bool {
        match event {
            Event::Connected => self.connected = true,
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => return false,
            Event::Closed => return false,
            Event::ChannelOpen(id, _) => {
                if self.channels.len() >= MAX_CHANNELS {
                    let busy = ChannelError::new(
                        ChannelErrorCode::Busy,
                        503,
                        "The device has as many channels open as it may",
                    );
                    self.channels.insert(id, Channel::closing(refusal(busy)));
                } else {
                    let channel = Channel {
                        phase: Phase::Header,
                        pending: None,
                        output: None,
                        room: false,
                        cancel: CancellationToken::new(),
                    };
                    self.channels.insert(id, channel);
                }
                if let Some(mut channel) = rtc.channel(id) {
                    channel.set_buffered_amount_low_threshold(QUEUE_BYTES / 2);
                }
            }
            Event::ChannelData(data) => self.data(data),
            // The page closed it: its operation ends.
            Event::ChannelClose(id) => {
                self.channels.remove(&id);
            }
            _ => {}
        }
        true
    }

    /// One message of the page's on a channel.
    fn data(&mut self, data: ChannelData) {
        let Some(channel) = self.channels.get_mut(&data.id) else {
            return;
        };
        match &channel.phase {
            Phase::Header => {
                let header = (!data.binary)
                    .then(|| serde_json::from_slice::<ChannelHeader>(&data.data).ok())
                    .flatten()
                    .filter(|header| header.validate().is_ok());
                let Some(header) = header else {
                    let invalid = ChannelError::new(
                        ChannelErrorCode::InvalidMessage,
                        400,
                        "The channel's first message is no valid header",
                    );
                    *channel = Channel::closing(refusal(invalid));
                    return;
                };
                let kind = match header {
                    ChannelHeader::Write { .. } => Kind::Write,
                    ChannelHeader::Stream { .. } => Kind::Stream,
                    ChannelHeader::Watch { .. } => Kind::Watch,
                    _ => Kind::Request,
                };
                let (input, inputs) = mpsc::unbounded_channel();
                let (outputs, output) = mpsc::channel(OUTPUT_MESSAGES);
                self.tasks.spawn(operation::run(
                    self.operations.clone(),
                    self.introduction.clone(),
                    header,
                    inputs,
                    outputs,
                    channel.cancel.clone(),
                ));
                channel.phase = Phase::Running { kind, input };
                channel.output = Some(output);
                channel.room = true;
            }
            Phase::Running { kind, input } => {
                match page_message(*kind, data.binary, Bytes::from(data.data)) {
                    Some(message) => {
                        // An operation that ended takes nothing more.
                        let _ = input.send(message);
                    }
                    None => {
                        // A message the operation cannot read ends it.
                        channel.cancel.cancel();
                        channel.output = None;
                        channel.phase = Phase::Closing;
                    }
                }
            }
            Phase::Closing => {}
        }
    }

    /// Writes each channel's waiting messages while its queue has room; a
    /// channel whose operation ended closes once its queue is empty. Whether
    /// it wrote or closed any, which str0m then has to send.
    fn pump(&mut self, rtc: &mut Rtc) -> bool {
        let mut wrote = false;
        let mut closed = Vec::new();
        for (id, channel) in &mut self.channels {
            let Some(mut sending) = rtc.channel(*id) else {
                closed.push(*id);
                continue;
            };
            loop {
                if channel.pending.is_none()
                    && let Some(output) = &mut channel.output
                {
                    match output.try_recv() {
                        Ok(message) => channel.pending = Some(message),
                        Err(mpsc::error::TryRecvError::Empty) => {}
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            channel.output = None;
                            channel.phase = Phase::Closing;
                        }
                    }
                }
                let Some(message) = &channel.pending else {
                    break;
                };
                let queued = sending.buffered_amount();
                if queued > 0 && queued + message.len() > QUEUE_BYTES {
                    break;
                }
                let written = match message {
                    Output::Text(text) => sending.write(false, text.as_bytes()),
                    Output::Binary(bytes) => sending.write(true, bytes),
                };
                match written {
                    Ok(true) => {
                        channel.pending = None;
                        wrote = true;
                    }
                    // Every channel's queue together is full.
                    Ok(false) => break,
                    Err(error) => {
                        tracing::debug!("a direct channel failed: {error}");
                        closed.push(*id);
                        break;
                    }
                }
            }
            channel.room = channel.pending.is_none()
                && channel.output.is_some()
                && sending.buffered_amount() < QUEUE_BYTES;
            let done = matches!(channel.phase, Phase::Closing)
                && channel.pending.is_none()
                && channel.output.is_none()
                && sending.buffered_amount() == 0;
            if done {
                closed.push(*id);
            }
        }
        // A close is sent like a message.
        let changed = wrote || !closed.is_empty();
        for id in closed {
            self.channels.remove(&id);
            rtc.direct_api().close_data_channel(id);
        }
        changed
    }

    /// Resolves once a channel with room has a message from its operation,
    /// or its operation ended.
    async fn output_ready(&mut self) {
        poll_fn(|context| {
            for channel in self.channels.values_mut() {
                if !channel.room {
                    continue;
                }
                let Some(output) = &mut channel.output else {
                    continue;
                };
                match output.poll_recv(context) {
                    Poll::Ready(Some(message)) => {
                        channel.pending = Some(message);
                        return Poll::Ready(());
                    }
                    Poll::Ready(None) => {
                        channel.output = None;
                        channel.phase = Phase::Closing;
                        return Poll::Ready(());
                    }
                    Poll::Pending => {}
                }
            }
            Poll::Pending
        })
        .await
    }
}

/// What a page's message after the header means to an operation of `kind`;
/// none for one it does not take.
fn page_message(kind: Kind, binary: bool, data: Bytes) -> Option<PageInput> {
    match (kind, binary) {
        (Kind::Write | Kind::Stream, true) => Some(PageInput::Bytes(data)),
        (Kind::Write, false) => serde_json::from_slice::<WriteEnd>(&data)
            .ok()
            .filter(|end| end.end)
            .map(|_| PageInput::End),
        (Kind::Watch, false) => serde_json::from_slice::<FileWatchRequest>(&data)
            .ok()
            .filter(|request| request.validate().is_ok())
            .map(|FileWatchRequest::Paths { paths }| PageInput::Paths(paths)),
        _ => None,
    }
}
