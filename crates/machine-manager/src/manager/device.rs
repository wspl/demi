//! A device's worker (`crates/machine-manager-protocol/src/lib.rs`): it owns
//! the device's sandbox and runs the device's operations one at a time, in
//! arrival order.
//!
//! The sandbox's exit is one more event between operations, never during one,
//! and the worker waits only on its current sandbox, so the late exit of an
//! earlier boot cannot stop a newer one. After the current boot exits the
//! worker stops what remains, saves the working pair and sends the death event,
//! even when stopping or saving fails.

use std::{path::Path, rc::Rc};

use demi_machine_manager_protocol::image::PROGRAMS_PATH;
use demi_machine_manager_protocol::{
    BaseVersion, Capacity, DeviceId, MachineImageState, SandboxMachine, SharedFolder, Volume,
};
use demi_runner_protocol::boot::ManagedBoot;
use tokio::sync::{mpsc, oneshot};

use super::{
    Core,
    errors::{OpError, Parts},
};
use crate::{
    blocking,
    network::slots::Slot,
    sandbox::Sandbox,
    server::chain,
    storage::{
        clone::clone_sparse,
        durable::{remove_tree, sync, write_json},
        base, ext4, room, skeleton,
        store::{ImagePair, Retention},
        working::{WorkingPair, new_generation},
    },
    tools::ToolError,
};

/// What a device's worker is asked to do.
pub enum DeviceCommand {
    Run {
        op: DeviceOp,
        reply: oneshot::Sender<Result<(), OpError>>,
    },
    /// The slot of the device's running sandbox; none when it is stopped.
    Running {
        reply: oneshot::Sender<Option<Slot>>,
    },
}

/// The operations that change a device.
pub enum DeviceOp {
    Wake {
        boot: ManagedBoot,
        machine: SandboxMachine,
        capacity: Capacity,
        shared: Option<SharedFolder>,
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
    /// Stop, and remove all the device's storage.
    Delete,
}

pub struct DeviceWorker {
    device: DeviceId,
    core: Rc<Core>,
    /// The base a device's first storage is made from.
    base: BaseVersion,
    working: WorkingPair,
    runtime: Option<Sandbox>,
    /// The shared folder the running sandbox took, which its stop gives
    /// back.
    shared: Option<String>,
    deaths: mpsc::Sender<DeviceId>,
    /// Told this device's id each time an operation ended, since it may
    /// have changed the device's stored image.
    changes: mpsc::Sender<DeviceId>,
}

/// A staging directory in the working directory, whose name starts with a
/// dot so recovery removes it; the operation removes it on every path.
struct Stage {
    path: std::path::PathBuf,
}

impl Stage {
    async fn create(core: &Core, prefix: &str) -> Result<Self, OpError> {
        let path = core
            .config
            .working()
            .join(format!(".{prefix}-{}", uuid::Uuid::new_v4()));
        let created = path.clone();
        blocking::run(move |_| fs_err::create_dir(created)).await?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    async fn remove(self) {
        let path = self.path;
        // A stage that stays behind has not replaced anything, and the next
        // reconciliation removes it; the failure is logged.
        if let Err(error) = blocking::run(move |off| remove_tree(off, &path)).await {
            tracing::warn!(event = "stage.remove_failed", kind = "device", %error, "a stage that stays behind could not be removed");
        }
    }
}

impl DeviceWorker {
    pub fn new(
        device: DeviceId,
        core: Rc<Core>,
        base: BaseVersion,
        deaths: mpsc::Sender<DeviceId>,
        changes: mpsc::Sender<DeviceId>,
    ) -> Self {
        Self {
            working: WorkingPair::new(&core.config.working(), &device),
            device,
            core,
            base,
            runtime: None,
            shared: None,
            deaths,
            changes,
        }
    }

