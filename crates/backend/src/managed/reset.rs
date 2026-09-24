//! A Cloud reset (`managed-hosts.md` § System reset): a clean system on the
//! base the reset selected, with the home kept. Its intent is durable: the
//! operation id the page chose, the base, the phase it reached and its
//! failure are written before each step, so a retry of the same id resumes
//! the same operation and a backend that stopped in the middle of one
//! finishes its disk step when it starts again.
//!
//! The reset holds the conversations that cannot work without the Cloud
//! (`sessions-and-targets.md` § How a conversation uses a device): their
//! turns stop, what their users send waits, and the transfers and streams
//! of those whose files are on the Cloud end. It then flushes the Cloud and
//! lets its runner go, which ends every command still running on it, an
//! attached conversation's among them, waits for the operations that held
//! the Cloud to let go, and saves, rebuilds and boots it.

use std::rc::Rc;

use demi_machines_protocol::{CurrentBaseVersionParams, ReconcileParams, ResetParams};
use demi_web_api::cloud::ResetPhase;
use demi_web_api::ids::OperationId;

use super::machine::{CloudError, Machine, Phase};
use crate::backend::Services;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::managed::ManagedOperation;

/// What a backend that started records of a reset its disks were recovered
/// for: the operation failed, and a retry boots the Cloud.
const RECOVERED: &str = "Reset disks recovered; retry to start Cloud";

impl Shard {
    /// Admits the reset `id` of the user's Cloud, or answers the one of that
    /// id: the same operation while it runs or once it is ready, and a
    /// failed one resumed on the base it selected. A new operation selects
    /// the base the manager has now. Another reset while one runs is
    /// refused, and a stopped Cloud that finds no capacity permit is too.
    pub(crate) async fn reset_cloud(&self, id: OperationId) -> Result<ManagedOperation, CloudError> {
        if self.cloud().is_closed() {
            return Err(CloudError::Closed);
        }
        let services = self.services();
        let device = services.control.managed_device_or_create(self.user().clone()).await?;
        let machine = self.cloud_machine(&device).await?;
        if let Some(running) = self.running_reset(&machine, &id)? {
            return Ok(running);
        }
        let stored = services
            .control
            .managed_operation(device.id.clone(), id.clone())
            .await?;
        if let Some(ready) = stored.as_ref().filter(|stored| stored.phase == ResetPhase::Ready) {
            return Ok(ready.clone());
        }
        let base_version = match stored {
            Some(stored) => stored.base_version,
            None => services.cloud.machines.call(CurrentBaseVersionParams {}).await?,
        };
        let operation = ManagedOperation {
            id: id.clone(),
            base_version,
            phase: ResetPhase::Stopping,
            error: None,
        };
        // Checked again after the reads: from here to the phase change
        // nothing awaits, so two resets never both start and none starts
        // beside the shard's close.
        if self.cloud().is_closed() {
            return Err(CloudError::Closed);
        }
        if let Some(running) = self.running_reset(&machine, &id)? {
            return Ok(running);
        }
        let mut phase = machine.phase();
        let permit = match std::mem::replace(&mut *phase, Phase::Off) {
            Phase::Off => match services.cloud.capacity.try_take() {
                Some(permit) => permit,
                None => return Err(CloudError::Capacity),
            },
            // A Cloud that is not stopped keeps the permit it holds; the
            // reset joins its boot or save first.
            Phase::Booting(permit) | Phase::Saving(permit) => permit,
            Phase::Running(running) => running.permit,
            Phase::Resetting { .. } => unreachable!("a running reset was answered above"),
        };
        machine.operation.replace(Some(operation.clone()));
        let task = {
            let shard = self.this();
            let machine = machine.clone();
            let operation = operation.clone();
            self.spawn_joinable(async move { shard.run_reset(&machine, operation).await })
        };
        *phase = Phase::Resetting {
            permit,
            operation: id,
            task,
        };
        Ok(operation)
    }

    /// The operation of the reset that holds the machine, if it is `id`;
    /// another one refuses.
    fn running_reset(&self, machine: &Machine, id: &OperationId) -> Result<Option<ManagedOperation>, CloudError> {
        match machine.resetting() {
            Some(running) if running == *id => Ok(machine.operation.borrow().clone()),
            Some(_) => Err(CloudError::Resetting),
            None => Ok(None),
        }
    }

