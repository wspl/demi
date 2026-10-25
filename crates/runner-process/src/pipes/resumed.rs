//! A pipe the runner reads that resumes after its connection is lost: a
//! call's standard output
//! (`crates/runner-protocol/src/wire/pipe_frames.rs`).
//!
//! The pipe is a WebSocket of its own whose messages are the frames of a
//! session (`crates/runner-protocol/src/wire/session.rs`). A connection opens
//! with the count of messages the runner delivered, and the backend writes the
//! pipe again from there, so a connection that fails ends nothing: the runner
//! opens another until the pipe ends or the call stops.
//!
//! - A network that fails a connection is tried again every second, as long as
//!   the call lives; the call's own end, or the link's session's, stops it.
//! - A refusal, which an ended session also answers, is tried again for a
//!   minute, which a runner still reconnecting its link needs, and then ends
//!   the pipe.
//! - The runner counts a message as received once it delivered it, so what it
//!   holds is the message its reader is handed and what the connection buffers.
//! - The runner closes the WebSocket itself once it reads no more, at the end
//!   or when its reader let go or the call stopped; the backend keeps the
//!   pipe's unconfirmed bytes for a connection that was lost instead.

use bytes::Bytes;
use demi_runner_protocol::wire::{self, PipeFrame, Read};
use futures_util::{SinkExt, StreamExt, stream::BoxStream};
use std::io;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest,
    http::{HeaderValue, header::AUTHORIZATION},
    protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
};
use tokio_util::sync::CancellationToken;

use super::moves::{self, Relayed};
use super::{Attempt, PipeClient, RECONNECT_FOR, RECONNECT_PAUSE, cancelled, socket_error};

/// How one connection of the pipe ended.
enum Ended {
    /// The end frame came: the pipe is over.
    Whole,
    /// Whoever reads the pipe let go of it.
    Dropped,
}