    /// Serves the device until its manager drops the queue.
    pub async fn run(mut self, mut inbox: mpsc::Receiver<DeviceCommand>) {
        loop {
            // The sandbox's exit is not queued behind requests: stopping a
            // sandbox waits for its waiter, which must never wait for room in
            // this bounded queue.
            let command = tokio::select! {
                command = inbox.recv() => command,
                outcome = exited(&mut self.runtime) => {
                    self.runtime_exited(outcome).await;
                    continue;
                }
            };
            let Some(command) = command else {
                break;
            };
            match command {
                DeviceCommand::Run { op, reply } => {
                    let outcome = self.operate(op).await;
                    self.report_change().await;
                    // The requester may have gone; the operation completed anyway.
                    let _ = reply.send(outcome);
                }
                DeviceCommand::Running { reply } => {
                    let slot = self.runtime.as_ref().map(|sandbox| sandbox.slot().clone());
                    // As above: no one may be waiting any more.
                    let _ = reply.send(slot);
                }
            }
        }
    }

    async fn operate(&mut self, op: DeviceOp) -> Result<(), OpError> {
        match op {
            DeviceOp::Wake {
                boot,
                machine,
                capacity,
                shared,
            } => self.wake(&boot, machine, capacity, shared.as_ref()).await,
            DeviceOp::Hibernate { retention } => {
                self.stop().await?;
                self.save(retention).await
            }
            DeviceOp::Checkpoint { retention } => self.checkpoint(retention).await,
            DeviceOp::Reset {
                operation,
                base,
                capacity,
                retention,
            } => self.reset(&operation, &base, capacity, retention).await,
            DeviceOp::Delete => self.delete().await,
        }
    }

