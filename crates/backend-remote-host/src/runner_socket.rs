//! A runner's WebSocket as the link reads and ends it
//! (`crates/runner-protocol/src/wire/mod.rs`): for the backend's runner route
//! and the test fixture's edge alike.

use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use demi_shared_link::{Handshake, Liveness, Received, Refusal, Side, Wire};
use demi_runner_protocol::wire;
use futures_util::{SinkExt as _, StreamExt as _, future::ready};

use crate::link::LinkEnd;

/// Takes a runner's socket, as the backend's end of the link: its hello and
/// the answer to it are messages of the socket, and the link's session starts
/// on it once the hello is accepted ([`Handshake::start`]).
pub fn open_link(socket: WebSocket, liveness: Option<Liveness>) -> Handshake {
    let (sink, stream) = socket.split();
    let incoming = stream.map(received);
    let outgoing = sink.with(|message| ready(Ok::<_, axum::Error>(sent(message))));
    Handshake::open(incoming, outgoing, Side::Backend, liveness)
}

/// What the link reads of one message of a runner's socket: a binary
/// message's bytes; the runner's refusal of a message of the backend's, as
/// its close frame names it; or the socket's failure; or nothing, for a
/// ping, a pong or another close.
fn received(message: Result<Message, axum::Error>) -> Result<Received, String> {
    match message {
        Ok(Message::Binary(bytes)) => Ok(Received::Binary(bytes.to_vec())),
        Ok(Message::Text(_)) => Err("the runner sent a text frame".to_owned()),
        Ok(Message::Close(Some(frame))) if frame.code == close_code::INVALID => {
            Ok(Received::Close(Some(Refusal(frame.reason.to_string()))))
        }
        Ok(Message::Close(_)) => Ok(Received::Close(None)),
        Ok(Message::Ping(_) | Message::Pong(_)) => Ok(Received::Nothing),
        Err(error) => Err(error.to_string()),
    }
}

/// The message the socket sends for what the link gives it.
fn sent(message: Wire) -> Message {
    match message {
        Wire::Binary(bytes) => Message::Binary(bytes.into()),
        Wire::Close(Some(reason)) => Message::Close(Some(CloseFrame {
            code: close_code::INVALID,
            reason: wire::close_reason(&reason).into(),
        })),
        Wire::Close(None) => Message::Close(None),
    }
}

/// The refusal the close of a runner's link carries once the connection
/// ended with `end`: a refusal names the runner's message the backend cannot
/// decode, as the runner's own refusal does; any other end has none.
pub fn refusal(end: &LinkEnd) -> Option<String> {
    match end {
        LinkEnd::Refused(refusal) => Some(refusal.clone()),
        LinkEnd::Closed(_) | LinkEnd::RunnerRefused(_) | LinkEnd::Disconnected(_) => None,
    }
}
