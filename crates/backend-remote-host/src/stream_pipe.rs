//! A stream pipe's source end (`runner.md` § Host operations): the runner
//! sends the bytes of a network or service stream's output in binary
//! messages of a WebSocket, which the backend's pipe route accepted, and ends
//! the pipe with a normal close frame, or fails it with one that carries the
//! error. The route and the test fixture's edge both relay through here.

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::response::Response;
use demi_runner_protocol::wire::{STREAM_PIPE_MESSAGE_BYTES, close_reason};
use futures_util::{SinkExt as _, StreamExt as _};

use crate::DeviceSource;

/// Takes the upgrade of `source`'s stream pipe: its binary messages are the
/// bytes, forwarded as the sink takes them; the runner's normal close frame
/// ends the pipe, and any other close, or a connection lost without one,
/// fails it. The backend's close frame is the pipe's outcome, which comes
/// early when the sink stops taking bytes or the pipe fails while the runner
/// still sends: normal once the sink drained it, an error with why it failed.
/// An upgrade that never completes drops the source, which fails the pipe.
pub fn accept_stream_pipe(upgrade: WebSocketUpgrade, source: DeviceSource) -> Response {
    upgrade
        .max_message_size(STREAM_PIPE_MESSAGE_BYTES)
        .max_frame_size(STREAM_PIPE_MESSAGE_BYTES)
        .on_upgrade(move |socket| relay(socket, source))
}

async fn relay(socket: WebSocket, source: DeviceSource) {
    let (mut to_runner, from_runner) = socket.split();
    let body = futures_util::stream::unfold(Some(from_runner), |reader| async move {
        let mut reader = reader?;
        loop {
            let failure = match reader.next().await {
                Some(Ok(Message::Binary(bytes))) => return Some((Ok(bytes), Some(reader))),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Close(None))) => return None,
                Some(Ok(Message::Close(Some(frame)))) if frame.code == close_code::NORMAL => {
                    return None;
                }
                Some(Ok(Message::Close(Some(frame)))) => {
                    format!("the runner closed the stream pipe ({}): {}", frame.code, frame.reason)
                }
                Some(Ok(Message::Text(_))) => "a stream pipe carries binary messages".to_owned(),
                Some(Err(error)) => error.to_string(),
                None => "the stream pipe's connection ended without a close frame".to_owned(),
            };
            return Some((Err(failure), None));
        }
    });
    let frame = match source.pump(body).await {
        Ok(()) => CloseFrame {
            code: close_code::NORMAL,
            reason: "drained".into(),
        },
        Err(failure) => CloseFrame {
            code: close_code::ERROR,
            reason: close_reason(&failure.to_string()).into(),
        },
    };
    // A runner that went meanwhile hears nothing, and its pipe has ended
    // either way. After the runner's own close the WebSocket refuses this
    // frame: it answers the runner's close itself, with the runner's code,
    // once the sink is closed, which flushes that answer; without it the
    // runner sees its connection end with no closing handshake.
    let _sent = to_runner.send(Message::Close(Some(frame))).await;
    let _closed = to_runner.close().await;
}
