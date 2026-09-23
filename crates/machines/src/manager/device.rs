//! A device's worker (`managed-hosts.md` § Control and ownership): it owns
//! the device's sandbox and runs the device's operations one at a time, in
//! arrival order. The sandbox's exit is one more event between operations,
//! never during one.

use std::{num::NonZeroU64, path::Path, rc::Rc};

use demi_machines_protocol::{BaseVersion, DeviceId, MachineImageState, RuntimeState, Volume};
use demi_runner_protocol::boot::ManagedBoot;
use tokio::sync::{mpsc, oneshot};

use super::{
    Core,
    errors::{OpError, Parts},
};
use crate::{
    blocking,
    fault,
    sandbox::Sandbox,
    server::chain,
    storage::{
        clone::clone_sparse,
        durable::{remove_tree, sync, write_json},
        ext4, skeleton,
        store::ImagePair,
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
    RuntimeState {
        reply: oneshot::Sender<RuntimeState>,
    },
}

/// The operations that change a device.
pub enum DeviceOp {
    Wake(ManagedBoot),
    /// Stop and save; also what `reconcile` and shutdown do to every device.
    Hibernate,
    Checkpoint,
    Grow(Volume, NonZeroU64),
    Reset { operation: String, base: BaseVersion },
}

pub struct DeviceWorker {
    device: DeviceId,
    core: Rc<Core>,
    /// The base a device's first storage is made from.
    base: BaseVersion,
    working: WorkingPair,
    runtime: Option<Sandbox>,
    deaths: mpsc::Sender<DeviceId>,
}

/// A staging directory in the working directory, whose name starts with a
/// dot so recovery removes it; the operation removes it on every path.
struct Stage {
    path: std::path::PathBuf,
}

impl Stage {
    async fn create(core: &Core, prefix: &str) -> Result<Self, OpError> {
        let path = core.config.working().join(format!(".{prefix}-{}", uuid::Uuid::new_v4()));
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
            tracing::warn!("machines: {error}");
        }
    }
}

