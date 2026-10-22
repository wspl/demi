//! Mounts through the kernel's interface: the base bind, the ext4 working
//! images, the system overlay and the credentials tmpfs, each in the
//! manager's private mount namespace.

use std::{ffi::CString, io, path::Path};

use rustix::{
    fs::{AtFlags, CWD, StatxAttributes, StatxFlags},
    io::Errno,
    mount::{MountFlags, MountPropagationFlags, UnmountFlags},
};

use super::failed;
use crate::blocking::OffLoop;

/// Whether `path` is the root of a mount; `None` when nothing is there.
pub fn mount_root(_: &OffLoop, path: &Path) -> io::Result<Option<bool>> {
    let flags = AtFlags::NO_AUTOMOUNT | AtFlags::SYMLINK_NOFOLLOW;
    match rustix::fs::statx(CWD, path, flags, StatxFlags::empty()) {
        Ok(status)
            if status
                .stx_attributes_mask
                .contains(StatxAttributes::MOUNT_ROOT) =>
        {
            Ok(Some(
                status.stx_attributes.contains(StatxAttributes::MOUNT_ROOT),
            ))
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the kernel does not report mount roots (Linux 5.8 or later is required)",
        )),
        Err(Errno::NOENT) => Ok(None),
        Err(error) => Err(failed("inspecting", path, error)),
    }
}

/// Binds `source` onto `target`.
pub fn bind(_: &OffLoop, source: &Path, target: &Path) -> io::Result<()> {
    rustix::mount::mount_bind(source, target).map_err(|error| failed("binding", target, error))
}

/// Makes the bind at `target` read-only.
pub fn remount_read_only(_: &OffLoop, target: &Path) -> io::Result<()> {
    rustix::mount::mount_remount(target, MountFlags::BIND | MountFlags::RDONLY, "")
        .map_err(|error| failed("remounting", target, error))
}

/// Mounts the ext4 filesystem on `device` at `target`, without device files.
pub fn ext4(_: &OffLoop, device: &Path, target: &Path) -> io::Result<()> {
    rustix::mount::mount(
        device,
        target,
        "ext4",
        MountFlags::NODEV,
        None::<&std::ffi::CStr>,
    )
    .map_err(|error| failed("mounting", target, error))
}

/// Mounts a tmpfs at `target` with `options`, such as `size=1m,mode=0700`.
pub fn tmpfs(_: &OffLoop, target: &Path, options: &str) -> io::Result<()> {
    let data = CString::new(options).map_err(io::Error::other)?;
    rustix::mount::mount(
        "tmpfs",
        target,
        "tmpfs",
        MountFlags::empty(),
        Some(data.as_c_str()),
    )
    .map_err(|error| failed("mounting", target, error))
}

/// Mounts the system overlay: the read-only `base` below the upper and
/// work directories of the system filesystem mounted at `volume`. A new
/// upper directory gets mode 0755, because OverlayFS presents it as the
/// sandbox's `/` and the service's umask would hide it; an existing one
/// keeps the mode its user set.
pub fn system_overlay(off: &OffLoop, base: &Path, volume: &Path, target: &Path) -> io::Result<()> {
    let upper = volume.join("upper");
    let work = volume.join("work");
    create_with_mode(&upper, 0o755)?;
    create_with_mode(&work, 0o700)?;
    overlay(off, base, &upper, &work, target)
}

/// Creates the directory `path` with exactly `mode`, or leaves an existing
/// one as it is.
fn create_with_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match fs_err::create_dir(path) {
        Ok(()) => fs_err::set_permissions(path, std::fs::Permissions::from_mode(mode)),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

fn overlay(_: &OffLoop, lower: &Path, upper: &Path, work: &Path, target: &Path) -> io::Result<()> {
    let options = format!(
        "lowerdir={},upperdir={},workdir={},index=off,metacopy=off,redirect_dir=off",
        lower.display(),
        upper.display(),
        work.display()
    );
    let data = CString::new(options).map_err(io::Error::other)?;
    rustix::mount::mount(
        "overlay",
        target,
        "overlay",
        MountFlags::empty(),
        Some(data.as_c_str()),
    )
    .map_err(|error| failed("mounting", target, error))
}

/// Unmounts `target`.
pub fn unmount(_: &OffLoop, target: &Path) -> io::Result<()> {
    rustix::mount::unmount(target, UnmountFlags::empty())
        .map_err(|error| failed("unmounting", target, error))
}

/// Detaches the mount at `target` now; the kernel releases it when its last
/// user goes, as `ip netns delete` does with a namespace file.
pub fn detach(_: &OffLoop, target: &Path) -> io::Result<()> {
    rustix::mount::unmount(target, UnmountFlags::DETACH)
        .map_err(|error| failed("unmounting", target, error))
}

/// Stops mount events propagating to and from `target`.
pub fn make_private(_: &OffLoop, target: &Path) -> io::Result<()> {
    rustix::mount::mount_change(target, MountPropagationFlags::PRIVATE)
        .map_err(|error| failed("changing the propagation of", target, error))
}
