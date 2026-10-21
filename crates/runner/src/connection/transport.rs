//! The backend WebSocket as the runner's end of the link
//! (`crates/shared-link/src/lib.rs`): the hello and the backend's answer are
//! ordinary WebSocket messages, and the link's session starts on the socket
//! once the backend accepted the hello. One task reads the messages of every
//! stream, each stream's in the order the backend sent them, into a queue of
//! eight for the connection's owner, whose replies go out on the streams of
//! their own.
//!
//! - When the queue is full the reader waits, so the backend's streams fill
//!   and its writes wait too. A message keeps its place in its stream until the
//!   owner lets go of it, so what the owner holds back, a command's input
//!   that nothing reads, stops the link reading that stream and no other. Only a message that breaks the protocol closes
//!   the connection (the limits and why: `demi_runner_protocol::wire`).
//! - A message the runner cannot decode ends the connection with a close frame
//!   naming its type and the decoding error.
//! - A connection the runner ends on purpose, as a drain or a stop does, ends
//!   in order: the queued messages, a close frame, and up to five seconds for
//!   the backend's. The backend sees the runner leave rather than lose it.
//! - The link pings the backend and counts a connection it hears nothing on for
//!   `RUNNER_PING_TIMEOUT` as lost, as one the network dropped without closing
//!   it is.

use std::{io, time::Duration};

use demi_command_protocol::host_target;
use demi_runner_process::backend::Backend;
use demi_shared_link::{
    Handshake, Held, Liveness, Outgoing, Received, Refusal, Side, SocketEnd, Wire,
};
use demi_runner_protocol::{
    release::{RELEASE_HEADER, RunnerUpdate, TARGET_HEADER, TOKEN_HEADER},
    values::DeviceToken,
    wire::{self, Inbound},
};
use futures_util::{SinkExt, StreamExt, future::ready};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Error, Message,
        client::IntoClientRequest,
        http::{HeaderValue, StatusCode},
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};
use tokio_util::sync::CancellationToken;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const QUEUE_MESSAGES: usize = 8;

pub struct Transport {
    /// What the runner sends: each message on the stream of its command,
    /// request or the connection.
    pub outgoing: Outgoing,
    /// The backend's messages, each stream's in the order it sent them.
    pub input: mpsc::Receiver<(Inbound, Held)>,
    cancel: CancellationToken,
    /// The socket until the hello is sent, which the reading task then takes.
    handshake: Option<(Handshake, mpsc::Sender<(Inbound, Held)>)>,
    owner: Option<JoinHandle<io::Result<()>>>,
}

/// The backend's runner WebSocket: its `ws`/`wss` form, at `/api/runner`
/// unless the URL names another path.
pub fn socket_url(backend: &Backend) -> io::Result<url::Url> {
    let mut url = backend.url().url().clone();
    let scheme = match backend.origin()?.scheme() {
        "http" => "ws",
        _ => "wss",
    };
    url.set_scheme(scheme)
        .map_err(|_| io::Error::other("invalid WebSocket scheme"))?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/api/runner");
    }
    Ok(url)
}

/// What asking for the socket gave: the socket, or the release the backend
/// requires first (`crates/runner/src/update.rs`).
pub enum Connected {
    Open(Transport),
    Update(RunnerUpdate),
}