impl PipeClient {
    /// Reads `pipe` to its end, through as many connections as it takes:
    /// the bytes of its data frames, and of the direct channels its moves
    /// name, in order, once each. An end frame with a cause is that cause's
    /// error; a pipe the backend no longer knows, after a minute of asking,
    /// is a broken pipe.
    pub async fn resumed(
        &self,
        pipe: &wire::PipeRef,
        cancel: CancellationToken,
    ) -> io::Result<BoxStream<'static, io::Result<Bytes>>> {
        // A path that is no pipe's fails here, not after a minute.
        self.url(&pipe.url)?;
        let (sender, receiver) = mpsc::channel(1);
        let client = self.clone();
        let path = pipe.url.clone();
        tokio::spawn(async move {
            client.resume(&path, &sender, &cancel).await;
        });
        let relayed = futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        })
        .boxed();
        Ok(moves::follow(pipe.id.clone(), relayed, self.pairs()))
    }

    async fn resume(
        &self,
        path: &str,
        sender: &mpsc::Sender<io::Result<Relayed>>,
        cancel: &CancellationToken,
    ) {
        let mut session = wire::Session::new();
        let mut refused_since: Option<tokio::time::Instant> = None;
        let mut connections = 0u32;
        loop {
            connections += 1;
            let ended = self
                .connection(path, &mut session, sender, cancel, &mut refused_since)
                .await;
            let failure = match ended {
                Ok(Ended::Whole | Ended::Dropped) => return,
                Err(failure) => failure,
            };
            let pause = match failure {
                Attempt::Lost(error) => {
                    tracing::debug!(event = "pipe_request.retried", cause = "connection_lost", connections, %error, "a pipe's connection was lost and goes again");
                    true
                }
                Attempt::Refused(error) => {
                    let since = *refused_since.get_or_insert_with(tokio::time::Instant::now);
                    if since.elapsed() >= RECONNECT_FOR {
                        let _gone = sender.send(Err(error)).await;
                        return;
                    }
                    true
                }
            };
            if pause {
                tokio::select! {
                    () = cancel.cancelled() => {
                        let _gone = sender.send(Err(cancelled())).await;
                        return;
                    }
                    () = tokio::time::sleep(RECONNECT_PAUSE) => {}
                }
            }
        }
    }

    /// One connection of the pipe: opens the socket, says how many messages
    /// were delivered, and delivers the rest as they come.
    async fn connection(
        &self,
        path: &str,
        session: &mut wire::Session,
        sender: &mpsc::Sender<io::Result<Relayed>>,
        cancel: &CancellationToken,
        refused_since: &mut Option<tokio::time::Instant>,
    ) -> Result<Ended, Attempt> {
        let mut url = self.url(path).map_err(Attempt::Refused)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| Attempt::Refused(io::Error::other("invalid WebSocket scheme")))?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| Attempt::Refused(io::Error::other(error)))?;
        let authorization = HeaderValue::from_str(&format!(
            "Bearer {}",
            self.token().map_err(Attempt::Refused)?.expose()
        ))
        .map_err(|error| Attempt::Refused(io::Error::other(error)))?;
        request.headers_mut().insert(AUTHORIZATION, authorization);
        let config = WebSocketConfig::default()
            .max_message_size(Some(wire::PIPE_SESSION_MESSAGE_BYTES))
            .max_frame_size(Some(wire::PIPE_SESSION_MESSAGE_BYTES));
        let connecting = demi_command_sdk::descriptors::retry(cancel, || async {
            self.backend
                .websocket(request.clone(), config)
                .await
                .map_err(|error| match error {
                    // A refused handshake reached the backend.
                    tokio_tungstenite::tungstenite::Error::Http(_) => {
                        Attempt::Refused(socket_error(error))
                    }
                    error => Attempt::Lost(socket_error(error)),
                })
        });
        let (mut socket, _) = tokio::select! {
            () = cancel.cancelled() => return Err(Attempt::Refused(cancelled())),
            connected = tokio::time::timeout(self.connect_timeout, connecting) => connected
                .unwrap_or_else(|_| {
                    Err(Attempt::Lost(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "pipe did not open",
                    )))
                })?,
        };
        // The backend took the pipe: refusals so far were of a pipe it was not
        // ready to hand out.
        *refused_since = None;
        let lost = |error: tokio_tungstenite::tungstenite::Error| Attempt::Lost(socket_error(error));
        // The backend writes again what was not handled.
        session.read_again();
        let hello = wire::own_frame(session.received().to_be_bytes().to_vec());
        socket
            .send(Message::Binary(hello.into()))
            .await
            .map_err(lost)?;
        loop {
            let due = session.ack_due();
            let message = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    close(&mut socket).await;
                    return Err(Attempt::Refused(cancelled()));
                }
                () = async {
                    match due {
                        Some(due) => tokio::time::sleep_until(due).await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(ack) = session.ack_frame() {
                        socket.send(Message::Binary(ack.into())).await.map_err(lost)?;
                    }
                    continue;
                }
                // The backend pings while the pipe is quiet, so a connection
                // that stays silent longer is lost.
                message = tokio::time::timeout(wire::LINK_SILENCE, socket.next()) => message,
            };
            let message = match message {
                Err(_) => {
                    return Err(Attempt::Lost(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the pipe was silent",
                    )));
                }
                Ok(None) => {
                    return Err(Attempt::Lost(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "the pipe's connection ended without its end",
                    )));
                }
                Ok(Some(message)) => message.map_err(lost)?,
            };
            match message {
                Message::Binary(bytes) => {
                    let read = session
                        .read(&bytes)
                        .map_err(|error| Attempt::Refused(io::Error::new(io::ErrorKind::InvalidData, error)))?;
                    let payload = match read {
                        // The backend's own opening: nothing the runner wrote
                        // is waiting for it.
                        Read::Own(_) | Read::Duplicate | Read::Ack => continue,
                        // Nothing in it, and counted in its turn.
                        Read::Skipped => {
                            session.handled();
                            continue;
                        }
                        Read::Message(payload) => payload,
                    };
                    let frame = wire::decode_message(payload)
                        .map_err(|error| Attempt::Refused(io::Error::new(io::ErrorKind::InvalidData, error)))?;
                    match frame {
                        PipeFrame::Data(bytes) => {
                            if sender.send(Ok(Relayed::Bytes(bytes))).await.is_err() {
                                close(&mut socket).await;
                                return Ok(Ended::Dropped);
                            }
                            session.handled();
                        }
                        PipeFrame::Moved { to, at, source } => {
                            if sender.send(Ok(Relayed::Moved { to, at, source })).await.is_err() {
                                close(&mut socket).await;
                                return Ok(Ended::Dropped);
                            }
                            session.handled();
                        }
                        PipeFrame::Idle => session.handled(),
                        PipeFrame::End(end) => {
                            if let Err(cause) = end {
                                let _gone = sender.send(Err(io::Error::other(cause))).await;
                            }
                            // Confirmed at once, once handed on: the backend
                            // keeps the pipe until it hears this.
                            session.handled();
                            if let Some(ack) = session.ack_frame() {
                                let _sent = socket.send(Message::Binary(ack.into())).await;
                            }
                            close(&mut socket).await;
                            return Ok(Ended::Whole);
                        }
                    }
                }
                Message::Close(_) => {
                    return Err(Attempt::Lost(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "the backend closed the pipe before its end",
                    )));
                }
                Message::Ping(_) | Message::Pong(_) => {}
                _ => {
                    return Err(Attempt::Refused(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "a pipe carries binary messages",
                    )));
                }
            }
        }
    }
}

/// Closes the connection on purpose: the runner reads the pipe no more, which
/// the backend takes as the pipe's end, unlike a connection that was lost.
async fn close<S>(socket: &mut S)
where
    S: futures_util::Sink<Message> + Unpin,
{
    let frame = CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    };
    let _closed = socket.send(Message::Close(Some(frame))).await;
}
