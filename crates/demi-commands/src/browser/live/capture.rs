//! The loopback socket the capture extension connects to
//! (`live-view.md` § Capture): the module starts and steers tab
//! captures over it, and receives their encoded frames. It binds before Chrome
//! starts, since the extension learns its address from its own files, and
//! accepts only this environment's token. One owner task, the capture
//! channel, keeps the extension's connection and routes the captures on it.

use std::{collections::HashMap, future::Future, pin::Pin};

use bytes::Bytes;
use demi_builtin_protocol::{
    DecodeError,
    capture::{self as extension, CaptureCommand},
};
use futures_util::{SinkExt, StreamExt, stream::FuturesUnordered};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
    },
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::super::{BrowserError, Result};

/// Frames a capture's consumer has not taken yet; beyond them the consumer
/// is behind and the newest frames are dropped until it asks for a key frame.
const FRAME_QUEUE: usize = 8;

/// Why this Host cannot capture, or none (`live-view.md` §
/// Capture). On Linux arm64, Chrome's SME code faults on a CPU that reports
/// SME without SVE, until the GPU process gives up and Chrome exits.
pub(crate) fn unavailable() -> Option<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        // SAFETY: getauxval only reads the process's auxiliary vector.
        let (hwcap, hwcap2) = unsafe {
            (
                libc::getauxval(libc::AT_HWCAP),
                libc::getauxval(libc::AT_HWCAP2),
            )
        };
        if sme_without_sve(hwcap, hwcap2) {
            return Some(
                "this Host's CPU reports SME without SVE, which Chrome's tab capture cannot run on; boot its kernel with arm64.nosme to show the browser",
            );
        }
    }
    None
}

/// The kernel's `HWCAP_SVE` and `HWCAP2_SME` bits (`asm/hwcap.h`).
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "aarch64")),
    allow(dead_code)
)]
fn sme_without_sve(hwcap: u64, hwcap2: u64) -> bool {
    const HWCAP_SVE: u64 = 1 << 22;
    const HWCAP2_SME: u64 = 1 << 23;
    hwcap2 & HWCAP2_SME != 0 && hwcap & HWCAP_SVE == 0
}

/// One encoded picture of a capture.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub sequence: u32,
    pub key: bool,
    pub timestamp: f64,
    pub width: u16,
    pub height: u16,
    pub data: Bytes,
}

#[derive(Debug)]
pub(crate) enum CaptureEvent {
    Started,
    Frame(Frame),
    /// No picture arrived: the page stopped painting before capture began.
    Stalled,
    Failed(String),
}

/// The socket, bound before Chrome starts.
pub(crate) struct CaptureServer {
    listener: TcpListener,
    token: String,
}

impl CaptureServer {
    pub async fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let mut token = [0_u8; 24];
        getrandom::fill(&mut token)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        Ok(Self {
            listener,
            token: token.iter().map(|byte| format!("{byte:02x}")).collect(),
        })
    }

    /// The address the extension dials.
    pub fn address(&self) -> Result<String> {
        Ok(format!(
            "ws://127.0.0.1:{}/?token={}",
            self.listener.local_addr()?.port(),
            self.token
        ))
    }

    /// Accepts the extension's connections until `ended`, each replacing the
    /// one before, and hands them to `channel`.
    pub fn serve(self, channel: CaptureChannel, tasks: &TaskTracker, ended: CancellationToken) {
        let connections = tasks.clone();
        tasks.spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = ended.cancelled() => break,
                    accepted = self.listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { continue };
                let token = self.token.clone();
                let channel = channel.clone();
                let ended = ended.clone();
                connections.spawn(async move {
                    let expected = format!("token={token}");
                    // Tungstenite's callback answers a refusal with a whole response.
                    #[allow(clippy::result_large_err)]
                    let authorized = |request: &Request, response: Response| {
                        if request.uri().query() == Some(expected.as_str()) {
                            Ok(response)
                        } else {
                            let mut refusal = ErrorResponse::new(None);
                            *refusal.status_mut() = StatusCode::FORBIDDEN;
                            Err(refusal)
                        }
                    };
                    let socket = tokio::select! {
                        _ = ended.cancelled() => return,
                        socket = tokio_tungstenite::accept_hdr_async(stream, authorized) => socket,
                    };
                    let Ok(socket) = socket else {
                        return;
                    };
                    // The channel ends with the environment, as this does.
                    let _ended = channel
                        .requests
                        .send(ChannelRequest::Connected(socket))
                        .await;
                });
            }
        });
    }
}

