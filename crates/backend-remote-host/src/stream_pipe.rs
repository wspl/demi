//! A stream pipe's source end (`crates/runner-protocol/src/wire/pipe_frames.rs`): the runner
//! sends the bytes of a network or resident stream's output in binary
//! messages of a WebSocket, which the backend's pipe route accepted, and ends
//! the pipe with a normal close frame, or fails it with one that carries the
//! error. The route and the test fixture's edge both relay through here.
//!
//! A call's standard output goes the other way, over a WebSocket that resumes:
//! [`accept_call_sink`] writes the pipe to the runner in the messages of a
//! session, and a connection that replaces another takes the session over and
//! writes again what the runner did not receive.

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::response::Response;
use demi_runner_protocol::wire::{
    self, PIPE_IDLE, PIPE_SESSION_MESSAGE_BYTES, PipeMove, PipeSink, Read, STREAM_PIPE_MESSAGE_BYTES,
    close_reason,
};
use futures_util::{SinkExt as _, StreamExt as _};
use std::{sync::Arc, time::Duration};

use crate::{DeviceSource, SinkSession, SourceMessage};

/// How long a runner has to say how many messages it delivered once its
/// WebSocket opened.
const OPENING: Duration = Duration::from_secs(15);

/// Takes the upgrade of `source`'s stream pipe: its binary messages are the
/// bytes, forwarded as the sink takes them, and its text messages its moves
/// ([`PipeMove`]). A pipe that may move to the direct channel tells the runner
/// its sink's device in a text message ([`PipeSink`]).
///
/// - The runner's normal close frame ends the pipe.
/// - Any other close, or a connection lost without one, fails the pipe.
///
/// The backend's close frame is the pipe's outcome, which comes early when the
/// sink stops taking bytes or the pipe fails while the runner still sends:
/// normal once the sink drained it, an error with why it failed. An upgrade
/// that never completes drops the source, which fails the pipe.
pub fn accept_stream_pipe(upgrade: WebSocketUpgrade, source: DeviceSource) -> Response {
    upgrade
        .max_message_size(STREAM_PIPE_MESSAGE_BYTES)
        .max_frame_size(STREAM_PIPE_MESSAGE_BYTES)
        .on_upgrade(move |socket| relay(socket, source))
}

