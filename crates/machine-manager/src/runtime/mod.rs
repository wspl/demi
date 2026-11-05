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
//!     +-- Simple: a directory tree under proot
//! ```
//!
//! A device runs on the runtime its storage is on: a gVisor generation or
//! working pair, or a Simple Isolation tree. One that has neither yet starts
//! on the isolation the master chose. Once stopped, a device whose storage
//! is on the other runtime moves to the chosen one (`moves`).

pub mod gvisor;
pub mod moves;
pub mod simple;

use std::io;

use std::rc::Rc;

use demi_machine_manager_protocol::{BaseVersion, Capacity, DeviceId, Isolation, LeftSandbox, LeftSimple, SandboxMachine};
use demi_runner_protocol::boot::ManagedBoot;
use tokio::sync::mpsc;

use crate::{
    manager::{Core, OpError},
    network::slots::Slot,
    storage::store::Retention,
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
    Simple(simple::Device),
}

/// A running device as the manager sees it.
pub struct Running {
    pub machine: SandboxMachine,
    /// The gVisor sandbox's network slot; a Simple Isolation Cloud, on the
    /// host's network, has none.
    pub slot: Option<Slot>,
}

/// What a manager that leaves its devices to the next one records of a
/// running one.
pub enum Left {
    Gvisor(LeftSandbox),
    Simple(LeftSimple),
}

impl DeviceRuntime {
    /// A stopped `device` on the runtime of `isolation`, whose new storage
    /// is made from `base`.
    pub fn on(isolation: Isolation, device: DeviceId, core: Rc<Core>, base: BaseVersion, reports: Reports) -> Self {
        match isolation {
            Isolation::Gvisor => Self::Gvisor(gvisor::Device::new(device, core, base, reports)),
            Isolation::Simple => Self::Simple(simple::Device::new(device, core, base)),
        }
    }

    pub fn isolation(&self) -> Isolation {
        match self {
            Self::Gvisor(_) => Isolation::Gvisor,
            Self::Simple(_) => Isolation::Simple,
        }
    }

    /// Runs `op`. A runtime that an operation could not keep running, such
    /// as a checkpoint whose thaw failed, reports the death itself.
    pub async fn operate(&mut self, op: DeviceOp) -> Result<(), OpError> {
        match self {
            Self::Gvisor(device) => device.operate(op).await,
            Self::Simple(device) => device.operate(op).await,
        }
    }

    /// Waits for the device's runtime to end; while none runs, forever.
    pub async fn exited(&mut self) -> io::Result<()> {
        match self {
            Self::Gvisor(device) => device.exited().await.map_err(io::Error::other),
            Self::Simple(device) => device.exited().await,
        }
    }

    /// Stops what remains of a runtime that ended unasked and makes the
    /// device's storage durable.
    pub async fn recover(&mut self, outcome: io::Result<()>) {
        match self {
            Self::Gvisor(device) => device.recover(outcome).await,
            Self::Simple(device) => device.recover(outcome).await,
        }
    }

    /// The running device; none when it is stopped.
    pub fn running(&self) -> Option<Running> {
        match self {
            Self::Gvisor(device) => device.running().map(|(slot, machine)| Running {
                machine,
                slot: Some(slot),
            }),
            Self::Simple(device) => device.running().map(|machine| Running { machine, slot: None }),
        }
    }

    /// What the next manager takes over of the running device; none when it
    /// is stopped. Nothing of it is released.
    pub fn leave(self) -> Option<Left> {
        match self {
            Self::Gvisor(device) => device.leave().map(Left::Gvisor),
            Self::Simple(device) => device.leave().map(Left::Simple),
        }
    }
}
