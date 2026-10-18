//! One boot (`managed-hosts.md` § Container initialization): its resources
//! are acquired in a fixed order, each recorded where recovery finds it, and
//! released in reverse by [`Sandbox::close`], which tolerates every absence,
//! so it also cleans up a failed start and a boot recovery found.

use std::{io, path::Path, time::Duration};

use demi_machine_manager_protocol::{SandboxMachine, Volume};
use demi_runner_protocol::boot::ManagedBoot;
use tokio::task::JoinHandle;

use super::{
    cgroup::{self, CgroupError},
    files::{MOUNT_POINTS, RuntimeDirectory, SandboxId, SandboxRecord},
    oci,
    runsc::{RunscError, Status},
};
use crate::{
    blocking::{self, OffLoop},
    fault,
    linux::{freeze, loopdev, mount, trim},
    manager::Core,
    network::{
        NetworkError,
        slots::{Slot, SlotLease},
    },
    storage::{
        clone::clone_sparse,
        durable::{sync, write_json},
        ext4,
        store::ImagePair,
        working::WorkingPair,
    },
    tools::{Tool, ToolError, Tools},
};

/// Sandbox processes get this long to exit when asked to terminate.
const STOP_GRACE: Duration = Duration::from_secs(3);
const STOP_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Cloud backend differs from configured allowlist")]
    Allowlist,
    #[error("The backend's runner socket directory {0} does not exist: is the backend running?")]
    NoBackendSocket(std::path::PathBuf),
    #[error("Cloud {0} filesystem needs recovery: {1}")]
    NeedsRecovery(Volume, String),
    #[error("Cannot signal Cloud sandbox: {0}")]
    Signal(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Runsc(#[from] RunscError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Network(#[from] NetworkError),
    #[error(transparent)]
    Cgroup(#[from] CgroupError),
    #[error(transparent)]
    Oci(#[from] oci_spec::OciSpecError),
    #[error(transparent)]
    Ext4(#[from] ext4::Ext4Error),
}

/// One boot and what it holds.
pub struct Sandbox {
    record: SandboxRecord,
    slot: Slot,
    /// The slot's lease; a boot recovery found holds none.
    _lease: Option<SlotLease>,
    directory: RuntimeDirectory,
    /// `runsc wait`, which ends when the sandbox exits.
    waiter: Option<JoinHandle<Result<(), ToolError>>>,
}

/// What a checkpoint's frozen window produced: whether the copies were
/// made, and each thaw that failed.
pub struct Capture {
    pub copied: io::Result<()>,
    pub thaw_errors: Vec<io::Error>,
}

impl Sandbox {
    /// A new boot on `lease`'s slot.
    pub fn new(core: &Core, lease: SlotLease) -> Self {
        let record = SandboxRecord {
            id: SandboxId::new(),
            slot: lease.slot().index,
        };
        Self {
            directory: RuntimeDirectory::new(core.config.runtime(), record.id.as_str()),
            slot: lease.slot().clone(),
            _lease: Some(lease),
            record,
            waiter: None,
        }
    }

    /// The boot `record` names, as recovery finds it after a crash.
    pub fn recorded(core: &Core, record: SandboxRecord) -> Self {
        Self {
            directory: RuntimeDirectory::new(core.config.runtime(), record.id.as_str()),
            slot: core.slots.slot(record.slot),
            _lease: None,
            record,
            waiter: None,
        }
    }

    pub fn id(&self) -> &SandboxId {
        &self.record.id
    }

    /// The network slot the sandbox runs on.
    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    /// Starts the boot on the working pair with the base at `base` and
    /// Demi's programs from `programs`, the configured base's, whose runner
    /// is of `runner_release`. On an error the caller closes the sandbox,
    /// which releases what it holds.
    pub async fn start(
        &mut self,
        core: &Core,
        working: &WorkingPair,
        base: &Path,
        programs: &Path,
        runner_release: &str,
        boot: &ManagedBoot,
        machine: SandboxMachine,
    ) -> Result<(), SandboxError> {
        if boot.backend_url.url() != &core.config.backend_url {
            return Err(SandboxError::Allowlist);
        }
        // The backend makes the socket's directory, which outlives its
        // restarts; a sandbox mounts the directory, so each socket the
        // backend makes again reaches it.
        let backend_socket = core
            .config
            .runner_socket
            .parent()
            .expect("the configured socket lies in a directory")
            .to_owned();
        if !tokio::fs::metadata(&backend_socket).await.is_ok_and(|metadata| metadata.is_dir()) {
            return Err(SandboxError::NoBackendSocket(backend_socket));
        }
        let record = self.record.clone();
        let directory = self.directory.clone();
        let base = base.to_owned();
        let programs = programs.to_owned();
        let pair = working.clone();
        blocking::run(move |off| -> io::Result<()> {
            write_json(off, &pair.sandbox_record(), &record)?;
            sync(off, pair.directory())?;
            fault::point("sandbox-record");
            directory.create(off)?;
            mount::bind(off, &base, &directory.base())?;
            mount::remount_read_only(off, &directory.base())?;
            mount::bind(off, &programs, &directory.programs())?;
            mount::remount_read_only(off, &directory.programs())?;
            fault::point("base-bound");
            Ok(())
        })
        .await?;
        for volume in Volume::ALL {
            let image = working.images().get(volume).clone();
            check(&core.tools, volume, &image).await?;
            let target = self.directory.volume(volume);
            blocking::run(move |off| -> io::Result<()> {
                let device = loopdev::attach(off, &image)?;
                mount::ext4(off, &device.path(), &target)
            })
            .await?;
            fault::point("volume-mounted");
        }
        let directory = self.directory.clone();
        let credentials = boot.clone();
        let dns = core.config.dns.clone();
        blocking::run(move |off| -> io::Result<()> {
            mount::system_overlay(
                off,
                &directory.base(),
                &directory.volume(Volume::System),
                &directory.rootfs(),
            )?;
            mount::tmpfs(off, &directory.credentials(), "size=1m,mode=0700")?;
            directory.write_credentials(off, &credentials, &dns)?;
            Ok(())
        })
        .await?;
        fault::point("credentials-written");
        core.network.attach(&self.slot).await?;
        fault::point("network-attached");
        let spec = oci::spec(&oci::Boot {
            directory: &self.directory,
            namespace: &self.slot.namespace(),
            cgroup: core.config.limits.map(|limits| oci::Cgroup {
                name: self.record.id.as_str(),
                limits: limits.of(machine),
            }),
            runner_release,
            backend_socket: &backend_socket,
        })?;
        let directory = self.directory.clone();
        let log = blocking::run(move |off| -> io::Result<_> {
            use fs_err::os::unix::fs::OpenOptionsExt;
            write_json(off, &directory.config(), &spec)?;
            let log = fs_err::OpenOptions::new()
                .append(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.log())?;
            Ok((log.into_parts().0, directory.log()))
        })
        .await?;
        core.runsc
            .start(&self.record.id, self.directory.root(), log)
            .await?;
        fault::point("sandbox-started");
        let runsc = core.runsc.clone();
        let id = self.record.id.clone();
        self.waiter = Some(tokio::task::spawn_local(
            async move { runsc.wait(&id).await },
        ));
        Ok(())
    }

    /// Waits until the sandbox exits; a sandbox without a waiter never does.
    /// Dropping the wait keeps the waiter, so it is safe in a `select!`.
    pub async fn exited(&mut self) -> Result<(), ToolError> {
        let Some(waiter) = self.waiter.as_mut() else {
            return std::future::pending().await;
        };
        let outcome = waiter.await;
        self.waiter = None;
        match outcome {
            Ok(outcome) => outcome,
            Err(error) => Err(ToolError::Spawn {
                tool: Tool::Runsc,
                source: io::Error::other(error),
            }),
        }
    }

    pub async fn pause(&self, core: &Core) -> Result<(), ToolError> {
        core.runsc.pause(&self.record.id).await
    }

    pub async fn resume(&self, core: &Core) -> Result<(), ToolError> {
        core.runsc.resume(&self.record.id).await
    }

    /// Trims both working filesystems through their host mounts, so the
    /// blocks the Cloud freed become holes in its images before they are
    /// saved; a failure is logged and the save goes on.
    pub async fn trim(&self) {
        let directory = self.directory.clone();
        blocking::run(move |off| trim_mounted(off, &directory)).await;
    }

    /// The frozen window of a checkpoint, as one blocking job: freeze both
    /// filesystems, copy both working images into `stage` and sync the
    /// copies, then thaw. The guard thaws on every path, a panic included.
    pub async fn capture(&self, working: &WorkingPair, stage: &Path) -> Capture {
        let directory = self.directory.clone();
        let images = working.images();
        let copies = ImagePair::in_directory(stage);
        blocking::run(move |off| {
            let mut frozen = freeze::Frozen::new(off);
            let copied = (|| -> io::Result<()> {
                for volume in Volume::ALL {
                    frozen.freeze(&directory.volume(volume))?;
                }
                fault::point("frozen");
                for volume in Volume::ALL {
                    clone_sparse(off, images.get(volume), copies.get(volume))?;
                    sync(off, copies.get(volume))?;
                }
                Ok(())
            })();
            Capture {
                copied,
                thaw_errors: frozen.thaw_all(),
            }
        })
        .await
    }

    /// Stops the boot and releases everything it holds, in reverse order of
    /// acquisition; each step tolerates a resource that is already gone. A
    /// failure keeps the sandbox, so a later operation retries the close.
    pub async fn close(&mut self, core: &Core, working: &WorkingPair) -> Result<(), SandboxError> {
        let directory = self.directory.clone();
        // A Gofer may be waiting on a frozen filesystem; thaw before signals.
        blocking::run(move |off| -> io::Result<()> {
            for volume in Volume::ALL {
                thaw_if_mounted(off, &directory.volume(volume))?;
            }
            Ok(())
        })
        .await?;
        self.stop_runtime(core).await?;
        if let Some(waiter) = self.waiter.take() {
            // The sandbox was stopped; how its wait ended does not matter.
            let _ = waiter.await;
        }
        // Without the limits the sandbox has no cgroup; deleting the runtime
        // ended its Sentry and Gofer.
        if core.config.limits.is_some() {
            cgroup::fence(&self.record.id).await?;
        }
        let directory = self.directory.clone();
        blocking::run(move |off| {
            // The save that follows every close keeps only what the Cloud
            // holds; nothing writes the filesystems any more.
            trim_mounted(off, &directory);
            unmount_all(off, &directory)
        })
        .await?;
        core.network.detach(&self.slot).await?;
        let directory = self.directory.clone();
        let pair = working.clone();
        blocking::run(move |off| -> io::Result<()> {
            directory.remove(off)?;
            match fs_err::remove_file(pair.sandbox_record()) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            sync(off, pair.directory())
        })
        .await?;
        Ok(())
    }

    /// Asks every sandbox process to terminate, waits for them briefly, and
    /// deletes the runtime.
    async fn stop_runtime(&self, core: &Core) -> Result<(), SandboxError> {
        let id = &self.record.id;
        let status = core.runsc.status(id).await?;
        if let Some(status) = status.filter(|status| *status != Status::Stopped) {
            if status == Status::Paused {
                core.runsc.resume(id).await?;
            }
            let signalled = core.runsc.terminate(id).await?;
            if !signalled.status.success() && core.runsc.status(id).await? != Some(Status::Stopped)
            {
                return Err(SandboxError::Signal(signalled.message().to_owned()));
            }
            let deadline = tokio::time::Instant::now() + STOP_GRACE;
            while tokio::time::Instant::now() < deadline
                && core.runsc.status(id).await? == Some(Status::Running)
            {
                tokio::time::sleep(STOP_POLL).await;
            }
        }
        if core.runsc.status(id).await?.is_some() {
            core.runsc.delete(id).await?;
        }
        Ok(())
    }
}

impl Drop for Sandbox {
    /// A sandbox is dropped without a close only when the manager exits
    /// after a failed drain; its waiter ends with it, which kills `runsc
    /// wait` and leaves the sandbox itself to the stop-post recovery.
    fn drop(&mut self) {
        if let Some(waiter) = &self.waiter {
            waiter.abort();
        }
    }
}

/// `e2fsck -p` before a mount; exit 1 means it corrected the filesystem.
async fn check(tools: &Tools, volume: Volume, image: &Path) -> Result<(), SandboxError> {
    let output = tools
        .output(Tool::E2fsck, [Path::new("-p"), image], None)
        .await?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(SandboxError::NeedsRecovery(
            volume,
            output.message().to_owned(),
        ));
    }
    Ok(())
}

/// Trims each working filesystem mounted in `directory`; one that is not
/// mounted, as after a failed start, is skipped. A failure is logged: an
/// untrimmed image is saved whole, only larger.
fn trim_mounted(off: &OffLoop, directory: &RuntimeDirectory) {
    for volume in Volume::ALL {
        let mount = directory.volume(volume);
        let trimmed = match mount::mount_root(off, &mount) {
            Ok(Some(true)) => trim::trim(off, &mount).map(Some),
            Ok(_) => Ok(None),
            Err(error) => Err(error),
        };
        match trimmed {
            Ok(Some(bytes)) => tracing::debug!("machines: trimmed {bytes} bytes of {volume}"),
            Ok(None) => {}
            Err(error) => tracing::warn!("machines: the {volume} image stays untrimmed: {error}"),
        }
    }
}

fn thaw_if_mounted(off: &OffLoop, mount: &Path) -> io::Result<()> {
    if mount::mount_root(off, mount)? == Some(true) {
        freeze::thaw(off, mount)?;
    }
    Ok(())
}

/// Unmounts the boot's mounts, the overlay first; the working filesystems
/// are thawed first, and unmounting them detaches their loop devices.
fn unmount_all(off: &OffLoop, directory: &RuntimeDirectory) -> io::Result<()> {
    for name in MOUNT_POINTS {
        let path = directory.root().join(name);
        if mount::mount_root(off, &path)? != Some(true) {
            continue;
        }
        if name == "system" || name == "home" {
            freeze::thaw(off, &path)?;
        }
        mount::unmount(off, &path)?;
    }
    Ok(())
}
