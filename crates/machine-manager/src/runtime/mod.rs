//! A device's runtime: what runs the device's processes and keeps its
//! storage. The device's worker (`crate::manager`) runs its operations in
//! order and reports what happens; the runtime does each operation.
//!
//! ```text
//! DeviceWorker (queue, order, exit, reports)
//!     | DeviceOp
//!     v
//! DeviceRuntime
//!     +-- Gvisor: a gVisor/systrap sandbox over ext4 images
//! ```

pub mod gvisor;

use demi_machine_manager_protocol::{BaseVersion, Capacity, DeviceId, SandboxMachine};
use demi_runner_protocol::boot::ManagedBoot;
use tokio::sync::mpsc;

use crate::{
    manager::OpError,
    network::slots::Slot,
    sandbox::files::SandboxRecord,
    storage::store::Retention,
    tools::ToolError,
};

/// The operations that change a device.
pub enum DeviceOp {
    Wake {
        boot: ManagedBoot,
        machine: SandboxMachine,
        capacity: Capacity,
    },
    /// Stop and save; also what shutdown does to every device.
    Hibernate { retention: Retention },
    Checkpoint { retention: Retention },
    Reset {
        operation: String,
        base: BaseVersion,
        capacity: Capacity,
        retention: Retention,
    },
}

/// Where a device tells the server what happened to it: that its stored
/// image may have changed, and that its runtime ended unasked.
#[derive(Clone)]
pub struct Reports {
    device: DeviceId,
    deaths: mpsc::Sender<DeviceId>,
    changes: mpsc::Sender<DeviceId>,
}

impl Reports {
    pub fn new(device: DeviceId, deaths: mpsc::Sender<DeviceId>, changes: mpsc::Sender<DeviceId>) -> Self {
        Self { device, deaths, changes }
    }

    pub async fn change(&self) {
        // The server stops listening at shutdown, when no one is left to tell.
        let _ = self.changes.send(self.device.clone()).await;
    }

    pub async fn death(&self) {
        // The server stops listening at shutdown, when no one is left to tell.
        let _ = self.deaths.send(self.device.clone()).await;
    }
}

/// A device on the runtime its storage is on.
pub enum DeviceRuntime {
    Gvisor(gvisor::Device),
}

impl DeviceRuntime {
    /// Runs `op`. A runtime that an operation could not keep running, such
    /// as a checkpoint whose thaw failed, reports the death itself.
    pub async fn operate(&mut self, op: DeviceOp) -> Result<(), OpError> {
        match self {
            Self::Gvisor(device) => device.operate(op).await,
        }
    }

    /// Waits for the device's runtime to end; while none runs, forever.
    pub async fn exited(&mut self) -> Result<(), ToolError> {
        match self {
            Self::Gvisor(device) => device.exited().await,
        }
    }

    /// Stops what remains of a runtime that ended unasked and saves the
    /// device's storage.
    pub async fn recover(&mut self, outcome: Result<(), ToolError>) {
        match self {
            Self::Gvisor(device) => device.recover(outcome).await,
        }
    }

    /// The network slot and the machine of the running device; none when it
    /// is stopped.
    pub fn running(&self) -> Option<(Slot, SandboxMachine)> {
        match self {
            Self::Gvisor(device) => device.running(),
        }
    }

    /// What the next manager takes over of the running device; none when it
    /// is stopped. Nothing of it is released.
    pub fn leave(self) -> Option<(SandboxRecord, BaseVersion, SandboxMachine)> {
        match self {
            Self::Gvisor(device) => device.leave(),
        }
    }
}
