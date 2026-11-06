//! A runner's socket at the edge (`crates/runner-protocol/src/wire/mod.rs`).
//!
//! The runner's first message is its hello, which must come within the hello
//! deadline.
//!
//! - A known token moves the socket into the core of the device's owner.
//! - A runner without one waits here for its user to claim the code it printed,
//!   the code changing every claim lifetime while it waits.

use std::sync::Arc;

use axum::extract::ws::Message;
use demi_core_api::runner_socket::{Hello, closed, hello, refuse};
use demi_backend_database::accounts::TokenHash;
use demi_core_store::StorageError;
use demi_runner_protocol::wire::{self, HelloErrorCode, Inbound, RunnerInfo};

use demi_backend_runners::claims::ClaimGrant;
use demi_backend_runners::codes::ClaimCode;
use demi_core_hosts::devices::{RunnerSocket, send};

use demi_backend_user_shard::services::Services;
use demi_core::pool::Shards;

/// Serves a runner's socket until it is handed to a core or closes.
pub(crate) async fn accept(services: Arc<Services>, shards: Shards, mut socket: RunnerSocket) {
    let deadline = services.runners.hello_deadline;
    let hello = match tokio::time::timeout(deadline, hello(&mut socket)).await {
        Ok(Some(hello)) => hello,
        Ok(None) => return,
        Err(_) => {
            tracing::info!(event = "runner_connection.closed", reason = "no_hello", deadline_ms = u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX), "closing a runner connection that sent no hello");
            return;
        }
    };
    let Hello {
        protocol,
        token,
        runner,
        kept,
    } = hello;
    if protocol != wire::VERSION {
        let reason = format!(
            "unsupported protocol {protocol}; this backend speaks {}",
            wire::VERSION
        );
        refuse(
            socket,
            &runner,
            HelloErrorCode::UnsupportedProtocol,
            &reason,
        )
        .await;
        return;
    }
    let Some(token) = token else {
        // A managed host is born with its token; one without it is
        // misbooted, never a device waiting to be paired.
        if runner.managed == Some(true) {
            let reason = "a managed host presents its device token; it is never paired";
            refuse(socket, &runner, HelloErrorCode::UnknownDevice, reason).await;
        } else {
            await_claim(&services, &shards, socket, runner, kept.instance).await;
        }
        return;
    };
    let lookup = async {
        let Some((id, owner)) = services.control.device_by_token(TokenHash::of(token.expose())).await? else {
            return Ok(None);
        };
        let device = services.stores.of(&owner).await?.device(id).await?;
        Ok::<_, StorageError>(device.map(|device| (owner, device)))
    };
    let device = tokio::select! {
        device = lookup => device,
        // The runner went away during the lookup: its socket never becomes
        // online. A repeated hello says nothing new.
        () = closed(&mut socket) => return,
    };
    let device = match device {
        Ok(Some(found)) => found,
        Ok(None) => {
            refuse(
                socket,
                &runner,
                HelloErrorCode::UnknownDevice,
                "unknown device",
            )
            .await;
            return;
        }
        Err(error) => {
            tracing::error!(
                event = "runner_device.read_failed",
                error = &error as &dyn std::error::Error,
                "a runner's device could not be read"
            );
            refuse(
                socket,
                &runner,
                HelloErrorCode::Internal,
                "the device could not be read",
            )
            .await;
            return;
        }
    };
    let (owner, device) = device;
    // The devices of a disabled account cannot reach the backend: its
    // runners are turned away without a word and connect again, so enabling
    // the account brings them back.
    match services.control.account(owner.clone()).await {
        Ok(Some(account)) if !account.user.disabled => {}
        Ok(_) => {
            tracing::info!(event = "runner_connection.closed", host_id = %device.id, reason = "account_disabled", "closing a runner connection of a disabled account");
            return;
        }
        Err(error) => {
            tracing::error!(
                event = "runner_owner.read_failed",
                host_id = %device.id,
                error = &error as &dyn std::error::Error,
                "a runner's owner could not be read"
            );
            refuse(socket, &runner, HelloErrorCode::Internal, "the device could not be read").await;
            return;
        }
    }
    let adopted = shards
        .of(&owner)
        .adopt(move |core| async move { core.adopt_runner(device, runner, kept, socket).await })
        .await;
    // A core that is closing takes no runner: dropping the socket closes it
    // without a word, as shutdown does to every runner.
    if adopted.is_err() {
        tracing::info!(event = "socket.refused", kind = "runner", reason = "shutting_down", "a runner connected while the backend shuts down");
    }
}

/// Waits for the runner's user to claim it, printing a new code every claim
/// lifetime, and hands the socket to the user's core once claimed. Frames
/// the runner sends meanwhile are ignored; unclaimed runners are not pinged.
async fn await_claim(
    services: &Services,
    shards: &Shards,
    mut socket: RunnerSocket,
    runner: RunnerInfo,
    instance: u64,
) {
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
                hand_over(shards, socket, runner, instance, grant).await;
                return;
            }
        }
    }
}

/// Moves the claimed runner's socket into the core of the user who claimed it,
/// which gives the runner its token. A runner lost on the way leaves the grant
/// unanswered, and the claim then deletes the device it made.
async fn hand_over(
    shards: &Shards,
    socket: RunnerSocket,
    runner: RunnerInfo,
    instance: u64,
    grant: ClaimGrant,
) {
    let ClaimGrant {
        owner,
        device,
        token,
        bound,
    } = grant;
    let adopted = shards
        .of(&owner)
        .adopt(move |core| async move {
            core.adopt_claimed(device, runner, instance, token, socket, bound).await
        })
        .await;
    if adopted.is_err() {
        tracing::info!(event = "socket.refused", kind = "runner_claimed", reason = "shutting_down", "a claimed runner arrived while the backend shuts down");
    }
}