    /// Runs the reset to its end, writing each phase before its step, and
    /// ends it running or off, the failure kept with the operation.
    async fn run_reset(&self, machine: &Rc<Machine>, operation: ManagedOperation) -> Result<(), CloudError> {
        let reset = self.reset_steps(machine, operation.clone()).await;
        let (next, phase) = match &reset {
            Ok(()) => (ResetPhase::Ready, None),
            Err(error) => {
                // The failure is the one reported; a save that fails too is
                // logged beside it.
                if let Err(saved) = self.save(machine).await {
                    tracing::warn!(device = %machine.device.id, "a Cloud whose reset failed was not saved: {saved}");
                }
                machine.error.replace(Some(error.to_string()));
                (ResetPhase::Failed, Some(error.to_string()))
            }
        };
        let recorded = self.record_phase(machine, &operation, next, phase).await;
        {
            let mut phase = machine.phase();
            // Only the reset ends the resetting phase.
            match std::mem::replace(&mut *phase, Phase::Off) {
                Phase::Resetting { permit, .. } if reset.is_ok() => {
                    machine.error.replace(None);
                    *phase = Phase::Running(self.running(machine, permit));
                }
                Phase::Resetting { .. } => {}
                other => *phase = other,
            }
        }
        reset.and(recorded)
    }

    async fn reset_steps(&self, machine: &Rc<Machine>, operation: ManagedOperation) -> Result<(), CloudError> {
        let services = self.services();
        let device = &machine.device.id;
        self.record_phase(machine, &operation, ResetPhase::Stopping, None).await?;
        if let Some(transition) = machine.transition() {
            // A boot or a save that failed still finishes before the reset
            // takes the disks over.
            let _ = transition.await;
        }
        let hold = services.cloud.tuning.reset_hold;
        let uses = self.cloud_uses().await?;
        let _held = self.hold_uses_for_reset(&uses, hold).await?;
        // The runner flushes, best effort, and goes: every command still
        // running on the Cloud ends with it.
        self.flush(device).await;
        self.devices().disconnect(device, "Cloud is resetting");
        let _reserved = tokio::time::timeout(hold, machine.gate.reserve())
            .await
            .map_err(|_| CloudError::Failed("The Cloud's operations did not end for the reset".into()))?;

        self.record_phase(machine, &operation, ResetPhase::Saving, None).await?;
        self.save(machine).await?;
        self.record_phase(machine, &operation, ResetPhase::Rebuilding, None).await?;
        services.cloud.machines.call(reset_params(machine, &operation)).await?;
        services
            .control
            .announce_cloud_reset(self.user().clone(), operation.id.clone())
            .await?;
        machine.clear_deaths();
        self.record_phase(machine, &operation, ResetPhase::Booting, None).await?;
        self.boot(machine).await
    }

    /// Writes the operation's phase, and shows it.
    async fn record_phase(
        &self,
        machine: &Machine,
        operation: &ManagedOperation,
        phase: ResetPhase,
        error: Option<String>,
    ) -> Result<(), CloudError> {
        let operation = ManagedOperation {
            phase,
            error,
            ..operation.clone()
        };
        machine.operation.replace(Some(operation.clone()));
        self.services()
            .control
            .put_managed_operation(machine.device.id.clone(), operation)
            .await?;
        Ok(())
    }
}

fn reset_params(machine: &Machine, operation: &ManagedOperation) -> ResetParams {
    ResetParams {
        device_id: machine.device.id.to_string(),
        operation_id: operation.id.to_string(),
        base_version: operation.base_version.to_string(),
    }
}

/// Recovers before the backend serves (`backend.md` § Startup and shutdown):
/// the manager stops and saves every machine and recovers its incomplete
/// operations, and each reset this backend left unfinished finishes its
/// disk step, which is idempotent by its id, is announced, and is recorded
/// as failed so that a retry boots the Cloud. Nothing boots here.
pub(crate) async fn recover_resets(services: &Services) -> Result<(), RecoveryError> {
    services.cloud.machines.call(ReconcileParams {}).await?;
    for (device, operation) in services.control.unfinished_managed_operations().await? {
        let record = services
            .control
            .device(device.clone())
            .await?
            .ok_or_else(|| RecoveryError::MissingDevice(device.to_string()))?;
        let params = ResetParams {
            device_id: device.to_string(),
            operation_id: operation.id.to_string(),
            base_version: operation.base_version.to_string(),
        };
        services.cloud.machines.call(params).await?;
        services
            .control
            .announce_cloud_reset(record.user, operation.id.clone())
            .await?;
        let failed = ManagedOperation {
            phase: ResetPhase::Failed,
            error: Some(RECOVERED.to_owned()),
            ..operation
        };
        services.control.put_managed_operation(device, failed).await?;
    }
    Ok(())
}

/// Why the backend could not recover its Clouds before serving.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RecoveryError {
    #[error(transparent)]
    Machines(#[from] super::client::MachinesError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("a reset names the device {0}, which no longer exists")]
    MissingDevice(String),
}
