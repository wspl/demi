//! One peer (`direct-channel.md` § Making the channel): a WebRTC connection
//! the backend introduced, with a page that offered, with another runner
//! that offered, or with another runner this one offers to
//! (`port-forwarding.md` § The path of a connection). Whichever role it
//! has, it owns its UDP sockets, its str0m connection and its channels; it
//! learns the addresses the internet sees its sockets at and offers them to
//! the other side, and checks the candidates the other side trickles. It
//! gives up when it has not connected within [`CONNECT_TIMEOUT`], and ends
//! when its connection fails or the runner closes it. Its end closes its
//! sockets and ends its channels' work.

use std::collections::{HashMap, HashSet};
use std::future::poll_fn;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::task::Poll;

use demi_runner_protocol::direct::{
    CONNECT_TIMEOUT, ChannelError, ChannelErrorCode, ChannelHeader, Introduction, MAX_CHANNELS,
    PROBE_LABEL, QUEUE_BYTES, StunUrl, VIDEO_QUEUE_BYTES, VideoOpened,
};
use demi_runner_protocol::forward::{ForwardHeader, PairReason, PairSide};
use garde::Validate;
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::{ChannelConfig, ChannelData, ChannelId};
use str0m::crypto::CryptoProvider;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output as RtcOutput, Rtc, RtcConfig};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::channel::{Channel, Kind, Phase};
use crate::forward::{ForwardChannel, Took};
use crate::gathering::{self, Gathering};
use crate::operation::{
    self, Input as ChannelInput, OUTPUT_MESSAGES, Operations, Output, VideoEnds, refusal,
};

/// The most bytes str0m queues on all of a peer's channels together: room
/// for a full queue on several channels at once. Each channel keeps its own
/// at [`QUEUE_BYTES`], since larger queues measured slower.
const SCTP_BUFFER: usize = 4 * QUEUE_BYTES;
/// What a live views' peer queues on all its channels together: room for a
/// few views' `video` channels.
const LIVE_SCTP_BUFFER: usize = 4 * VIDEO_QUEUE_BYTES;
/// The least a live views' peer's SCTP congestion window ever is: far
/// beyond what any path carries in a round trip, so the window never limits
/// what the views' own rate control sends (`direct-channel.md` § Making the
/// channel).
const LIVE_CONGESTION_WINDOW: u32 = 64 * 1024 * 1024;
/// A live views' peer resends a lost chunk after this long at the least, at
/// first and at the most, where SCTP's defaults wait a second at the least.
const LIVE_RTO: (std::time::Duration, std::time::Duration, std::time::Duration) = (
    std::time::Duration::from_millis(50),
    std::time::Duration::from_millis(250),
    std::time::Duration::from_secs(2),
);
/// The most bytes of one datagram; a data channel's are well under it.
const DATAGRAM_BYTES: usize = 2048;
/// Datagrams received that wait for the peer's loop.
const DATAGRAMS: usize = 1024;
/// The label of a channel that carries a forwarded connection.
const FORWARD_LABEL: &str = "forward";

/// The time str0m reads, which is tokio's clock, so a paused clock pauses
/// a peer too.
fn now() -> std::time::Instant {
    Instant::now().into_std()
}

/// What the other side's channels carry.
#[derive(Clone)]
pub(crate) enum Behaviour {
    /// A page's operations, which the runner carries out for the
    /// introducing user.
    Page {
        operations: Arc<dyn Operations>,
        introduction: Arc<Introduction>,
        /// The page's peer for live views, which carries only their `stream`
        /// and `video` channels, without SCTP's congestion window.
        live: bool,
    },
    /// Forwarded connections that the runner of `device` opens, which this
    /// runner connects as their forwards' Connect To.
    Runner {
        operations: Arc<dyn Operations>,
        device: String,
    },
    /// Nothing: this side offered, and opens every channel itself.
    Offerer,
}

/// Where a peer is: connecting, connected, or ended, with why a direct
/// path failed when it failed (`port-forwarding.md` § What the user sees).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerStatus {
    Connecting,
    Connected,
    /// Ended; why, unless the runner closed it.
    Ended(Option<PairReason>),
}

