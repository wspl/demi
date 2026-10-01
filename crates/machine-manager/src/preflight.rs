//! The checks before the manager serves (`setup.md` § Linux requirements):
//! privileges, a private mount namespace, the pinned runsc, one filesystem
//! for the state directory, and a storage probe that mounts, freezes and
//! copies a small image.

use std::{io, num::NonZeroU64, os::unix::fs::MetadataExt, path::Path};

use crate::{
    blocking::{self, OffLoop},
    linux::{freeze, loopdev, mount},
    manager::Core,
    sandbox::runsc::{RuntimeRelease, reports_version},
    storage::{clone::clone_sparse, durable::remove_tree, ext4},
    tools::ToolError,
};

const PROBE_PREFIX: &str = ".preflight-";
const PROBE_BYTES: u64 = 32 << 20;

#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("Cloud manager requires privileged Linux service execution")]
    Privileges,
    #[error("Cloud manager requires a private mount namespace (systemd PrivateMounts=yes)")]
    SharedNamespace,
    #[error("Cloud requires runsc {0}")]
    Runsc(String),
    #[error("DEMI_MACHINES_DATA must be one filesystem: {0} and {1} are on different filesystems")]
    Filesystems(String, String),
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
pub fn require_one_filesystem(_: &OffLoop, working: &Path, images: &Path) -> Result<(), PreflightError> {
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
    let stage = core.config.data.join(format!("{PROBE_PREFIX}{}", uuid::Uuid::new_v4()));
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
        crate::fault::point("probe-frozen");
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
            if entry.file_name().to_string_lossy().starts_with(PROBE_PREFIX) {
                release(off, &entry.path())?;
            }
        }
        Ok(())
    })
    .await?;
    Ok(())
}
