//! A runner's socket at the edge (`runner.md` § Connection and identity).
//! The runner's first message is its hello, which must come within the
//! hello deadline. A known token moves the socket into the shard of the
//! device's owner; a runner without one waits here for its user to claim the
//! code it printed, the code changing every claim lifetime while it waits.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::{self, HelloErrorCode, Inbound, Outbound, RunnerInfo};

use super::claims::ClaimGrant;
use super::codes::ClaimCode;
use crate::auth::sessions::TokenHash;
use crate::backend::Services;
use crate::shard::Shards;

/// Serves a runner's socket until it is handed to a shard or closes.
pub(crate) async fn accept(services: Arc<Services>, shards: Shards, mut socket: WebSocket) {
    let deadline = services.runners.hello_deadline;
    let hello = match tokio::time::timeout(deadline, hello(&mut socket)).await {
        Ok(Some(hello)) => hello,
        Ok(None) => return,
        Err(_) => {
            tracing::info!("closing a runner connection that sent no hello within {deadline:?}");
            return;
        }
    };
    let Hello {
        protocol,
        token,
        runner,
    } = hello;
    if protocol != wire::VERSION {
        let reason = format!("unsupported protocol {protocol}; this backend speaks {}", wire::VERSION);
        refuse(socket, &runner, HelloErrorCode::UnsupportedProtocol, &reason).await;
        return;
    }
    let Some(token) = token else {
        // A managed host is born with its token; one without it is
        // misbooted, never a device waiting to be paired.
        if runner.managed == Some(true) {
            let reason = "a managed host presents its device token; it is never paired";
            refuse(socket, &runner, HelloErrorCode::UnknownDevice, reason).await;
        } else {
            await_claim(&services, &shards, socket, runner).await;
        }
        return;
    };
    let lookup = services.control.device_by_token(TokenHash::of(token.expose()));
    let device = tokio::select! {
        device = lookup => device,
        // The runner went away during the lookup: its socket never becomes
        // online. A repeated hello says nothing new.
        () = closed(&mut socket) => return,
    };
    let device = match device {
        Ok(Some(device)) => device,
        Ok(None) => {
            refuse(socket, &runner, HelloErrorCode::UnknownDevice, "unknown device").await;
            return;
        }
        Err(error) => {
            tracing::error!(error = &error as &dyn std::error::Error, "a runner's device could not be read");
            refuse(socket, &runner, HelloErrorCode::Internal, "the device could not be read").await;
            return;
        }
    };
    let owner = device.user.clone();
    let adopted = shards
        .of(&owner)
        .adopt(move |shard| async move { shard.adopt_runner(device, runner, socket).await })
        .await;
    // A shard that is closing takes no runner: dropping the socket closes it
    // without a word, as shutdown does to every runner.
    if adopted.is_err() {
        tracing::info!("a runner connected while the backend shuts down");
    }
}

/// What a runner's hello says.
struct Hello {
    protocol: u32,
    token: Option<DeviceToken>,
    runner: RunnerInfo,
}

/// The first message, which must be a hello; none when the socket closed or
/// sent something else, which breaks the protocol.
async fn hello(socket: &mut WebSocket) -> Option<Hello> {
    loop {
        let frame = match socket.recv().await? {
            Ok(Message::Binary(frame)) => frame,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Text(_)) => {
                tracing::info!("closing a runner connection that sent a text frame");
                return None;
            }
            Ok(Message::Close(_)) | Err(_) => return None,
        };
        return match wire::decode::<Outbound>(&frame) {
            Ok(Outbound::Hello {
                protocol,
                device_token,
                runner,
            }) => Some(Hello {
                protocol,
                token: device_token,
                runner,
            }),
            Ok(_) => {
                tracing::info!("closing a runner connection whose first message is not a hello");
                None
            }
            Err(error) => {
                tracing::info!("closing a runner connection over a malformed frame: {error}");
                None
            }
        };
    }
}

/// Resolves once the socket closed; the messages before that are dropped,
/// since a runner sends nothing but its hello before it is answered.
async fn closed(socket: &mut WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        if matches!(message, Message::Close(_)) {
            return;
        }
    }
}

/// Answers the hello with a refusal and closes the socket.
async fn refuse(mut socket: WebSocket, runner: &RunnerInfo, code: HelloErrorCode, reason: &str) {
    tracing::warn!(
        "runner hello refused ({code}): {reason} [{}, {}]",
        runner.name,
        runner.platform
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

/// Sends one message on a socket nothing else writes to yet.
pub(super) async fn send(socket: &mut WebSocket, message: &Inbound) -> Result<(), axum::Error> {
    let frame = wire::encode(message)
        .expect("the backend's own runner messages encode")
        .into_bytes();
    socket.send(Message::Binary(frame.into())).await
}

/// Waits for the runner's user to claim it, printing a new code every claim
/// lifetime, and hands the socket to the user's shard once claimed. Frames
/// the runner sends meanwhile are ignored; unclaimed runners are not pinged.
async fn await_claim(services: &Services, shards: &Shards, mut socket: WebSocket, runner: RunnerInfo) {
    loop {
        let code = ClaimCode::generate();
        let Some(granted) = services.claims.wait(code, runner.clone()) else {
            return;
        };
        let pending = Inbound::ClaimPending {
            claim_token: code.printed(),
        };
        if send(&mut socket, &pending).await.is_err() {
            services.claims.withdraw(&code);
            return;
        }
        let expired = tokio::time::sleep(services.runners.claim_lifetime);
        tokio::pin!(expired, granted);
        let grant = loop {
            tokio::select! {
                () = &mut expired => break None,
                grant = &mut granted => match grant {
                    Ok(grant) => break Some(grant),
                    // The backend is shutting down and lets its waiting
                    // runners go.
                    Err(_) => return,
                },
                message = socket.recv() => match message {
                    Some(Ok(Message::Close(_)) | Err(_)) | None => {
                        services.claims.withdraw(&code);
                        return;
                    }
                    Some(Ok(_)) => {}
                },
            }
        };
        match grant {
            // The code expired: it is dead, and a new one goes out.
            None => services.claims.withdraw(&code),
            Some(grant) => {
                hand_over(shards, socket, runner, grant).await;
                return;
            }
        }
    }
}

/// Gives the claimed runner its token and moves its socket into the shard
/// of the user who claimed it. A runner lost on the way leaves the grant
/// unanswered, and the claim then deletes the device it made.
async fn hand_over(shards: &Shards, mut socket: WebSocket, runner: RunnerInfo, grant: ClaimGrant) {
    let ClaimGrant { device, token, bound } = grant;
    let claimed = Inbound::Claimed { device_token: token };
    if send(&mut socket, &claimed).await.is_err() {
        return;
    }
    let owner = device.user.clone();
    let adopted = shards
        .of(&owner)
        .adopt(move |shard| async move { shard.adopt_claimed(device, runner, socket, bound).await })
        .await;
    if adopted.is_err() {
        tracing::info!("a claimed runner arrived while the backend shuts down");
    }
}