/// A channel this side opens, with what it carries.
pub(crate) struct LocalOpen {
    pub(crate) config: ChannelConfig,
    pub(crate) kind: Kind,
    pub(crate) first: Option<Output>,
    pub(crate) input: mpsc::UnboundedSender<ChannelInput>,
    pub(crate) output: mpsc::Receiver<Output>,
    pub(crate) cancel: CancellationToken,
    pub(crate) credit: bool,
    /// Closes the channel at once (`Channel::abort_on`).
    pub(crate) abort: Option<CancellationToken>,
    /// The channel's id, once str0m gave it one, for its consumer's grants.
    pub(crate) id: tokio::sync::oneshot::Sender<(ChannelId, mpsc::UnboundedSender<(ChannelId, usize)>)>,
}

/// What the other side says after the offer.
pub(crate) enum Remote {
    /// A candidate it found after the offer or the answer.
    Candidate(String),
    /// Its answer, to a peer this side offered.
    Answer(String),
}

/// A peer made, before it serves.
pub(crate) struct Peer {
    rtc: Rtc,
    sockets: Vec<Arc<UdpSocket>>,
    behaviour: Behaviour,
    stun: Vec<StunUrl>,
    /// This side's offer, which the other side's answer completes.
    offered: Option<SdpPendingOffer>,
    /// The other side offered an address the internet sees it at.
    remote_public: bool,
}

/// What a served peer hears from the other side and tells it and the
/// runner.
pub(crate) struct Signals {
    /// The other side's answer and candidates.
    pub(crate) remote: mpsc::UnboundedReceiver<Remote>,
    /// The candidates this side finds after its offer or answer.
    pub(crate) found: mpsc::UnboundedSender<String>,
    /// The channels this side opens.
    pub(crate) local: mpsc::UnboundedReceiver<LocalOpen>,
    pub(crate) status: watch::Sender<PeerStatus>,
}

/// Why no peer was made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OfferError {
    #[error("the offer is not one the runner can answer: {0}")]
    Invalid(String),
    #[error("no socket could be bound: {0}")]
    Sockets(std::io::Error),
}

/// A connection with one socket on each of `addresses`, on a port the
/// system picks, offered as its candidates.
async fn bind(
    addresses: Vec<Ipv4Addr>,
    provider: Arc<CryptoProvider>,
    live: bool,
) -> Result<(Rtc, Vec<Arc<UdpSocket>>), OfferError> {
    let config = RtcConfig::new().set_crypto_provider(provider);
    let config = if live {
        let (least, initial, most) = LIVE_RTO;
        config
            .set_sctp_max_buffered_amount(LIVE_SCTP_BUFFER)
            .set_sctp_congestion_window_floor(LIVE_CONGESTION_WINDOW)
            .set_sctp_rto(least, initial, most)
    } else {
        config.set_sctp_max_buffered_amount(SCTP_BUFFER)
    };
    let mut rtc = config.build(now());
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
                    // An address ICE does not take, such as a link-local
                    // one, is not offered.
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
    Ok((rtc, sockets))
}

/// Whether an SDP candidate names an address the internet sees its side at.
fn is_public(candidate: &str) -> bool {
    candidate.contains(" typ srflx")
}

impl Peer {
    /// Answers `offer` with a new peer whose channels carry what
    /// `behaviour` says; once it serves, it asks the STUN servers `stun`
    /// for more candidates.
    pub(crate) async fn answer(
        offer: &str,
        addresses: Vec<Ipv4Addr>,
        provider: Arc<CryptoProvider>,
        behaviour: Behaviour,
        stun: Vec<StunUrl>,
    ) -> Result<(Self, String), OfferError> {
        let parsed =
            SdpOffer::from_sdp_string(offer).map_err(|error| OfferError::Invalid(error.to_string()))?;
        let live = matches!(behaviour, Behaviour::Page { live: true, .. });
        let (mut rtc, sockets) = bind(addresses, provider, live).await?;
        let answer = rtc
            .sdp_api()
            .accept_offer(parsed)
            .map_err(|error| OfferError::Invalid(error.to_string()))?;
        let peer = Self {
            rtc,
            sockets,
            behaviour,
            stun,
            offered: None,
            remote_public: offer.lines().any(is_public),
        };
        Ok((peer, answer.to_sdp_string()))
    }

    /// Offers a new peer, whose channels this side opens; the other side's
    /// answer arrives as [`Remote::Answer`] once it serves.
    pub(crate) async fn offer(
        addresses: Vec<Ipv4Addr>,
        provider: Arc<CryptoProvider>,
        stun: Vec<StunUrl>,
    ) -> Result<(Self, String), OfferError> {
        let (mut rtc, sockets) = bind(addresses, provider, false).await?;
        let mut changes = rtc.sdp_api();
        // A first channel, so that the offer carries one: the probe
        // channel, as a page's is.
        changes.add_channel_with_config(probe_config());
        let (offer, pending) = changes
            .apply()
            .expect("an offer with a channel has a change to apply");
        let peer = Self {
            rtc,
            sockets,
            behaviour: Behaviour::Offerer,
            stun,
            offered: Some(pending),
            remote_public: false,
        };
        Ok((peer, offer.to_sdp_string()))
    }

