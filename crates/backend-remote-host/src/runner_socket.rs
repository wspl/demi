//! A runner's WebSocket as the connection's driver reads and ends it
//! (`runner.md` § Connection and identity): for the backend's runner route
//! and the test fixture's edge alike.

use axum::extract::ws::{CloseFrame, Message, close_code};
use demi_runner_protocol::wire;

use crate::link::{LinkEnd, SocketEnd};

/// What the driver reads of one message of a runner's socket: a binary
/// message's frame; the runner's refusal of a message of the backend's, as
/// its close frame names it; the socket's failure; or nothing, for a ping,
/// a pong or another close.
pub fn socket_frame(message: Result<Message, axum::Error>) -> Option<Result<Vec<u8>, SocketEnd>> {
    match message {
        Ok(Message::Binary(frame)) => Some(Ok(frame.to_vec())),
        Ok(Message::Text(_)) => Some(Err(SocketEnd::Broken(
            "the runner sent a text frame".to_owned(),
        ))),
        Ok(Message::Close(Some(frame))) if frame.code == close_code::INVALID => {
            Some(Err(SocketEnd::Refused(frame.reason.to_string())))
        }
        Ok(Message::Ping(_) | Message::Pong(_) | Message::Close(_)) => None,
        Err(error) => Some(Err(SocketEnd::Broken(error.to_string()))),
    }
}

/// The close frame that ends a runner's socket once its connection ended
/// with `end`: a refusal names the runner's message the backend cannot
/// decode, as the runner's own refusal does; any other end has none.
pub fn close_frame(end: &LinkEnd) -> Option<CloseFrame> {
    match end {
        LinkEnd::Refused(refusal) => Some(CloseFrame {
            code: close_code::INVALID,
            reason: wire::close_reason(refusal).into(),
        }),
        LinkEnd::Closed(_) | LinkEnd::RunnerRefused(_) | LinkEnd::Disconnected(_) => None,
    }
}
