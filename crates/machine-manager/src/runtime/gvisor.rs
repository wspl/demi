//! The gVisor runtime: each device runs as a gVisor/systrap sandbox
//! (`crate::sandbox`) over a pair of ext4 images (`crate::storage`), on a
//! network slot of its own (`crate::network`).

use std::{net::Ipv4Addr, path::Path, rc::Rc};

use demi_machine_manager_protocol::image::PROGRAMS_PATH;
use demi_machine_manager_protocol::{
    BaseVersion, Capacity, DeviceId, LeftSandbox, MachineImageState, SandboxMachine, Volume,
};
use demi_runner_protocol::boot::ManagedBoot;

use super::{DeviceOp, Reports};
use crate::{
    blocking,
    config::Config,
    manager::{Core, OpError, Parts},
    network::{
        CloudNetwork,
        slots::{Slot, SlotPool},
    },
    sandbox::{
        Sandbox,
        files::USER_ID,
        runsc::Runsc,
    },
    server::chain,
    storage::{
        clone::clone_sparse,
        durable::{remove_tree, sync, write_json},
        base, ext4, room, tree,
        store::{ImagePair, ImageStore, Retention},
        working::{WorkingPair, new_generation},
    },
    tools::{Tool, ToolError, Tools},
};

/// What the gVisor runtime's devices share, made once at startup.
pub struct Gvisor {
    pub runsc: Runsc,
    pub store: ImageStore,
    pub network: CloudNetwork,
    pub slots: SlotPool,
}

impl Gvisor {
    /// The runtime whose Clouds use the resolvers `dns`.
    pub fn new(config: &Config, tools: &Tools, dns: Vec<Ipv4Addr>) -> Self {
        Self {
            runsc: Runsc::new(tools.clone(), config.runtime(), config.limits.is_some()),
            network: CloudNetwork::new(config.subnet, dns, tools.path(Tool::Nft).to_owned()),
            store: ImageStore::new(config.images()),
            slots: SlotPool::new(config.subnet, config.slots),
        }
    }
}

/// One device on gVisor: its working pair and the sandbox that runs on it.
pub struct Device {
    device: DeviceId,
    core: Rc<Core>,
    /// The base a device's first storage is made from.
    base: BaseVersion,
    working: WorkingPair,
    sandbox: Option<Sandbox>,
    reports: Reports,
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

impl Device {
    pub fn new(device: DeviceId, core: Rc<Core>, base: BaseVersion, reports: Reports) -> Self {
        Self {
            working: WorkingPair::new(&core.config.working(), &device),
            device,
            core,
            base,
            sandbox: None,
            reports,
        }
    }

    /// `device`, whose sandbox a manager before this one left running and
    /// this one took over.
    pub fn adopted(
        device: DeviceId,
        core: Rc<Core>,
        base: BaseVersion,
        reports: Reports,
        sandbox: Sandbox,
    ) -> Self {
        let mut device = Self::new(device, core, base, reports);
        device.sandbox = Some(sandbox);
        device
    }

    pub async fn operate(&mut self, op: DeviceOp) -> Result<(), OpError> {
        match op {
            DeviceOp::Wake {
                boot,
                machine,
                capacity,
            } => self.wake(&boot, machine, capacity).await,
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
        }
    }

    /// The sandbox exited without being asked to: stop what remains and
    /// save the working pair. The worker reports the death after.
    pub async fn recover(&mut self, outcome: std::io::Result<()>) {
        if let Err(error) = outcome {
            tracing::error!(event = "cloud_runtime.wait_failed", device_id = %self.device, %error, "a Cloud's runtime could not be waited for");
        }
        // No request named the machine: the save removes no fallback.
        let recovered = match self.stop().await {
            Ok(()) => self.save(Retention::Previous).await,
            Err(error) => Err(error),
        };
        if let Err(error) = recovered {
            tracing::error!(event = "cloud_recovery.failed", device_id = %self.device, error = %chain(&error), "a Cloud's recovery after its sandbox exited failed");
        }
    }

    /// Waits for the sandbox's exit; without a sandbox, forever.
    pub async fn exited(&mut self) -> Result<(), ToolError> {
        match &mut self.sandbox {
            Some(sandbox) => sandbox.exited().await,
            None => std::future::pending().await,
        }
    }

    /// The slot and the machine of the running sandbox; none when it is
    /// stopped.
    pub fn running(&self) -> Option<(Slot, SandboxMachine)> {
        self.sandbox.as_ref().and_then(Sandbox::admitted)
    }