    /// Serves the peer until it gives up, fails or `cancel` closes it.
    pub(crate) async fn serve(self, cancel: CancellationToken, signals: Signals) {
        let Self {
            mut rtc,
            sockets,
            behaviour,
            stun,
            mut offered,
            mut remote_public,
        } = self;
        let Signals {
            mut remote,
            found,
            mut local,
            status,
        } = signals;
        let crossing = !stun.is_empty();
        let mut remote_open = true;
        let mut local_open = true;
        // The servers' addresses, resolved beside the peer's start; the
        // requests go once they are known.
        let (resolved, mut resolving) = tokio::sync::oneshot::channel();
        let _resolver = AbortOnDropHandle::new(tokio::spawn(async move {
            // A peer that ended no longer waits for the addresses.
            let _ = resolved.send(gathering::resolve(stun).await);
        }));
        let mut gathering = Gathering::default();
        let mut servers_known = false;
        let mut found_public = false;
        let (datagrams, mut received) = mpsc::channel(DATAGRAMS);
        let _readers: Vec<AbortOnDropHandle<()>> = sockets
            .iter()
            .map(|socket| AbortOnDropHandle::new(tokio::spawn(read(socket.clone(), datagrams.clone()))))
            .collect();
        drop(datagrams);
        let (took, mut taken) = mpsc::unbounded_channel();
        let mut served = Served {
            channels: HashMap::new(),
            probes: HashSet::new(),
            tasks: JoinSet::new(),
            behaviour,
            connected: false,
            took,
            videos: HashMap::new(),
            next_video: 0,
        };
        let give_up = Instant::now() + CONNECT_TIMEOUT;
        let end = loop {
            let timeout = match served.drain(&mut rtc, &sockets).await {
                Ok(Some(timeout)) => timeout,
                Ok(None) => break failed("the connection closed"),
                Err(error) => {
                    tracing::debug!("a direct peer failed: {error}");
                    break failed("the connection failed");
                }
            };
            if served.connected && *status.borrow() == PeerStatus::Connecting {
                status.send_replace(PeerStatus::Connected);
            }
            if served.pump(&mut rtc) {
                // What was written goes out now, not at str0m's next timer.
                if let Err(error) = rtc.handle_input(Input::Timeout(now())) {
                    tracing::debug!("a direct peer failed: {error}");
                    break failed("the connection failed");
                }
                continue;
            }
            let resend = gathering.next();
            let woke = tokio::select! {
                biased;
                () = cancel.cancelled() => break End::Closed,
                () = tokio::time::sleep_until(give_up), if !served.connected => {
                    break End::TimedOut;
                }
                datagram = received.recv() => match datagram {
                    Some(datagram) => Wake::Datagram(datagram),
                    None => break failed("its sockets closed"),
                },
                servers = &mut resolving, if !servers_known => {
                    servers_known = true;
                    // A resolver that ended without an answer found nothing.
                    gathering.start(&servers.unwrap_or_default(), &sockets).await;
                    continue;
                }
                () = async { tokio::time::sleep_until(resend.unwrap_or_else(Instant::now)).await }, if resend.is_some() => {
                    gathering.resend().await;
                    continue;
                }
                said = remote.recv(), if remote_open => match said {
                    Some(said) => Wake::Remote(said),
                    None => {
                        remote_open = false;
                        continue;
                    }
                },
                opened = local.recv(), if local_open => match opened {
                    Some(opened) => {
                        served.open(&mut rtc, opened);
                        Wake::Timeout
                    }
                    None => {
                        local_open = false;
                        continue;
                    }
                },
                Some((id, bytes)) = taken.recv() => {
                    if let Some(channel) = served.channels.get_mut(&id) {
                        channel.took(bytes);
                    }
                    continue;
                }
                () = tokio::time::sleep_until(Instant::from_std(timeout)) => Wake::Timeout,
                () = served.output_ready() => Wake::Output,
            };
            let input = match &woke {
                Wake::Remote(Remote::Candidate(candidate)) => {
                    remote_public |= is_public(candidate);
                    match Candidate::from_sdp_string(candidate) {
                        Ok(candidate) => rtc.add_remote_candidate(candidate),
                        // A candidate ICE cannot use, such as a browser's
                        // name for its address, is passed over.
                        Err(error) => tracing::debug!("a candidate is passed over: {error}"),
                    }
                    Input::Timeout(now())
                }
                Wake::Remote(Remote::Answer(answer)) => {
                    let Some(pending) = offered.take() else {
                        tracing::debug!("an answer to a peer that offered nothing is passed over");
                        continue;
                    };
                    remote_public |= answer.lines().any(is_public);
                    let accepted = SdpAnswer::from_sdp_string(answer)
                        .map_err(|error| error.to_string())
                        .and_then(|answer| {
                            rtc.sdp_api()
                                .accept_answer(pending, answer)
                                .map_err(|error| error.to_string())
                        });
                    if let Err(error) = accepted {
                        tracing::debug!("a direct peer's answer is refused: {error}");
                        break failed("its answer could not be taken");
                    }
                    Input::Timeout(now())
                }
                Wake::Datagram(datagram) => {
                    if let Some(mapped) = gathering.answer(datagram.destination, &datagram.contents) {
                        if offer_public(mapped, datagram.destination, &mut rtc, &found) {
                            found_public = true;
                        }
                        Input::Timeout(now())
                    } else {
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
                }
                Wake::Timeout => Input::Timeout(now()),
                Wake::Output => continue,
            };
            if let Err(error) = rtc.handle_input(input) {
                tracing::debug!("a direct peer failed: {error}");
                break failed("the connection failed");
            }
        };
        tracing::debug!("a direct peer ended: {end:?}");
        let reason = match end {
            End::Closed => None,
            _ if served.connected => Some(PairReason::Dropped),
            _ if !crossing => Some(PairReason::NotOffered),
            _ if !found_public => Some(PairReason::Network {
                side: PairSide::Browser,
            }),
            _ if !remote_public => Some(PairReason::Network {
                side: PairSide::Device,
            }),
            _ => Some(PairReason::Unreachable),
        };
        status.send_replace(PeerStatus::Ended(reason));
        // The other side hears the close at once, rather than when its
        // checks of a peer gone quiet give up.
        if rtc.is_alive() && rtc.close().is_ok() {
            // What could not be sent no longer matters.
            let _ = served.drain(&mut rtc, &sockets).await;
        }
        rtc.disconnect();
        // The channels' work ends with them: their tasks abort, and each
        // channel's drop ends what it holds.
        drop(served);
    }
}

/// How a peer's loop ended.
#[derive(Debug)]
enum End {
    /// The runner closed it.
    Closed,
    /// It did not connect in time.
    TimedOut,
    Failed,
}

/// A peer's loop ends failed, which the runner's log says why.
fn failed(why: &str) -> End {
    tracing::debug!("a direct peer failed: {why}");
    End::Failed
}

/// What woke a peer's loop.
enum Wake {
    Datagram(Datagram),
    Remote(Remote),
    Timeout,
    Output,
}

/// Offers the address a STUN server saw the socket at `base` at, `mapped`,
/// to the other side when it is new; whether it was.
fn offer_public(
    mapped: SocketAddr,
    base: SocketAddr,
    rtc: &mut Rtc,
    found: &mpsc::UnboundedSender<String>,
) -> bool {
    match Candidate::server_reflexive(mapped, base, "udp") {
        // A candidate the peer had already, such as the same address another
        // server named, is not offered again.
        Ok(candidate) => {
            if let Some(added) = rtc.add_local_candidate(candidate) {
                // A side that went no longer needs it.
                let _ = found.send(added.to_sdp_string());
            }
            true
        }
        Err(error) => {
            tracing::debug!("{mapped} is not offered: {error}");
            false
        }
    }
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

/// A peer's channels and the work they carry.
struct Served {
    channels: HashMap<ChannelId, Channel>,
    /// The page's probe channels, which echo what they carry.
    probes: HashSet<ChannelId>,
    /// Each channel's work, which aborts with the peer.
    tasks: JoinSet<()>,
    behaviour: Behaviour,
    connected: bool,
    /// Where a credited channel's consumer says what it took.
    took: mpsc::UnboundedSender<(ChannelId, usize)>,
    /// The `video` channels no stream claimed yet, by their number, with
    /// their channel.
    videos: HashMap<u32, (ChannelId, VideoEnds)>,
    /// The last `video` channel's number.
    next_video: u32,
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

    /// Opens a channel of this side's.
    fn open(&mut self, rtc: &mut Rtc, open: LocalOpen) {
        let LocalOpen {
            config,
            kind,
            first,
            input,
            output,
            cancel,
            credit,
            abort,
            id,
        } = open;
        // An opener that went has nothing for the channel to carry.
        if id.is_closed() {
            return;
        }
        let made = rtc.direct_api().create_data_channel(config);
        // The opener went meanwhile: its channel closes as it ends.
        let _ = id.send((made, self.took.clone()));
        let mut channel = Channel::opening(kind, first, input, output, cancel, credit);
        if let Some(abort) = abort {
            channel.abort_on(abort);
        }
        self.channels.insert(made, channel);
    }

    /// Takes one event; false once the connection is over.
    fn event(&mut self, rtc: &mut Rtc, event: Event) -> bool {
        match event {
            Event::Connected => self.connected = true,
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => return false,
            Event::Closed => return false,
            Event::ChannelOpen(id, _) if self.channels.contains_key(&id) => {
                // A channel this side opened.
                if let Some(channel) = self.channels.get_mut(&id) {
                    channel.opened();
                }
            }
            // The other side's probe channel carries no operation: each
            // message goes back as it came (`direct-channel.md` § Measuring
            // the paths).
            Event::ChannelOpen(id, label) if label == PROBE_LABEL => {
                self.probes.insert(id);
            }
            Event::ChannelOpen(id, _) => {
                if matches!(self.behaviour, Behaviour::Offerer) {
                    // The side that offered opens every channel itself.
                    rtc.direct_api().close_data_channel(id);
                } else if self.channels.len() >= MAX_CHANNELS {
                    let busy = ChannelError::new(
                        ChannelErrorCode::Busy,
                        503,
                        "The device has as many channels open as it may",
                    );
                    self.channels.insert(id, Channel::closing(refusal(busy)));
                } else {
                    self.channels.insert(id, Channel::awaiting_header());
                }
                if let Some(mut channel) = rtc.channel(id) {
                    channel.set_buffered_amount_low_threshold(QUEUE_BYTES / 2);
                }
            }
            Event::ChannelData(data) if self.probes.contains(&data.id) => {
                if let Some(mut probe) = rtc.channel(data.id) {
                    // A probe the queue cannot take now is lost, as an
                    // unanswered probe is; the page counts it so.
                    let _ = probe.write(data.binary, &data.data);
                }
            }
            Event::ChannelData(data) => self.data(data),
            // The other side closed it: its work ends.
            Event::ChannelClose(id) => {
                self.channels.remove(&id);
                self.probes.remove(&id);
                // A `video` channel no stream claimed goes with it.
                self.videos.retain(|_, (channel, _)| *channel != id);
            }
            _ => {}
        }
        true
    }

    /// One message of the other side's on a channel.
    fn data(&mut self, data: ChannelData) {
        let Some(channel) = self.channels.get_mut(&data.id) else {
            return;
        };
        if !matches!(channel.phase, Phase::Header) {
            channel.receive(data.binary, data.data);
            return;
        }
        let (input, inputs) = mpsc::unbounded_channel();
        let (outputs, output) = mpsc::channel(OUTPUT_MESSAGES);
        let cancel = channel.cancel.clone();
        match &self.behaviour {
            Behaviour::Page {
                operations,
                introduction,
                live,
            } => {
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
                // A live views' peer carries their `stream` and `video`
                // channels alone, and only it carries `video`.
                let carried = match header {
                    ChannelHeader::Stream { .. } => true,
                    ChannelHeader::Video {} => *live,
                    _ => !*live,
                };
                if !carried {
                    let elsewhere = ChannelError::new(
                        ChannelErrorCode::InvalidMessage,
                        400,
                        "The peer does not carry this operation",
                    );
                    *channel = Channel::closing(refusal(elsewhere));
                    return;
                }
                if let ChannelHeader::Video {} = header {
                    self.next_video += 1;
                    let number = self.next_video;
                    let opened = VideoOpened { ok: true, id: number };
                    let opened = serde_json::to_string(&opened).expect("an answer serializes");
                    outputs
                        .try_send(Output::Text(opened))
                        .expect("a new channel has room for its answer");
                    channel.run(Kind::Stream, input, output, false);
                    channel.queue(VIDEO_QUEUE_BYTES);
                    let ends = VideoEnds {
                        chunks: outputs,
                        acks: inputs,
                    };
                    self.videos.insert(number, (data.id, ends));
                    return;
                }
                let video = match &header {
                    ChannelHeader::Stream { video: Some(number), .. } => {
                        match self.videos.remove(number) {
                            Some((_, ends)) => Some(ends),
                            None => {
                                let unknown = ChannelError::new(
                                    ChannelErrorCode::InvalidMessage,
                                    400,
                                    "The stream names no video channel of the peer's",
                                );
                                *channel = Channel::closing(refusal(unknown));
                                return;
                            }
                        }
                    }
                    _ => None,
                };
                let kind = match header {
                    ChannelHeader::Write { .. } => Kind::Write,
                    ChannelHeader::Stream { .. } => Kind::Stream,
                    ChannelHeader::Watch { .. } => Kind::Watch,
                    _ => Kind::Request,
                };
                self.tasks.spawn(operation::run(
                    operations.clone(),
                    introduction.clone(),
                    header,
                    inputs,
                    outputs,
                    video,
                    cancel,
                ));
                channel.run(kind, input, output, false);
            }
            Behaviour::Runner { operations, device } => {
                let header = (!data.binary)
                    .then(|| serde_json::from_slice::<ForwardHeader>(&data.data).ok())
                    .flatten()
                    .filter(|header| header.validate().is_ok());
                let Some(header) = header else {
                    // A channel that is no forwarded connection's carries
                    // nothing between runners.
                    channel.phase = Phase::Closing;
                    return;
                };
                let abort = CancellationToken::new();
                let forwarded = ForwardChannel {
                    input: inputs,
                    output: outputs,
                    took: Took {
                        id: data.id,
                        sender: self.took.clone(),
                    },
                    cancel,
                    abort: abort.clone(),
                };
                self.tasks.spawn(crate::forward::serve(
                    operations.clone(),
                    device.clone(),
                    header,
                    forwarded,
                ));
                channel.run(Kind::Forward, input, output, true);
                channel.abort_on(abort);
            }
            Behaviour::Offerer => channel.phase = Phase::Closing,
        }
    }

    /// Writes each channel's waiting messages while it has room; a channel
    /// whose work ended closes once its queue is empty. Whether it wrote or
    /// closed any, which str0m then has to send.
    fn pump(&mut self, rtc: &mut Rtc) -> bool {
        let mut wrote = false;
        let mut closed = Vec::new();
        for (id, channel) in &mut self.channels {
            let pumped = channel.pump(rtc, *id);
            wrote |= pumped.wrote;
            if pumped.done {
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

    /// Resolves once a channel with room has a message from its work, or
    /// its work ended.
    async fn output_ready(&mut self) {
        poll_fn(|context| {
            for channel in self.channels.values_mut() {
                if channel.poll_output(context).is_ready() {
                    return Poll::Ready(());
                }
            }
            Poll::Pending
        })
        .await
    }
}

/// The probe channel's settings, unordered and never retransmitted, as the
/// page's (`direct-channel.md` § Measuring the paths).
pub(crate) fn probe_config() -> ChannelConfig {
    ChannelConfig {
        label: PROBE_LABEL.into(),
        ordered: false,
        reliability: str0m::channel::Reliability::MaxRetransmits { retransmits: 0 },
        ..Default::default()
    }
}

/// A channel this side opens for a forwarded connection of `forward`,
/// credited, whose first message is its header.
pub(crate) fn forward_open(
    forward: &str,
) -> (LocalOpen, tokio::sync::oneshot::Receiver<(ChannelId, mpsc::UnboundedSender<(ChannelId, usize)>)>, mpsc::UnboundedReceiver<ChannelInput>, mpsc::Sender<Output>, CancellationToken, CancellationToken) {
    let header = ForwardHeader::Forward {
        forward: forward.to_owned(),
    };
    let header = serde_json::to_string(&header).expect("a header serializes");
    let (input, inputs) = mpsc::unbounded_channel();
    let (outputs, output) = mpsc::channel(OUTPUT_MESSAGES);
    let (id, named) = tokio::sync::oneshot::channel();
    let cancel = CancellationToken::new();
    let abort = CancellationToken::new();
    let open = LocalOpen {
        config: ChannelConfig {
            label: FORWARD_LABEL.into(),
            ..Default::default()
        },
        kind: Kind::Forward,
        first: Some(Output::Text(header)),
        input,
        output,
        cancel: cancel.clone(),
        credit: true,
        abort: Some(abort.clone()),
        id,
    };
    (open, named, inputs, outputs, cancel, abort)
}