async fn relay(socket: WebSocket, source: DeviceSource) {
    let (mut to_runner, from_runner) = socket.split();
    let mut sink = source.sink();
    let body = futures_util::stream::unfold(Some(from_runner), |reader| async move {
        let mut reader = reader?;
        loop {
            let failure = match reader.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    return Some((Ok(SourceMessage::Bytes(bytes)), Some(reader)));
                }
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<PipeMove>(&text) {
                    Ok(PipeMove { to, at }) => {
                        return Some((Ok(SourceMessage::Moved { to, at }), Some(reader)));
                    }
                    Err(error) => format!("a stream pipe's text message is no move: {error}"),
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Close(None))) => return None,
                Some(Ok(Message::Close(Some(frame)))) if frame.code == close_code::NORMAL => {
                    return None;
                }
                // The runner's own failure, in its words.
                Some(Ok(Message::Close(Some(frame)))) if frame.code == close_code::ERROR => {
                    frame.reason.to_string()
                }
                Some(Ok(Message::Close(Some(frame)))) => {
                    format!("the runner closed the stream pipe ({}): {}", frame.code, frame.reason)
                }
                Some(Err(error)) => error.to_string(),
                None => "the stream pipe's connection ended without a close frame".to_owned(),
            };
            return Some((Err(failure), None));
        }
    });
    // The sink's device goes to the runner as soon as the pipe knows it, while
    // the bytes flow.
    let tell = async {
        let mut told = None;
        loop {
            let device = sink.borrow_and_update().clone();
            if let Some(device) = device
                && told.as_ref() != Some(&device)
            {
                let text = serde_json::to_string(&PipeSink { sink: device.clone() })
                    .expect("a pipe's sink encodes");
                if to_runner.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
                told = Some(device);
            }
            if sink.changed().await.is_err() {
                return;
            }
        }
    };
    let pumped = source.pump(body);
    tokio::pin!(pumped);
    let outcome = tokio::select! {
        outcome = &mut pumped => outcome,
        () = tell => pumped.await,
    };
    let frame = match outcome {
        Ok(()) => CloseFrame {
            code: close_code::NORMAL,
            reason: "drained".into(),
        },
        Err(failure) => CloseFrame {
            code: close_code::ERROR,
            reason: close_reason(failure.reason()).into(),
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

/// Takes the upgrade of a call's standard output: the runner reads it over this
/// connection, and over the next ones if this one is lost.
pub fn accept_call_sink(upgrade: WebSocketUpgrade, sink: Arc<SinkSession>) -> Response {
    upgrade
        .max_message_size(PIPE_SESSION_MESSAGE_BYTES)
        .max_frame_size(PIPE_SESSION_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve_call_sink(socket, sink))
}

/// Serves one connection of the call's standard output until the pipe's end is
/// confirmed, the connection ends or another connection takes over.
async fn serve_call_sink(socket: WebSocket, sink: Arc<SinkSession>) {
    let (mut to_runner, mut from_runner) = socket.split();
    // The connection that served until now ends first, and releases the
    // session's state.
    let taken = sink.attach();
    let mut state = tokio::select! {
        () = taken.cancelled() => return,
        state = sink.state.lock() => state,
    };
    // The runner opens with how many messages it delivered.
    let opening = tokio::time::timeout(OPENING, async {
        loop {
            match from_runner.next().await? {
                Ok(Message::Binary(frame)) => return Some(frame),
                Ok(Message::Ping(_) | Message::Pong(_)) => continue,
                _ => return None,
            }
        }
    })
    .await;
    let Ok(Some(opening)) = opening else {
        return;
    };
    let received = match state.numbers.read(&opening) {
        Ok(Read::Own(bytes)) => bytes
            .try_into()
            .map(u64::from_be_bytes)
            .ok()
            .and_then(|received| state.numbers.resume(received).ok()),
        _ => None,
    };
    if received.is_none() {
        let frame = CloseFrame {
            code: close_code::ERROR,
            reason: "the opening of a pipe's connection names no count to resume from".into(),
        };
        let _sent = to_runner.send(Message::Close(Some(frame))).await;
        return;
    }
    let opened = wire::own_frame(state.numbers.received().to_be_bytes().to_vec());
    if to_runner.send(Message::Binary(opened.into())).await.is_err() {
        return;
    }
    // What the runner did not receive goes again, before anything new.
    while let Some(frame) = state.numbers.next_resend() {
        if to_runner.send(Message::Binary(frame.into())).await.is_err() {
            return;
        }
    }
    loop {
        if state.ended && state.numbers.unconfirmed() == 0 {
            sink.finish();
            let frame = CloseFrame {
                code: close_code::NORMAL,
                reason: "drained".into(),
            };
            let _sent = to_runner.send(Message::Close(Some(frame))).await;
            let _closed = to_runner.close().await;
            return;
        }
        let state = &mut *state;
        tokio::select! {
            biased;
            () = taken.cancelled() => return,
            message = from_runner.next() => match message {
                Some(Ok(Message::Binary(frame))) => {
                    if state.numbers.read(&frame).is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                // The runner closes the pipe itself once it reads no more: at
                // its end, or when whoever read it let go.
                Some(Ok(Message::Close(_))) => {
                    sink.finish();
                    return;
                }
                // A runner that failed reads the pipe over its next
                // connection, if it still wants it.
                _ => return,
            },
            frames = state.next_frames() => {
                // All of them are numbered and kept before the first is sent:
                // the bytes left the pipe, so a send that fails cuts none of
                // them off from the next connection.
                let messages: Vec<_> = frames
                    .into_iter()
                    .map(|frame| state.numbers.stamp(frame.to_vec(), true))
                    .collect();
                for message in messages {
                    if to_runner.send(Message::Binary(message.into())).await.is_err() {
                        return;
                    }
                }
            }
            // A pipe quiet for long keeps its connection through an edge that
            // ends a silent one.
            () = tokio::time::sleep(PIPE_IDLE) => {
                if to_runner.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
        }
    }
}