impl Transport {
    /// Opens the backend's socket for a runner of `release`, which the
    /// backend checks before it opens it; a paired runner names its device
    /// with `token`.
    pub async fn connect(
        backend: &Backend,
        release: &str,
        token: Option<&DeviceToken>,
        cancel: CancellationToken,
    ) -> io::Result<Connected> {
        let url = socket_url(backend)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(io::Error::other)?;
        let headers = request.headers_mut();
        headers.insert(
            RELEASE_HEADER,
            HeaderValue::from_str(release).map_err(io::Error::other)?,
        );
        headers.insert(TARGET_HEADER, HeaderValue::from_static(host_target()));
        if let Some(token) = token {
            let mut value = HeaderValue::from_str(token.expose()).map_err(io::Error::other)?;
            value.set_sensitive(true);
            headers.insert(TOKEN_HEADER, value);
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(wire::MAX_MESSAGE_BYTES))
            .max_frame_size(Some(wire::MAX_MESSAGE_BYTES));
        let connected = tokio::select! {
            _ = cancel.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted, "connection cancelled")),
            result = tokio::time::timeout(HANDSHAKE_TIMEOUT, backend.websocket(request, config)) => {
                result.map_err(io::Error::other)?
            }
        };
        match connected {
            Ok((socket, _)) => Ok(Connected::Open(Self::from_socket(socket, cancel))),
            Err(Error::Http(response)) if response.status() == StatusCode::CONFLICT => {
                let body = response.body().as_deref().unwrap_or_default();
                let update = RunnerUpdate::decode(body)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                Ok(Connected::Update(update))
            }
            Err(error) => Err(io::Error::other(error)),
        }
    }

    pub fn from_socket<S>(socket: WebSocketStream<S>, cancel: CancellationToken) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let cancel = cancel.child_token();
        let (writer, reader) = socket.split();
        let incoming = reader.map(received);
        let outgoing = writer.with(|wire| ready(Ok::<_, Error>(sent(wire))));
        let handshake =
            Handshake::open(incoming, outgoing, Side::Runner, Some(Liveness::default()));
        let outgoing = handshake.outgoing();
        let (messages, input) = mpsc::channel(QUEUE_MESSAGES);
        Self {
            outgoing,
            input,
            cancel,
            handshake: Some((handshake, messages)),
            owner: None,
        }
    }

    /// Sends the hello, as one message of the socket, and starts reading the
    /// backend's answers.
    pub async fn hello(&mut self, hello: wire::Frame) -> io::Result<()> {
        let Some((mut handshake, messages)) = self.handshake.take() else {
            return Err(io::Error::other("the hello was sent already"));
        };
        handshake
            .write(hello.into_bytes())
            .await
            .map_err(ended)?;
        self.owner = Some(tokio::spawn(read(
            handshake,
            messages,
            self.outgoing.clone(),
            self.cancel.clone(),
        )));
        Ok(())
    }

    pub async fn close(mut self) -> io::Result<()> {
        self.cancel.cancel();
        match self.owner.take() {
            Some(owner) => owner.await.map_err(io::Error::other)?,
            None => Ok(()),
        }
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

/// What the link reads of a message of the backend's socket.
fn received(message: Result<Message, Error>) -> Result<Received, String> {
    match message {
        Ok(Message::Binary(bytes)) => Ok(Received::Binary(bytes.to_vec())),
        // The backend closed it over a message of this runner's that it
        // cannot decode, and says which.
        Ok(Message::Close(Some(frame))) if frame.code == CloseCode::Invalid => {
            Ok(Received::Close(Some(Refusal(frame.reason.to_string()))))
        }
        Ok(Message::Close(_)) => Ok(Received::Close(None)),
        Ok(Message::Ping(_) | Message::Pong(_)) => Ok(Received::Nothing),
        Ok(_) => Err("runner requires binary WebSocket messages".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// The message the socket sends for what the link gives it.
fn sent(message: Wire) -> Message {
    match message {
        Wire::Binary(bytes) => Message::Binary(bytes.into()),
        Wire::Close(Some(reason)) => Message::Close(Some(CloseFrame {
            code: CloseCode::Invalid,
            reason: wire::close_reason(&reason).to_owned().into(),
        })),
        Wire::Close(None) => Message::Close(Some(CloseFrame {
            code: CloseCode::Away,
            reason: "runner stopping".into(),
        })),
    }
}

/// The error a socket's end is for the connection that read it.
fn ended(end: SocketEnd) -> io::Error {
    match end {
        SocketEnd::Refused(reason) => {
            io::Error::new(io::ErrorKind::InvalidData, format!("the backend {reason}"))
        }
        SocketEnd::Unresponsive => io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "the backend did not answer for {} seconds",
                wire::RUNNER_PING_TIMEOUT.as_secs()
            ),
        ),
        SocketEnd::Broken(error) => io::Error::other(error),
    }
}

/// Reads the backend's messages into `messages`: its answers to the hello
/// first, as the socket's own messages, and then those of the session that
/// starts once the backend accepted the hello. Ends with the refusal of a
/// message this side cannot decode, which the connection then closes over,
/// or with the link's end.
async fn read(
    mut handshake: Handshake,
    messages: mpsc::Sender<(Inbound, Held)>,
    outgoing: Outgoing,
    stopped: CancellationToken,
) -> io::Result<()> {
    let mut incoming = loop {
        let next = tokio::select! {
            () = stopped.cancelled() => {
                handshake.close(None).await;
                return Ok(());
            }
            next = handshake.read() => next,
        };
        let bytes = match next {
            None => return Ok(()),
            Some(Err(end)) => return Err(ended(end)),
            Some(Ok(bytes)) => bytes,
        };
        let message = match wire::decode::<Inbound>(&bytes) {
            Ok(message) => message,
            Err(error) => {
                let refusal = wire::refusal(&bytes, &error);
                handshake.close(Some(refusal.clone())).await;
                return Err(io::Error::new(io::ErrorKind::InvalidData, refusal));
            }
        };
        // The session starts with the welcome of the backend, or with the
        // token of a claim; a refusal ends the socket; a claim that waits
        // goes on.
        let starts = matches!(message, Inbound::HelloOk { .. } | Inbound::Claimed { .. });
        let refused = matches!(message, Inbound::HelloError { .. });
        if messages.send((message, Held::nothing())).await.is_err() {
            return Ok(());
        }
        if starts {
            break handshake.start().incoming;
        }
        if refused {
            return Ok(());
        }
    };
    loop {
        let next = tokio::select! {
            () = stopped.cancelled() => {
                // The runner ends this connection on purpose: what is
                // queued goes first, then a close frame, and the connection
                // ends with the backend's answer, so the backend sees the
                // runner leave rather than lose it
                // (`crates/runner/src/connection/mod.rs`).
                outgoing.close(None).await;
                return Ok(());
            }
            next = incoming.next_held() => next,
        };
        match next {
            None => return Ok(()),
            Some(Err(end)) => return Err(ended(end)),
            Some(Ok((bytes, held))) => {
                let message = match wire::decode(&bytes) {
                    Ok(message) => message,
                    Err(error) => {
                        let refusal = wire::refusal(&bytes, &error);
                        outgoing.close(Some(refusal.clone())).await;
                        return Err(io::Error::new(io::ErrorKind::InvalidData, refusal));
                    }
                };
                // A full queue stops the reading until the connection's
                // owner takes a message.
                if messages.send((message, held)).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        // Cancellation wakes the owner on every early return; explicit close joins it.
        self.cancel.cancel();
    }
}
