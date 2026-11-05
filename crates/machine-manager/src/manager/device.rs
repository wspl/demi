//! A device's worker (`crates/machine-manager-protocol/src/lib.rs`): it owns
//! the device's runtime (`crate::runtime`) and runs the device's operations
//! one at a time, in arrival order.
//!
//! The runtime's exit is one more event between operations, never during one,
//! and the worker waits only on its current runtime, so the late exit of an
//! earlier boot cannot stop a newer one. After the current boot exits the
//! runtime stops what remains and saves the device's storage, and the worker
//! sends the death event, even when stopping or saving fails.
//!
//! A stopped device whose storage is on another runtime than the isolation
//! the master chose moves to it (`crate::runtime::moves`): when it stops,
//! when the manager asks, and before it wakes, so a wake waits for the
//! move. A move that fails leaves the device where it was, and its wake goes
//! on there; the next wake tries again. A move ends early when the manager
//! stops, and the next manager makes it.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use demi_machine_manager_protocol::{BaseVersion, DeviceId, Isolation, MoveStatus, RuntimeOffer};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{Core, errors::OpError};
use crate::runtime::{DeviceOp, DeviceRuntime, Left, Reports, Running, moves};

/// The isolation the master chose, what this manager can run, and each
/// device's move to the chosen isolation, which its status reports.
pub struct Placement {
    offers: Vec<RuntimeOffer>,
    chosen: Cell<Isolation>,
    moves: RefCell<HashMap<DeviceId, MoveStatus>>,
    /// Cancelled when the manager stops or leaves its devices: the saves it
    /// asks for start no move, and a move under way ends, for the next
    /// manager to make.
    stopping: CancellationToken,
}

impl Placement {
    /// Before the backend chooses, new devices start on gVisor where this
    /// manager runs it, as every Cloud did before it could choose.
    pub fn new(offers: Vec<RuntimeOffer>) -> Self {
        let placement = Self {
            offers,
            chosen: Cell::new(Isolation::Simple),
            moves: RefCell::default(),
            stopping: CancellationToken::new(),
        };
        if placement.runs(Isolation::Gvisor) {
            placement.chosen.set(Isolation::Gvisor);
        }
        placement
    }

    pub fn offers(&self) -> &[RuntimeOffer] {
        &self.offers
    }

    pub fn runs(&self, isolation: Isolation) -> bool {
        self.offers.iter().any(|offer| offer.isolation == isolation && offer.offer.is_ok())
    }

    pub fn chosen(&self) -> Isolation {
        self.chosen.get()
    }

    /// Chooses `isolation`, and forgets each failed move, which was to the
    /// isolation chosen before: the devices it names are to be told.
    pub fn choose(&self, isolation: Isolation) -> Vec<DeviceId> {
        self.chosen.set(isolation);
        let mut moves = self.moves.borrow_mut();
        let failed: Vec<DeviceId> = moves
            .iter()
            .filter(|(_, status)| matches!(status, MoveStatus::Failed { .. }))
            .map(|(device, _)| device.clone())
            .collect();
        for device in &failed {
            moves.remove(device);
        }
        failed
    }

    /// Where a new device's storage goes: the chosen isolation, or the other
    /// where this manager cannot run it.
    pub fn for_new(&self) -> Isolation {
        match (self.chosen(), self.runs(self.chosen())) {
            (chosen, true) => chosen,
            (Isolation::Gvisor, false) => Isolation::Simple,
            (Isolation::Simple, false) => Isolation::Gvisor,
        }
    }

    /// The manager stops or leaves its devices.
    pub fn stop(&self) {
        self.stopping.cancel();
    }

    pub fn move_status(&self, device: &DeviceId) -> Option<MoveStatus> {
        self.moves.borrow().get(device).cloned()
    }

    fn set(&self, device: &DeviceId, status: Option<MoveStatus>) {
        let mut moves = self.moves.borrow_mut();
        match status {
            Some(status) => moves.insert(device.clone(), status),
            None => moves.remove(device),
        };
    }
}

/// What a worker needs to move its device: the runtime it makes on the
/// other side starts from these.
pub struct Mover {
    pub device: DeviceId,
    pub core: Rc<Core>,
    pub base: BaseVersion,
    pub placement: Rc<Placement>,
}

