//! A device's worker (`crates/machine-manager-protocol/src/lib.rs`): it owns
//! the device's runtime (`crate::runtime`) and runs the device's operations
//! one at a time, in arrival order.
//!
//! The runtime's exit is one more event between operations, never during one,
//! and the worker waits only on its current runtime, so the late exit of an
//! earlier boot cannot stop a newer one. After the current boot exits the
//! runtime stops what remains and saves the device's storage, and the worker
//! sends the death event, even when stopping or saving fails.

use demi_machine_manager_protocol::{BaseVersion, SandboxMachine};
use tokio::sync::{mpsc, oneshot};

use super::errors::OpError;
use crate::{
    network::slots::Slot,
    runtime::{DeviceOp, DeviceRuntime, Reports},
    sandbox::files::SandboxRecord,
};

/// What a device's worker is asked to do.
pub enum DeviceCommand {
    Run {
        op: DeviceOp,
        reply: oneshot::Sender<Result<(), OpError>>,
    },
    /// The running device's slot and machine; none when it is stopped.
    Running {
        reply: oneshot::Sender<Option<(Slot, SandboxMachine)>>,
    },
    /// The manager leaves its sandboxes to the next one
    /// (`crates/machine-manager/src/handover.rs`): the worker hands its
    /// running runtime's boot, programs' base and machine over, releases
    /// nothing of it, and serves no more.
    Leave {
        reply: oneshot::Sender<Option<(SandboxRecord, BaseVersion, SandboxMachine)>>,
    },
}

pub struct DeviceWorker {
    runtime: DeviceRuntime,
    reports: Reports,
}

impl DeviceWorker {
    pub fn new(runtime: DeviceRuntime, reports: Reports) -> Self {
        Self { runtime, reports }
    }

    /// Serves the device until its manager drops the queue, or leaves it.
    pub async fn run(mut self, mut inbox: mpsc::Receiver<DeviceCommand>) {
        loop {
            // The runtime's exit is not queued behind requests: stopping a
            // runtime waits for its waiter, which must never wait for room in
            // this bounded queue.
            let command = tokio::select! {
                command = inbox.recv() => command,
                outcome = self.runtime.exited() => {
                    self.runtime.recover(outcome).await;
                    self.reports.change().await;
                    self.reports.death().await;
                    continue;
                }
            };
            let Some(command) = command else {
                break;
            };
            match command {
                DeviceCommand::Run { op, reply } => {
                    let outcome = self.runtime.operate(op).await;
                    self.reports.change().await;
                    // The requester may have gone; the operation completed anyway.
                    let _ = reply.send(outcome);
                }
                DeviceCommand::Running { reply } => {
                    // As above: no one may be waiting any more.
                    let _ = reply.send(self.runtime.running());
                }
                DeviceCommand::Leave { reply } => {
                    // As above: no one may be waiting any more.
                    let _ = reply.send(self.runtime.leave());
                    break;
                }
            }
        }
    }
}