    /// Hands the running sandbox's boot, programs' base and machine over
    /// to the next manager, releasing nothing of it.
    pub fn leave(mut self) -> Option<LeftSandbox> {
        let (record, programs, machine) = self.sandbox.take().and_then(Sandbox::leave)?;
        Some(LeftSandbox {
            device: self.device.clone(),
            id: record.id.as_str().to_owned(),
            slot: record.slot,
            programs,
            machine,
        })
    }

    async fn working_manifest(&self) -> Result<Option<MachineImageState>, OpError> {
        let pair = self.working.clone();
        Ok(blocking::run(move |off| pair.manifest(off)).await?)
    }

    async fn committed(&self) -> Result<Option<MachineImageState>, OpError> {
        let store = self.core.gvisor().store.clone();
        let device = self.device.clone();
        Ok(blocking::run(move |off| store.read(off, &device)).await?)
    }

    /// Stops the sandbox, if any; a failed close keeps it, and its slot.
    async fn stop(&mut self) -> Result<(), OpError> {
        let Some(mut sandbox) = self.sandbox.take() else {
            return Ok(());
        };
        if let Err(error) = sandbox.close(&self.core, &self.working).await {
            self.sandbox = Some(sandbox);
            return Err(error.into());
        }
        Ok(())
    }

    /// Publishes the working pair, which no sandbox may be writing, keeping
    /// the older generations `retention` names.
    async fn save(&mut self, retention: Retention) -> Result<(), OpError> {
        if self.sandbox.is_some() {
            return Err(OpError::ActiveWriter);
        }
        self.working
            .save(&self.core.tools, &self.core.gvisor().store, &self.device, retention)
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
    ) -> Result<(), OpError> {
        if self.sandbox.is_some() {
            return Ok(());
        }
        self.save(Retention::of(machine)).await?;
        let state = match self.committed().await? {
            Some(state) => state,
            None => self.initialize(&self.base.clone(), capacity).await?,
        };
        let bases = self.core.gvisor().store.bases();
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
        let lease = self.core.gvisor().slots.take()?;
        let mut sandbox = Sandbox::new(&self.core, lease);
        let started = sandbox
            .start(
                &self.core,
                &self.working,
                &base.join("rootfs"),
                &programs,
                &self.base,
                &runner_release,
                boot,
                machine,
            )
            .await;
        let Err(error) = started else {
            self.sandbox = Some(sandbox);
            return Ok(());
        };
        match sandbox.close(&self.core, &self.working).await {
            Ok(()) => Err(error.into()),
            Err(cleanup) => {
                self.sandbox = Some(sandbox);
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
        let sources = self.core.gvisor().store.images(&self.device, &state.generation);
        let copies = ImagePair::in_directory(stage);
        let copied = copies.clone();
        let mut room = room::for_device(self.core.gvisor().store.clone(), self.device.clone());
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
        let skeleton = self.core.gvisor().store.bases().join(base).join("rootfs/etc/skel");
        let root = stage.join("mkhome");
        let prepared = root.clone();
        blocking::run(move |off| -> std::io::Result<()> {
            use std::os::unix::fs::PermissionsExt;
            // The home filesystem's root is the sandbox's /home.
            fs_err::create_dir(&prepared)?;
            fs_err::set_permissions(&prepared, std::fs::Permissions::from_mode(0o755))?;
            tree::copy(off, &skeleton, &prepared.join("demi"), Some(USER_ID), None)
        })
        .await?;
        let images = ImagePair::in_directory(stage);
        ext4::make_populated(&self.core.tools, Volume::Home, &root, &images.home, capacity.home_bytes).await?;
        ext4::make_system(&self.core.tools, &images.system, capacity.system_bytes).await?;
        let state = MachineImageState {
            generation: new_generation(),
            base_version: base.clone(),
            reset_id: None,
            system_bytes: capacity.system_bytes,
            home_bytes: capacity.home_bytes,
        };
        let store = self.core.gvisor().store.clone();
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
        if self.sandbox.is_none() {
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
            .sandbox
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
                let room = room::for_device(self.core.gvisor().store.clone(), self.device.clone());
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
            self.reports.death().await;
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
        let store = self.core.gvisor().store.clone();
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
        let manifest = self.core.gvisor().store.bases().join(base).join("manifest.json");
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
        let home = self.core.gvisor().store.images(&self.device, &state.generation).home;
        let reset = MachineImageState {
            generation: new_generation(),
            base_version: base.clone(),
            reset_id: Some(operation.to_owned()),
            system_bytes,
            home_bytes: state.home_bytes,
        };
        let store = self.core.gvisor().store.clone();
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