/// Requests waiting for the channel's owner; commands for a capture past
/// them are dropped, being superseded by the next.
const REQUESTS: usize = 64;
/// Messages from the extension the owner has not routed yet; a full queue
/// holds back the socket.
const INBOUND: usize = 16;
/// Commands the connection's writer has not sent yet. Acks, key frame
/// requests and encodings past them are dropped; a start or a stop that
/// finds no room ends the connection, whose captures fail and restart.
const EXTENSION_COMMANDS: usize = 64;

enum ChannelRequest {
    /// The extension connected.
    Connected(WebSocketStream<TcpStream>),
    Start {
        target: String,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        reply: oneshot::Sender<Result<(u32, mpsc::Receiver<CaptureEvent>)>>,
    },
    /// An ack, a key frame request or an encoding for a capture.
    Command(CaptureCommand),
}

/// What an extension connection's reader hands the owner.
enum Inbound {
    Frame(u32, Frame),
    Event(extension::CaptureEvent),
    /// The connection closed, or sent a message that did not decode.
    Ended,
}

/// The way to one environment's capture channel (`live-view.md` §
/// Capture): its owner keeps the extension's connection and the captures
/// running on it.
#[derive(Clone)]
pub(crate) struct CaptureChannel {
    requests: mpsc::Sender<ChannelRequest>,
    connected: watch::Receiver<bool>,
}

impl CaptureChannel {
    /// Starts the channel's owner; it ends with the environment.
    pub fn open(tasks: &TaskTracker, ended: CancellationToken) -> Self {
        let (requests, received) = mpsc::channel(REQUESTS);
        let (inbound, routed) = mpsc::channel(INBOUND);
        let (connected, watched) = watch::channel(false);
        let owner = Owner {
            tasks: tasks.clone(),
            ended: ended.clone(),
            connection: None,
            generation: 0,
            routes: HashMap::new(),
            next: 0,
            connected,
            inbound,
            closings: FuturesUnordered::new(),
        };
        tasks.spawn(owner.run(received, routed));
        Self {
            requests,
            connected: watched,
        }
    }

    /// Captures the tab of CDP `target` at `width` × `height` pixels, once
    /// the extension has connected, which it does as Chrome starts.
    pub async fn start(
        &self,
        target: &str,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        cancel: &CancellationToken,
    ) -> Result<Capture> {
        if let Some(reason) = unavailable() {
            return Err(BrowserError::UnsupportedCapability(reason.into()));
        }
        let mut connected = self.connected.clone();
        let connection = async {
            tokio::select! {
                _ = cancel.cancelled() => Err(BrowserError::Cancelled),
                connected = connected.wait_for(|connected| *connected) => {
                    connected.map(|_| ()).map_err(|_| BrowserError::Closed)
                }
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), connection)
            .await
            .map_err(|_| {
                BrowserError::Unavailable("the capture extension did not connect".into())
            })??;
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(ChannelRequest::Start {
                target: target.into(),
                width,
                height,
                fps,
                bitrate,
                reply,
            })
            .await
            .map_err(|_| BrowserError::Closed)?;
        let (id, events) = answer.await.map_err(|_| BrowserError::Closed)??;
        Ok(Capture {
            id,
            requests: self.requests.clone(),
            events,
        })
    }
}

/// One tab's capture; dropping it stops the capture, which the channel's
/// owner notices as the capture's queue closes.
pub(crate) struct Capture {
    id: u32,
    requests: mpsc::Sender<ChannelRequest>,
    pub events: mpsc::Receiver<CaptureEvent>,
}

impl Capture {
    /// The consumer has these frames; up to `window` may be in flight.
    pub fn ack(&self, sequence: u32, window: u32) {
        self.command(CaptureCommand::Ack {
            capture: self.id,
            sequence,
            window,
        });
    }

    pub fn key_frame(&self) {
        self.command(CaptureCommand::Keyframe { capture: self.id });
    }

    pub fn encoding(&self, bitrate: u32, fps: u32) {
        self.command(CaptureCommand::Encoding {
            capture: self.id,
            bitrate,
            fps,
        });
    }