/// What a device's worker is asked to do.
pub enum DeviceCommand {
    Run {
        op: DeviceOp,
        reply: oneshot::Sender<Result<(), OpError>>,
    },
    /// The running device; none when it is stopped.
    Running {
        reply: oneshot::Sender<Option<Running>>,
    },
    /// The manager leaves its sandboxes to the next one
    /// (`crates/machine-manager/src/handover.rs`): the worker hands over
    /// what the next manager takes over of its running runtime, releases
    /// nothing of it, and serves no more.
    Leave {
        reply: oneshot::Sender<Option<Left>>,
    },
    /// Moves the device to the chosen isolation if it is stopped and its
    /// storage is on the other.
    Settle { reply: oneshot::Sender<()> },
}

pub struct DeviceWorker {
    runtime: DeviceRuntime,
    reports: Reports,
    mover: Mover,
}

impl DeviceWorker {
    pub fn new(runtime: DeviceRuntime, reports: Reports, mover: Mover) -> Self {
        Self { runtime, reports, mover }
    }

    /// Moves the stopped device to the chosen isolation, when its storage is
    /// on the other and this manager runs the chosen one, until it is where
    /// the master chose: the master may choose again during a move. A
    /// failure is the device's status, not the caller's.
    async fn settle(&mut self) {
        let placement = self.mover.placement.clone();
        let device = self.mover.device.clone();
        loop {
            let to = placement.chosen();
            if placement.stopping.is_cancelled()
                || self.runtime.isolation() == to
                || !placement.runs(to)
                || self.runtime.running().is_some()
            {
                return;
            }
            placement.set(&device, Some(MoveStatus::Moving));
            self.reports.change().await;
            // What a crash left in a working pair joins the storage that
            // moves; the manager's own save removes no fallback.
            let saved = self
                .runtime
                .operate(DeviceOp::Hibernate {
                    retention: crate::storage::store::Retention::Previous,
                })
                .await;
            let stop = &placement.stopping;
            let moved = match saved {
                Ok(()) => match to {
                    Isolation::Simple => moves::to_simple(&self.mover.core, &device, stop).await,
                    Isolation::Gvisor => moves::to_gvisor(&self.mover.core, &device, stop).await,
                },
                Err(error) => Err(error),
            };
            match moved {
                Ok(()) => {
                    tracing::info!(event = "cloud_move.ended", host_id = %device, to = ?to, "a Cloud moved to the chosen isolation");
                    self.runtime = DeviceRuntime::on(to, device.clone(), self.mover.core.clone(), self.mover.base.clone(), self.reports.clone());
                    placement.set(&device, None);
                    self.reports.change().await;
                }
                Err(_) if stop.is_cancelled() => {
                    tracing::info!(event = "cloud_move.interrupted", host_id = %device, to = ?to, "a Cloud's move ended with the manager; the next manager makes it");
                    placement.set(&device, None);
                    self.reports.change().await;
                    return;
                }
                Err(error) => {
                    let message = crate::server::chain(&error);
                    tracing::error!(event = "cloud_move.failed", host_id = %device, to = ?to, error = %message, "a Cloud could not move to the chosen isolation; it stays where it was");
                    placement.set(&device, Some(MoveStatus::Failed { message: error.to_string() }));
                    self.reports.change().await;
                    return;
                }
            }
        }
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
                    self.settle().await;
                    continue;
                }
            };
            let Some(command) = command else {
                break;
            };
            match command {
                DeviceCommand::Run { op, reply } => {
                    let wake = matches!(op, DeviceOp::Wake { .. });
                    if wake {
                        self.settle().await;
                    }
                    let outcome = self.runtime.operate(op).await;
                    self.reports.change().await;
                    // The requester may have gone; the operation completed anyway.
                    let _ = reply.send(outcome);
                    // A wake that failed leaves the device stopped; the move
                    // it could not make waits for the next one.
                    if !wake {
                        self.settle().await;
                    }
                }
                DeviceCommand::Settle { reply } => {
                    self.settle().await;
                    // As above: no one may be waiting any more.
                    let _ = reply.send(());
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