impl DeviceWorker {
    pub fn new(device: DeviceId, core: Rc<Core>, base: BaseVersion, deaths: mpsc::Sender<DeviceId>) -> Self {
        Self {
            working: WorkingPair::new(&core.config.working(), &device),
            device,
            core,
            base,
            runtime: None,
            deaths,
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
                    // The requester may have gone; the operation completed anyway.
                    let _ = reply.send(outcome);
                }
                DeviceCommand::RuntimeState { reply } => {
                    let state = if self.runtime.is_some() {
                        RuntimeState::Running
                    } else {
                        RuntimeState::Stopped
                    };
                    // As above: no one may be waiting any more.
                    let _ = reply.send(state);
                }
            }
        }
    }

    async fn operate(&mut self, op: DeviceOp) -> Result<(), OpError> {
        match op {
            DeviceOp::Wake(boot) => self.wake(&boot).await,
            DeviceOp::Hibernate => {
                self.stop().await?;
                self.save().await
            }
            DeviceOp::Checkpoint => self.checkpoint().await,
            DeviceOp::Grow(volume, bytes) => self.grow(volume, bytes).await,
            DeviceOp::Reset { operation, base } => self.reset(&operation, &base).await,
        }
    }

    /// The sandbox exited without being asked to: stop what remains, save
    /// the working pair, and report the death even when those fail.
    async fn runtime_exited(&mut self, outcome: Result<(), ToolError>) {
        if let Err(error) = outcome {
            tracing::error!(device = %self.device, "machines: Cloud runtime wait failed: {error}");
        }
        let recovered = match self.stop().await {
            Ok(()) => self.save().await,
            Err(error) => Err(error),
        };
        if let Err(error) = recovered {
            tracing::error!(device = %self.device, "machines: Cloud recovery failed: {}", chain(&error));
        }
        self.report_death().await;
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
        Ok(())
    }

    /// Publishes the working pair, which no sandbox may be writing.
    async fn save(&mut self) -> Result<(), OpError> {
        if self.runtime.is_some() {
            return Err(OpError::ActiveWriter);
        }
        self.working.save(&self.core.tools, &self.core.store, &self.device).await?;
        Ok(())
    }

    /// Starts the device's sandbox on its newest storage: the working pair a
    /// crash left, saved first, else the committed generation, else a new one.
    async fn wake(&mut self, boot: &ManagedBoot) -> Result<(), OpError> {
        if self.runtime.is_some() {
            return Ok(());
        }
        self.save().await?;
        let state = match self.committed().await? {
            Some(state) => state,
            None => self.initialize(&self.base.clone()).await?,
        };
        let stage = Stage::create(&self.core, "wake").await?;
        let copied = self.stage_working(&state, stage.path()).await;
        stage.remove().await;
        copied?;
        fault::point("working-staged");
        let lease = self.core.slots.take()?;
        let mut sandbox = Sandbox::new(&self.core, lease);
        let base = self.core.store.bases().join(&state.base_version).join("rootfs");
        let started = sandbox.start(&self.core, &self.working, &base, boot).await;
        let Err(error) = started else {
            self.runtime = Some(sandbox);
            return Ok(());
        };
        match sandbox.close(&self.core, &self.working).await {
            Ok(()) => Err(error.into()),
            Err(cleanup) => {
                self.runtime = Some(sandbox);
                Err(OpError::StartAndCleanup(Parts(vec![error.into(), cleanup.into()])))
            }
        }
    }

    /// Copies the committed generation into `stage` and makes the stage the
    /// working pair.
    async fn stage_working(&self, state: &MachineImageState, stage: &Path) -> Result<(), OpError> {
        let sources = self.core.store.images(&self.device, &state.generation);
        let copies = ImagePair::in_directory(stage);
        let stage = stage.to_owned();
        let working = self.working.directory().to_owned();
        let state = state.clone();
        blocking::run(move |off| -> std::io::Result<()> {
            for volume in Volume::ALL {
                clone_sparse(off, sources.get(volume), copies.get(volume))?;
                sync(off, copies.get(volume))?;
            }
            write_json(off, &stage.join("manifest.json"), &state)?;
            sync(off, &stage)?;
            fs_err::rename(&stage, &working)?;
            sync(off, working.parent().expect("a working pair lies in the working directory"))
        })
        .await?;
        Ok(())
    }

    /// A device's first storage: a home from the base's skeleton, an empty
    /// system, published as its first generation.
    async fn initialize(&self, base: &BaseVersion) -> Result<MachineImageState, OpError> {
        let stage = Stage::create(&self.core, "initial").await?;
        let made = self.make_first_generation(base, stage.path()).await;
        stage.remove().await;
        made
    }

    async fn make_first_generation(&self, base: &BaseVersion, stage: &Path) -> Result<MachineImageState, OpError> {
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
        let config = &self.core.config;
        let images = ImagePair::in_directory(stage);
        ext4::make_home(&self.core.tools, &root, &images.home, config.home_bytes()).await?;
        ext4::make_system(&self.core.tools, &images.system, config.system_bytes()).await?;
        let state = MachineImageState {
            generation: new_generation(),
            base_version: base.clone(),
            reset_id: None,
            system_bytes: config.system_bytes(),
            home_bytes: config.home_bytes(),
        };
        let store = self.core.store.clone();
        let device = self.device.clone();
        let published = state.clone();
        blocking::run(move |off| -> Result<(), OpError> {
            for volume in Volume::ALL {
                sync(off, images.get(volume))?;
            }
            store.publish(off, &device, &published, images.as_deref())?;
            Ok(())
        })
        .await?;
        Ok(state)
    }

    /// Publishes the running pair without stopping its processes: pause,
    /// the frozen window, resume, then publication of the copies. A thaw or
    /// resume that fails ends the sandbox and reports its death.
    async fn checkpoint(&mut self) -> Result<(), OpError> {
        if self.runtime.is_none() {
            return Ok(());
        }
        let Some(state) = self.working_manifest().await? else {
            return Err(OpError::NoWorkingManifest);
        };
        let stage = Stage::create(&self.core, "checkpoint").await?;
        let published = self.checkpoint_into(state, stage.path()).await;
        stage.remove().await;
        published
    }

    async fn checkpoint_into(&mut self, state: MachineImageState, stage: &Path) -> Result<(), OpError> {
        let sandbox = self.runtime.as_ref().expect("the checkpoint checked for a sandbox");
        let (captured, mut cleanup) = match sandbox.pause(&self.core).await {
            // Pausing failed: nothing is frozen, and the failure is reported
            // once the sandbox is resumed.
            Err(error) => (Err(OpError::from(error)), Vec::new()),
            Ok(()) => {
                let capture = sandbox.capture(&self.working, stage).await;
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
        blocking::run(move |off| store.publish(off, &device, &saved, copies.as_deref())).await?;
        Ok(())
    }

    /// Grows a running volume, never shrinking it, and records the
    /// capacity its filesystem reports.
    async fn grow(&mut self, volume: Volume, bytes: NonZeroU64) -> Result<(), OpError> {
        let Some(sandbox) = self.runtime.as_ref() else {
            return Err(OpError::NotRunning);
        };
        let Some(state) = self.working_manifest().await? else {
            return Err(OpError::NoWorkingManifest);
        };
        if bytes <= state.bytes(volume) {
            return Ok(());
        }
        let capacity = sandbox.grow(&self.core, &self.working, volume, bytes).await?;
        let grown = state.with_bytes(volume, capacity);
        let pair = self.working.clone();
        blocking::run(move |off| pair.write_manifest(off, &grown)).await?;
        Ok(())
    }

    /// Publishes a fresh system on `base` with the saved home, once per
    /// `operation`; it does not boot.
    async fn reset(&mut self, operation: &str, base: &BaseVersion) -> Result<(), OpError> {
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
        self.save().await?;
        let state = match self.committed().await? {
            Some(state) => state,
            None => self.initialize(base).await?,
        };
        if state.reset_id.as_deref() == Some(operation) {
            return Ok(());
        }
        let stage = Stage::create(&self.core, "reset").await?;
        let published = self.publish_reset(state, operation, base, stage.path()).await;
        stage.remove().await;
        published
    }

    async fn publish_reset(
        &self,
        state: MachineImageState,
        operation: &str,
        base: &BaseVersion,
        stage: &Path,
    ) -> Result<(), OpError> {
        let system = ImagePair::in_directory(stage).system;
        let system_bytes = self.core.config.system_bytes();
        ext4::make_system(&self.core.tools, &system, system_bytes).await?;
        fault::point("reset-staged");
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
            store.publish(off, &device, &reset, sources)?;
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