    /// A command the next one supersedes, so a full channel drops it.
    fn command(&self, command: CaptureCommand) {
        let _dropped = self.requests.try_send(ChannelRequest::Command(command));
    }
}

/// The extension's connection as the owner keeps it.
struct Extension {
    commands: mpsc::Sender<CaptureCommand>,
    /// Ends the connection's reader and writer.
    stop: CancellationToken,
}

type Closing = Pin<Box<dyn Future<Output = u32> + Send>>;

struct Owner {
    tasks: TaskTracker,
    ended: CancellationToken,
    connection: Option<Extension>,
    /// Counts the connections, so messages of a replaced one are ignored.
    generation: u64,
    routes: HashMap<u32, mpsc::Sender<CaptureEvent>>,
    next: u32,
    connected: watch::Sender<bool>,
    inbound: mpsc::Sender<(u64, Inbound)>,
    /// Each capture's queue closing, which stops the capture.
    closings: FuturesUnordered<Closing>,
}

impl Owner {
    async fn run(
        mut self,
        mut requests: mpsc::Receiver<ChannelRequest>,
        mut inbound: mpsc::Receiver<(u64, Inbound)>,
    ) {
        let ended = self.ended.clone();
        loop {
            tokio::select! {
                biased;
                _ = ended.cancelled() => break,
                Some(capture) = self.closings.next() => self.stopped(capture),
                message = inbound.recv() => match message {
                    Some((generation, message)) if generation == self.generation => {
                        self.routed(message);
                    }
                    // A replaced connection's last words.
                    Some(_) => {}
                    None => break,
                },
                request = requests.recv() => match request {
                    Some(request) => self.request(request),
                    None => break,
                },
            }
        }
        self.disconnect();
    }

    fn request(&mut self, request: ChannelRequest) {
        match request {
            ChannelRequest::Connected(socket) => self.connected(socket),
            ChannelRequest::Start {
                target,
                width,
                height,
                fps,
                bitrate,
                reply,
            } => {
                let started = self.start(target, width, height, fps, bitrate);
                // A requester that left drops the capture, which stops it.
                let _left = reply.send(started);
            }
            ChannelRequest::Command(command) => {
                if let Some(connection) = &self.connection {
                    let _dropped = connection.commands.try_send(command);
                }
            }
        }
    }

    /// Takes a new connection. The previous connection's captures fail
    /// first, so its late teardown cannot fail their replacements.
    fn connected(&mut self, socket: WebSocketStream<TcpStream>) {
        self.disconnect();
        self.generation += 1;
        let (commands, outgoing) = mpsc::channel(EXTENSION_COMMANDS);
        let stop = self.ended.child_token();
        let (sink, stream) = socket.split();
        self.tasks.spawn(write(sink, outgoing, stop.clone()));
        self.tasks.spawn(read(
            stream,
            self.inbound.clone(),
            self.generation,
            stop.clone(),
        ));
        self.connection = Some(Extension { commands, stop });
        self.connected.send_replace(true);
    }