    /// Stops the sandbox, if any, and removes the working pair and every
    /// committed generation. Each is first renamed to a stage in the working
    /// directory, on the same filesystem as the store, which recovery
    /// removes: a crash midway never leaves half a device that a wake would
    /// boot.
    async fn delete(&mut self) -> Result<(), OpError> {
        self.stop().await?;
        let working = self.working.directory().to_owned();
        let committed = self.core.store.device_directory(&self.device);
        let stages = (
            self.core.config.working().join(format!(".deleted-{}", uuid::Uuid::new_v4())),
            self.core.config.working().join(format!(".deleted-{}", uuid::Uuid::new_v4())),
        );
        blocking::run(move |off| -> std::io::Result<()> {
            for (directory, stage) in [(working, stages.0), (committed, stages.1)] {
                match fs_err::rename(&directory, &stage) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    renamed => renamed?,
                }
                sync(off, stage.parent().expect("a stage lies in its directory"))?;
                remove_tree(off, &stage)?;
            }
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// The sandbox exited without being asked to: stop what remains, save
    /// the working pair, and report the death even when those fail.
    async fn runtime_exited(&mut self, outcome: Result<(), ToolError>) {
        if let Err(error) = outcome {
            tracing::error!(event = "cloud_runtime.wait_failed", host_id = %self.device, %error, "a Cloud's runtime could not be waited for");
        }
        // No request named the machine: the save removes no fallback.
        let recovered = match self.stop().await {
            Ok(()) => self.save(Retention::Previous).await,
            Err(error) => Err(error),
        };
        if let Err(error) = recovered {
            tracing::error!(event = "cloud_recovery.failed", host_id = %self.device, error = %chain(&error), "a Cloud's recovery after its sandbox exited failed");
        }
        self.report_change().await;
        self.report_death().await;
    }

    async fn report_change(&self) {
        // The server stops listening at shutdown, when no one is left to tell.
        let _ = self.changes.send(self.device.clone()).await;
    }

    async fn report_death(&self) {
        // The server stops listening at shutdown, when no one is left to tell.
        let _ = self.deaths.send(self.device.clone()).await;
    }

    async fn working_manifest(&self) -> Result<Option<MachineImageState>, OpError> {
        let pair = self.working.clone();
        Ok(blocking::run(move |off| pair.manifest(off)).await?)
    }

    async fn committed(&self) -> Result<Option<MachineImageState>, OpError> {
        let store = self.core.store.clone();
        let device = self.device.clone();
        Ok(blocking::run(move |off| store.read(off, &device)).await?)
    }

    /// Stops the sandbox, if any; a failed close keeps it, and its slot.
    async fn stop(&mut self) -> Result<(), OpError> {
        let Some(mut sandbox) = self.runtime.take() else {
            return Ok(());
        };
        if let Err(error) = sandbox.close(&self.core, &self.working).await {
            self.runtime = Some(sandbox);
            return Err(error.into());
        }
        self.give_back_shared().await;
        Ok(())
    }

    /// Gives back the shared folder the sandbox that stopped took; a
    /// failure is logged, and the mount left behind serves the next start,
    /// or goes with a deletion or the manager's next start.
    async fn give_back_shared(&mut self) {
        if let Some(id) = self.shared.take()
            && let Err(error) = self.core.shared.release(&self.core, &id).await
        {
            tracing::warn!(event = "shared_folder.release_failed", host_id = %self.device, %error, "a shared folder stays mounted");
        }
    }

    /// Publishes the working pair, which no sandbox may be writing, keeping
    /// the older generations `retention` names.
    async fn save(&mut self, retention: Retention) -> Result<(), OpError> {
        if self.runtime.is_some() {
            return Err(OpError::ActiveWriter);
        }
        self.working
            .save(&self.core.tools, &self.core.store, &self.device, retention)
            .await?;
        Ok(())
    }

    /// Starts the device's sandbox on its newest storage: the working pair a
    /// crash left, saved first, else the committed generation, else a new one
    /// at `capacity`. An image smaller than `capacity` is enlarged before it
    /// is mounted.
    async fn wake(
        &mut self,
        boot: &ManagedBoot,
        machine: SandboxMachine,
        capacity: Capacity,
        shared: Option<&SharedFolder>,
    ) -> Result<(), OpError> {
        if self.runtime.is_some() {
            return Ok(());
        }
        self.save(Retention::of(machine)).await?;
        let state = match self.committed().await? {
            Some(state) => state,
            None => self.initialize(&self.base.clone(), capacity).await?,
        };
        let bases = self.core.store.bases();
        let base = bases.join(&state.base_version);
        // Demi's own programs come from the configured base, whatever base
        // the device is pinned to.
        let configured = bases.join(&self.base);
        let runner_release = base::runner_release(&configured).await?;
        let programs = configured.join("rootfs").join(PROGRAMS_PATH.trim_start_matches('/'));
        let stage = Stage::create(&self.core, "wake").await?;
        let copied = self.stage_working(state, capacity, stage.path()).await;
        stage.remove().await;
        copied?;
        let lease = self.core.slots.take()?;
        let mount = match shared {
            Some(folder) => {
                let source = self.core.shared.acquire(&self.core, &folder.id, folder.capacity_bytes).await?;
                self.shared = Some(folder.id.clone());
                Some((source, folder.path.clone()))
            }
            None => None,
        };
        let mut sandbox = Sandbox::new(&self.core, lease);
        let started = sandbox
            .start(
                &self.core,
                &self.working,
                &base.join("rootfs"),
                &programs,
                &runner_release,
                boot,
                machine,
                mount.as_ref().map(|(source, path)| (source.as_path(), path.as_str())),
            )
            .await;
        let Err(error) = started else {
            self.runtime = Some(sandbox);
            return Ok(());
        };
        match sandbox.close(&self.core, &self.working).await {
            Ok(()) => {
                self.give_back_shared().await;
                Err(error.into())
            }
            Err(cleanup) => {
                self.runtime = Some(sandbox);
                Err(OpError::StartAndCleanup(Parts(vec![
                    error.into(),
                    cleanup.into(),
                ])))
            }
        }
    }

    /// Copies the committed generation into `stage`, enlarges each copy
    /// smaller than `capacity` and records the capacity its superblock
    /// reports, and makes the stage the working pair. A larger image keeps
    /// its size: shrinking a filesystem risks its data.
    async fn stage_working(
        &self,
        mut state: MachineImageState,
        capacity: Capacity,
        stage: &Path,
    ) -> Result<(), OpError> {
        let sources = self.core.store.images(&self.device, &state.generation);
        let copies = ImagePair::in_directory(stage);
        let copied = copies.clone();
        let mut room = room::for_device(self.core.store.clone(), self.device.clone());
        blocking::run(move |off| -> std::io::Result<()> {
            for volume in Volume::ALL {
                clone_sparse(off, sources.get(volume), copied.get(volume), &mut room)?;
                sync(off, copied.get(volume))?;
            }
            Ok(())
        })
        .await?;
        for volume in Volume::ALL {
            let wanted = capacity.bytes(volume);
            if state.bytes(volume) < wanted {
                let enlarged = ext4::enlarge(&self.core.tools, copies.get(volume), wanted).await?;
                match volume {
                    Volume::System => state.system_bytes = enlarged,
                    Volume::Home => state.home_bytes = enlarged,
                }
            }
        }
        let stage = stage.to_owned();
        let working = self.working.directory().to_owned();
        blocking::run(move |off| -> std::io::Result<()> {
            write_json(off, &stage.join("manifest.json"), &state)?;
            sync(off, &stage)?;
            fs_err::rename(&stage, &working)?;
            sync(
                off,
                working
                    .parent()
                    .expect("a working pair lies in the working directory"),
            )
        })
        .await?;
        Ok(())
    }

    /// A device's first storage at `capacity`: a home from the base's
    /// skeleton, an empty system, published as its first generation.
    async fn initialize(
        &self,
        base: &BaseVersion,
        capacity: Capacity,
    ) -> Result<MachineImageState, OpError> {
        let stage = Stage::create(&self.core, "initial").await?;
        let made = self.make_first_generation(base, capacity, stage.path()).await;
        stage.remove().await;
        made
    }

    async fn make_first_generation(
        &self,
        base: &BaseVersion,
        capacity: Capacity,
        stage: &Path,
    ) -> Result<MachineImageState, OpError> {
        let skeleton = self.core.store.bases().join(base).join("rootfs/etc/skel");
        let root = stage.join("mkhome");
        let prepared = root.clone();
        blocking::run(move |off| -> std::io::Result<()> {
            use std::os::unix::fs::PermissionsExt;
            // The home filesystem's root is the sandbox's /home.
            fs_err::create_dir(&prepared)?;
            fs_err::set_permissions(&prepared, std::fs::Permissions::from_mode(0o755))?;
            skeleton::copy(off, &skeleton, &prepared.join("demi"))
        })
        .await?;
        let images = ImagePair::in_directory(stage);
        ext4::make_home(&self.core.tools, &root, &images.home, capacity.home_bytes).await?;
        ext4::make_system(&self.core.tools, &images.system, capacity.system_bytes).await?;
        let state = MachineImageState {
            generation: new_generation(),
            base_version: base.clone(),
            reset_id: None,
            system_bytes: capacity.system_bytes,
            home_bytes: capacity.home_bytes,
        };
        let store = self.core.store.clone();
        let device = self.device.clone();
        let published = state.clone();
        blocking::run(move |off| -> Result<(), OpError> {
            for volume in Volume::ALL {
                sync(off, images.get(volume))?;
            }
            // The device's first generation: there is no older one to keep.
            store.publish(off, &device, &published, images.as_deref(), Retention::Current)?;
            Ok(())
        })
        .await?;
        Ok(state)
    }

    /// Publishes the running pair without stopping its processes: pause,
    /// the frozen window, resume, then publication of the copies. A thaw or
    /// resume that fails ends the sandbox and reports its death.
    async fn checkpoint(&mut self, retention: Retention) -> Result<(), OpError> {
        if self.runtime.is_none() {
            return Ok(());
        }
        let Some(state) = self.working_manifest().await? else {
            return Err(OpError::NoWorkingManifest);
        };
        let stage = Stage::create(&self.core, "checkpoint").await?;
        let published = self.checkpoint_into(state, stage.path(), retention).await;
        stage.remove().await;
        published
    }

    async fn checkpoint_into(
        &mut self,
        state: MachineImageState,
        stage: &Path,
        retention: Retention,
    ) -> Result<(), OpError> {
        let sandbox = self
            .runtime
            .as_ref()
            .expect("the checkpoint checked for a sandbox");
        // Trimmed while the filesystems are still writable, so the copies
        // the frozen window makes hold only what the Cloud holds.
        sandbox.trim().await;
        let (captured, mut cleanup) = match sandbox.pause(&self.core).await {
            // Pausing failed: nothing is frozen, and the failure is reported
            // once the sandbox is resumed.
            Err(error) => (Err(OpError::from(error)), Vec::new()),
            Ok(()) => {
                let room = room::for_device(self.core.store.clone(), self.device.clone());
                let capture = sandbox.capture(&self.working, stage, room).await;
                let cleanup = capture.thaw_errors.into_iter().map(OpError::from).collect();
                (capture.copied.map_err(OpError::from), cleanup)
            }
        };
        if let Err(error) = sandbox.resume(&self.core).await {
            cleanup.push(error.into());
        }
        if !cleanup.is_empty() {
            // A runtime that could not thaw or resume must not look running.
            if let Err(error) = self.stop().await {
                cleanup.push(error);
            }
            self.report_death().await;
            let mut parts = Vec::new();
            if let Err(error) = captured {
                parts.push(error);
            }
            parts.extend(cleanup);
            return Err(OpError::CheckpointRecovery(Parts(parts)));
        }
        captured?;
        let saved = MachineImageState {
            generation: new_generation(),
            ..state
        };
        let store = self.core.store.clone();
        let device = self.device.clone();
        let copies = ImagePair::in_directory(stage);
        blocking::run(move |off| {
            store.publish(off, &device, &saved, copies.as_deref(), retention)
        })
        .await?;
        Ok(())
    }

    /// Publishes a fresh system on `base` at `capacity` with the saved home,
    /// once per `operation`; it does not boot.
    async fn reset(
        &mut self,
        operation: &str,
        base: &BaseVersion,
        capacity: Capacity,
        retention: Retention,
    ) -> Result<(), OpError> {
        let manifest = self.core.store.bases().join(base).join("manifest.json");
        let imported = blocking::run(move |_| fs_err::metadata(manifest)).await;
        match imported {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(OpError::MissingBase(base.clone()));
            }
            Err(error) => return Err(error.into()),
        }
        self.stop().await?;
        self.save(retention).await?;
        let state = match self.committed().await? {
            Some(state) => state,
            None => self.initialize(base, capacity).await?,
        };
        if state.reset_id.as_deref() == Some(operation) {
            return Ok(());
        }
        let stage = Stage::create(&self.core, "reset").await?;
        let published = self
            .publish_reset(state, operation, base, capacity, retention, stage.path())
            .await;
        stage.remove().await;
        published
    }

    async fn publish_reset(
        &self,
        state: MachineImageState,
        operation: &str,
        base: &BaseVersion,
        capacity: Capacity,
        retention: Retention,
        stage: &Path,
    ) -> Result<(), OpError> {
        let system = ImagePair::in_directory(stage).system;
        let system_bytes = capacity.system_bytes;
        ext4::make_system(&self.core.tools, &system, system_bytes).await?;
        // The home enters the new generation as a link to the saved image.
        let home = self.core.store.images(&self.device, &state.generation).home;
        let reset = MachineImageState {
            generation: new_generation(),
            base_version: base.clone(),
            reset_id: Some(operation.to_owned()),
            system_bytes,
            home_bytes: state.home_bytes,
        };
        let store = self.core.store.clone();
        let device = self.device.clone();
        blocking::run(move |off| -> Result<(), OpError> {
            sync(off, &system)?;
            let sources = ImagePair {
                system: system.as_path(),
                home: home.as_path(),
            };
            store.publish(off, &device, &reset, sources, retention)?;
            Ok(())
        })
        .await
    }
}

/// Waits for `runtime`'s exit; without a sandbox, forever.
async fn exited(runtime: &mut Option<Sandbox>) -> Result<(), ToolError> {
    match runtime {
        Some(sandbox) => sandbox.exited().await,
        None => std::future::pending().await,
    }
}
