//! The checks before the manager serves (`setup.md` § Linux requirements):
//! privileges, a private mount namespace, the pinned runsc, one filesystem
//! for the state directory, and a storage probe that mounts, freezes and
//! copies a small image.

use std::{io, num::NonZeroU64, os::unix::fs::MetadataExt, path::Path};

use demi_machine_manager_protocol::runtime::RuntimeRelease;

use crate::{
    blocking::{self, OffLoop},
    linux::{freeze, loopdev, mount},
    manager::Core,
    sandbox::runsc::reports_version,
    storage::{clone::clone_sparse, durable::remove_tree, ext4},
    tools::ToolError,
};

const PROBE_PREFIX: &str = ".preflight-";
const PROBE_BYTES: u64 = 32 << 20;
/// How long a probe sandbox may take to run `/bin/true`.
const RUNTIME_PROBE: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("Cloud manager requires privileged Linux service execution")]
    Privileges,
    #[error("Cloud manager requires a private mount namespace (systemd PrivateMounts=yes)")]
    SharedNamespace,
    #[error("Cloud requires runsc {0}")]
    Runsc(String),
    #[error(
        "DEMI_MACHINE_MANAGER_DATA must be one filesystem: {0} and {1} are on different filesystems"
    )]
    Filesystems(String, String),
    #[error("Cloud's gVisor sandbox does not run on this machine: {0}")]
    Runtime(ToolError),
    #[error(
        "Cloud's gVisor sandbox does not run on this machine: every process here starts below a seccomp listener, and gVisor cannot add the one it intercepts system calls with (WSL installs one with networkingMode=mirrored; networkingMode=nat in .wslconfig does not): {0}"
    )]
    InheritedSeccomp(ToolError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Ext4(#[from] ext4::Ext4Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Requires root, which the mounts, namespaces and firewall need.
pub fn require_root() -> Result<(), PreflightError> {
    if !rustix::process::geteuid().is_root() {
        return Err(PreflightError::Privileges);
    }
    Ok(())
}

/// Requires a mount namespace of the manager's own, which keeps its storage
/// mounts off the host.
pub fn require_private_namespace(_: &OffLoop) -> Result<(), PreflightError> {
    let own = fs_err::metadata("/proc/self/ns/mnt")?;
    let host = fs_err::metadata("/proc/1/ns/mnt")?;
    if (own.dev(), own.ino()) == (host.dev(), host.ino()) {
        return Err(PreflightError::SharedNamespace);
    }
    Ok(())
}

/// Runs `/bin/true` in a gVisor sandbox on the host's root, with the
/// platform the profile uses and no network or cgroup, which shows the
/// kernel lets runsc run at all; the probe's state goes with it.
pub async fn probe_runtime(core: &Core) -> Result<(), PreflightError> {
    let state = core
        .config
        .runtime()
        .join(format!("{PROBE_PREFIX}{}", uuid::Uuid::new_v4()));
    let created = state.clone();
    blocking::run(move |off| crate::storage::durable::create_private(off, &created)).await?;
    let root = format!("--root={}", state.display());
    let ran = core
        .tools
        .run(
            crate::tools::Tool::Runsc,
            [
                root.as_str(),
                "--platform=systrap",
                "--network=none",
                "--ignore-cgroups",
                "--sidecar-usage-policy=STRICT",
                "do",
                "/bin/true",
            ],
            Some(RUNTIME_PROBE),
        )
        .await;
    blocking::run(move |off| remove_tree(off, &state)).await?;
    if let Err(error) = ran {
        // The sandbox's own failure only says that it ended.
        let taken = blocking::run_on_new_thread("seccomp-probe", |_| Ok(listener_taken())).await?;
        return Err(if taken {
            PreflightError::InheritedSeccomp(error)
        } else {
            PreflightError::Runtime(error)
        });
    }
    Ok(())
}

/// Whether a seccomp filter this thread inherited has a listener, which
/// keeps systrap from adding its own: the kernel allows one listener in a
/// chain of filters. The thread asks for one on a filter that allows every
/// call; a filter binds only the thread it is installed on, so it ends with
/// the thread.
fn listener_taken() -> bool {
    let mut allow = [libc::sock_filter {
        code: (libc::BPF_RET | libc::BPF_K) as u16,
        jt: 0,
        jf: 0,
        k: libc::SECCOMP_RET_ALLOW,
    }];
    let program = libc::sock_fprog {
        len: 1,
        filter: allow.as_mut_ptr(),
    };
    // SAFETY: `program` and the filter it points to outlive the call, which
    // only reads them.
    let listener = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_NEW_LISTENER,
            &program,
        )
    };
    if listener >= 0 {
        // SAFETY: the call returned this descriptor, and nothing else holds it.
        unsafe { libc::close(listener as libc::c_int) };
        return false;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EBUSY)
}