    fn start(
        &mut self,
        target: String,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<(u32, mpsc::Receiver<CaptureEvent>)> {
        let Some(connection) = &self.connection else {
            return Err(BrowserError::Unavailable(
                "the capture extension is not connected".into(),
            ));
        };
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        let command = CaptureCommand::Start {
            capture: id,
            target,
            width,
            height,
            fps,
            bitrate,
        };
        if connection.commands.try_send(command).is_err() {
            self.disconnect();
            return Err(BrowserError::Unavailable(
                "the capture extension is not keeping up".into(),
            ));
        }
        let (sender, events) = mpsc::channel(FRAME_QUEUE);
        self.routes.insert(id, sender.clone());
        self.closings.push(Box::pin(async move {
            sender.closed().await;
            id
        }));
        Ok((id, events))
    }

    /// A capture's consumer dropped it: the extension stops it.
    fn stopped(&mut self, capture: u32) {
        if self.routes.remove(&capture).is_none() {
            // It failed with a connection that is gone.
            return;
        }
        if let Some(connection) = &self.connection
            && connection
                .commands
                .try_send(CaptureCommand::Stop { capture })
                .is_err()
        {
            self.disconnect();
        }
    }

    fn routed(&mut self, message: Inbound) {
        match message {
            // A consumer that is behind loses the newest pictures, not the socket.
            Inbound::Frame(capture, frame) => self.deliver(capture, CaptureEvent::Frame(frame)),
            Inbound::Event(event) => match event {
                extension::CaptureEvent::Ready {} | extension::CaptureEvent::Stopped { .. } => {}
                extension::CaptureEvent::Started { capture } => {
                    self.deliver(capture, CaptureEvent::Started)
                }
                extension::CaptureEvent::Stalled { capture } => {
                    self.deliver(capture, CaptureEvent::Stalled)
                }
                extension::CaptureEvent::Error { capture, message } => {
                    self.deliver(capture, CaptureEvent::Failed(message))
                }
            },
            Inbound::Ended => self.disconnect(),
        }
    }

    /// Hands an event to the capture's consumer, if it still runs; one that
    /// is behind loses it.
    fn deliver(&self, capture: u32, event: CaptureEvent) {
        if let Some(events) = self.routes.get(&capture) {
            let _behind = events.try_send(event);
        }
    }

    /// Ends the connection; its captures fail, and their streams restart
    /// them once the extension connects again.
    fn disconnect(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.stop.cancel();
        }
        self.connected.send_replace(false);
        for (_, events) in self.routes.drain() {
            let _gone = events.try_send(CaptureEvent::Failed(
                "capture extension disconnected".into(),
            ));
        }
    }
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>;
type Source = futures_util::stream::SplitStream<WebSocketStream<TcpStream>>;

/// Sends the owner's commands to the extension until the connection ends.
async fn write(
    mut sink: Sink,
    mut outgoing: mpsc::Receiver<CaptureCommand>,
    stop: CancellationToken,
) {
    loop {
        let command = tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            command = outgoing.recv() => command,
        };
        let Some(command) = command else {
            break;
        };
        let text = serde_json::to_string(&command).expect("capture commands serialize");
        let sent = tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            sent = sink.send(Message::text(text)) => sent,
        };
        // A connection that broke also ends its reader, which tells the owner.
        if sent.is_err() {
            break;
        }
    }
}

/// Decodes what the extension sends and hands it to the owner. A message
/// that does not decode ends the connection and its captures, and is logged
/// (`live-view.md` § Capture).
async fn read(
    mut stream: Source,
    inbound: mpsc::Sender<(u64, Inbound)>,
    generation: u64,
    stop: CancellationToken,
) {
    loop {
        let message = tokio::select! {
            biased;
            _ = stop.cancelled() => return,
            message = stream.next() => message,
        };
        let decoded = match message {
            Some(Ok(Message::Binary(bytes))) => frame(&bytes).map(Some),
            Some(Ok(Message::Text(text))) => {
                extension::CaptureEvent::decode(&text).map(|event| Some(Inbound::Event(event)))
            }
            Some(Ok(Message::Close(_)) | Err(_)) | None => break,
            // Pings and pongs.
            Some(Ok(_)) => Ok(None),
        };
        let message = match decoded {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!("capture extension message refused: {error}");
                break;
            }
        };
        let sent = tokio::select! {
            biased;
            _ = stop.cancelled() => return,
            sent = inbound.send((generation, message)) => sent,
        };
        // The owner ended with the environment.
        if sent.is_err() {
            return;
        }
    }
    let _ended = inbound.send((generation, Inbound::Ended)).await;
}

fn frame(bytes: &Bytes) -> std::result::Result<Inbound, DecodeError> {
    let (header, data) = extension::FrameHeader::split(bytes)?;
    Ok(Inbound::Frame(
        header.capture,
        Frame {
            sequence: header.sequence,
            key: header.key,
            timestamp: header.timestamp,
            width: header.width,
            height: header.height,
            data: bytes.slice_ref(data),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cpu_with_sme_but_no_sve_cannot_capture() {
        // AT_HWCAP2 of an Apple M4 guest, then of the same guest booted with
        // arm64.nosme; neither has SVE in AT_HWCAP.
        let hwcap = 0xefff_ffff & !(1 << 22);
        assert!(sme_without_sve(hwcap, 0x1a0_3fb2_6181));
        assert!(!sme_without_sve(hwcap, 0x32_6181));
        // A CPU with both, such as a server with SVE and SME.
        assert!(!sme_without_sve(hwcap | 1 << 22, 0x1a0_3fb2_6181));
    }
}
