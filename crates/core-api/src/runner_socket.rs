//! A runner's socket as every way in to a core takes it
//! (`crates/runner-protocol/src/wire/mod.rs`): its first message is its
//! hello, which must come within the hello deadline, and a socket whose
//! device is known moves into the core that holds it. The server knows the
//! device by the token in the hello, or by the code its user claims; a
//! program's process by the channel the runner came on.

use std::rc::Rc;

use axum::extract::ws::{Message, WebSocket};
use demi_core::{Core, KeptJobs};
use demi_core::pool::Shards;
use demi_core_hosts::devices::{RunnerSocket, send};
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::{self, HelloErrorCode, Inbound, Outbound, RunnerInfo};
use demi_web_api_protocol::ids::{DeviceId, UserId};

/// What a runner's hello says.
pub struct Hello {
    pub protocol: u32,
    pub token: Option<DeviceToken>,
    pub runner: RunnerInfo,
    pub kept: KeptJobs,
}

/// The first message, which must be a hello; none when the socket closed or
/// sent something else, which breaks the protocol.
pub async fn hello(socket: &mut WebSocket) -> Option<Hello> {
    loop {
        let frame = match socket.recv().await? {
            Ok(Message::Binary(frame)) => frame,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Text(_)) => {
                tracing::info!(event = "runner_connection.closed", reason = "text_frame", "closing a runner connection that sent a text frame");
                return None;
            }
            Ok(Message::Close(_)) | Err(_) => return None,
        };
        // The hello belongs to the connection: no number covers it.
        let own = match wire::Session::new().read(&frame) {
            Ok(wire::Read::Own(own)) => own,
            _ => {
                tracing::info!(event = "runner_connection.closed", reason = "first_not_hello", "closing a runner connection whose first message is not a hello");
                return None;
            }
        };
        return match wire::decode::<Outbound>(own) {
            Ok(Outbound::Hello {
                protocol,
                device_token,
                runner,
                instance,
                jobs,
                resume,
            }) => Some(Hello {
                protocol,
                token: device_token,
                runner,
                kept: KeptJobs {
                    instance,
                    jobs,
                    resume,
                },
            }),
            Ok(_) => {
                tracing::info!(event = "runner_connection.closed", reason = "first_not_hello", "closing a runner connection whose first message is not a hello");
                None
            }
            Err(error) => {
                tracing::info!(event = "runner_connection.closed", reason = "malformed_frame", %error, "closing a runner connection over a malformed frame");
                None
            }
        };
    }
}

/// Resolves once the socket closed; the messages before that are dropped,
/// since a runner sends nothing but its hello before it is answered.
pub async fn closed(socket: &mut WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        if matches!(message, Message::Close(_)) {
            return;
        }
    }
}

/// Answers the hello with a refusal and closes the socket.
pub async fn refuse(mut socket: RunnerSocket, runner: &RunnerInfo, code: HelloErrorCode, reason: &str) {
    tracing::warn!(
        event = "runner_hello.refused",
        code = %code,
        reason,
        runner_name = %runner.name,
        platform = %runner.platform,
        "a runner's hello was refused"
    );
    let refusal = Inbound::HelloError {
        code,
        reason: reason.into(),
    };
    // A runner that went away hears nothing, and there is nothing to close.
    if send(&mut socket, &refusal).await.is_ok() {
        let _ = socket.send(Message::Close(None)).await;
    }
}


/// Serves the socket of the runner a channel carries for the device `device`
/// of `owner`'s core until the core takes it: the runner's hello must come
/// within `deadline`, and presents no token, since the channel names the
/// device.
pub async fn accept_device(
    deadline: std::time::Duration,
    shards: Shards,
    owner: UserId,
    device: DeviceId,
    mut socket: RunnerSocket,
) {
    let hello = match tokio::time::timeout(deadline, hello(&mut socket)).await {
        Ok(Some(hello)) => hello,
        Ok(None) => return,
        Err(_) => {
            tracing::info!(event = "runner_connection.closed", reason = "no_hello", "closing a runner connection that sent no hello");
            return;
        }
    };
    let Hello {
        protocol,
        token: _,
        runner,
        kept,
    } = hello;
    if protocol != wire::VERSION {
        let reason = format!("unsupported protocol {protocol}; this core speaks {}", wire::VERSION);
        refuse(socket, &runner, HelloErrorCode::UnsupportedProtocol, &reason).await;
        return;
    }
    let adopted = shards
        .of(&owner)
        .adopt(move |core: Rc<Core>| async move {
            match core.store().device(device).await {
                Ok(Some(record)) => core.adopt_runner(record, runner, kept, socket).await,
                Ok(None) => refuse(socket, &runner, HelloErrorCode::UnknownDevice, "unknown device").await,
                Err(error) => {
                    tracing::error!(
                        event = "runner_device.read_failed",
                        error = &error as &dyn std::error::Error,
                        "a runner's device could not be read"
                    );
                    refuse(socket, &runner, HelloErrorCode::Internal, "the device could not be read").await;
                }
            }
        })
        .await;
    // A core that is closing takes no runner: dropping the socket closes it
    // without a word.
    if adopted.is_err() {
        tracing::info!(event = "socket.refused", kind = "runner", reason = "shutting_down", "a runner connected while its core closes");
    }
}