/// Requires the configured runsc to report exactly the pinned version.
pub async fn require_runsc(core: &Core) -> Result<(), PreflightError> {
    let version = RuntimeRelease::pinned().version();
    let reported = core.runsc.version().await?;
    if !reports_version(&reported, &version) {
        return Err(PreflightError::Runsc(version));
    }
    Ok(())
}

/// Requires the working and image directories on one filesystem, because
/// publication links images between them.
pub fn require_one_filesystem(
    _: &OffLoop,
    working: &Path,
    images: &Path,
) -> Result<(), PreflightError> {
    fs_err::create_dir_all(working)?;
    fs_err::create_dir_all(images)?;
    if fs_err::metadata(working)?.dev() != fs_err::metadata(images)?.dev() {
        return Err(PreflightError::Filesystems(
            working.display().to_string(),
            images.display().to_string(),
        ));
    }
    Ok(())
}

/// Makes a small image, mounts it through a loop device under an overlay,
/// writes through the overlay, freezes the filesystem and copies the image;
/// the probe is released whatever happens.
pub async fn probe_storage(core: &Core) -> Result<(), PreflightError> {
    let stage = core
        .config
        .data
        .join(format!("{PROBE_PREFIX}{}", uuid::Uuid::new_v4()));
    let probed = probe(core, &stage).await;
    let released = {
        let stage = stage.clone();
        blocking::run(move |off| release(off, &stage)).await
    };
    probed?;
    released?;
    Ok(())
}

async fn probe(core: &Core, stage: &Path) -> Result<(), PreflightError> {
    let created = stage.to_owned();
    blocking::run(move |_| -> io::Result<()> {
        fs_err::create_dir(&created)?;
        for name in ["volume", "merged", "base"] {
            fs_err::create_dir(created.join(name))?;
        }
        Ok(())
    })
    .await?;
    let image = stage.join("volume.ext4");
    let bytes = NonZeroU64::new(PROBE_BYTES).expect("the probe has a size");
    ext4::make_system(&core.tools, &image, bytes).await?;
    let stage = stage.to_owned();
    blocking::run(move |off| -> io::Result<()> {
        let volume = stage.join("volume");
        let device = loopdev::attach(off, &image)?;
        mount::ext4(off, &device.path(), &volume)?;
        drop(device);
        mount::system_overlay(off, &stage.join("base"), &volume, &stage.join("merged"))?;
        fs_err::write(stage.join("merged/probe"), "Cloud storage preflight")?;
        freeze::freeze(off, &volume)?;
        clone_sparse(off, &image, &stage.join("copy.ext4"))
    })
    .await?;
    Ok(())
}

/// Thaws and unmounts a probe, then removes it.
fn release(off: &OffLoop, stage: &Path) -> io::Result<()> {
    let volume = stage.join("volume");
    if mount::mount_root(off, &volume)? == Some(true) {
        freeze::thaw(off, &volume)?;
    }
    for name in ["merged", "volume"] {
        let path = stage.join(name);
        if mount::mount_root(off, &path)? == Some(true) {
            mount::unmount(off, &path)?;
        }
    }
    remove_tree(off, stage)
}

/// Releases the probes an interrupted manager left in `data`.
pub async fn release_probes(data: &Path) -> Result<(), PreflightError> {
    let data = data.to_owned();
    blocking::run(move |off| -> io::Result<()> {
        for entry in fs_err::read_dir(&data)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(PROBE_PREFIX)
            {
                release(off, &entry.path())?;
            }
        }
        Ok(())
    })
    .await?;
    Ok(())
}
