//! Mounts through the kernel's interface (`managed-hosts.md` § Linux
//! control): the base bind, the ext4 working images, the system overlay and
//! the credentials tmpfs, each in the manager's private mount namespace.

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
        Ok(status) if status.stx_attributes_mask.contains(StatxAttributes::MOUNT_ROOT) => {
            Ok(Some(status.stx_attributes.contains(StatxAttributes::MOUNT_ROOT)))
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
    rustix::mount::mount(device, target, "ext4", MountFlags::NODEV, None::<&std::ffi::CStr>)
        .map_err(|error| failed("mounting", target, error))
}

/// Mounts a tmpfs at `target` with `options`, such as `size=1m,mode=0700`.
pub fn tmpfs(_: &OffLoop, target: &Path, options: &str) -> io::Result<()> {
    let data = CString::new(options).map_err(io::Error::other)?;
    rustix::mount::mount("tmpfs", target, "tmpfs", MountFlags::empty(), Some(data.as_c_str()))
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
    rustix::mount::mount("overlay", target, "overlay", MountFlags::empty(), Some(data.as_c_str()))
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::linux::{loopdev, testing::isolate};

    fn make_image(path: &Path) {
        let output = std::process::Command::new("mke2fs")
            .args(["-q", "-t", "ext4", "-F"])
            .arg(path)
            .arg("32m")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    #[ignore = "needs root: run the Linux suite with --ignored as root"]
    fn a_new_root_is_readable_under_a_restrictive_umask_and_a_users_mode_stays() {
        isolate();
        let off = OffLoop::in_test();
        // The service runs with umask 077; this thread has its own now.
        rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
        let directory = tempfile::tempdir().unwrap();
        let (base, volume, root) = (directory.path().join("base"), directory.path().join("volume"), directory.path().join("root"));
        for path in [&base, &volume, &root] {
            std::fs::create_dir(path).unwrap();
        }
        let image = directory.path().join("system.ext4");
        make_image(&image);
        let device = loopdev::attach(&off, &image).unwrap();
        ext4(&off, &device.path(), &volume).unwrap();
        drop(device);
        system_overlay(&off, &base, &volume, &root).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&root), 0o755);
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500)).unwrap();
        unmount(&off, &root).unwrap();
        system_overlay(&off, &base, &volume, &root).unwrap();
        assert_eq!(mode(&root), 0o500);
        unmount(&off, &root).unwrap();
        unmount(&off, &volume).unwrap();
    }

    #[test]
    #[ignore = "needs root: run the Linux suite with --ignored as root"]
    fn a_loop_device_detaches_on_unmount_and_after_a_failed_mount() {
        use crate::storage::clone::tests::{loop_attached, loop_detaches};
        isolate();
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("home.ext4");
        make_image(&image);
        let target = directory.path().join("home");
        std::fs::create_dir(&target).unwrap();
        let device = loopdev::attach(&off, &image).unwrap();
        ext4(&off, &device.path(), &target).unwrap();
        drop(device);
        assert!(loop_attached(&image));
        assert_eq!(mount_root(&off, &target).unwrap(), Some(true));
        unmount(&off, &target).unwrap();
        assert_eq!(mount_root(&off, &target).unwrap(), Some(false));
        assert!(loop_detaches(&image));
        // An image that is not ext4 fails to mount and leaves no device.
        let garbage = directory.path().join("garbage.img");
        std::fs::File::create(&garbage).unwrap().set_len(8 << 20).unwrap();
        let device = loopdev::attach(&off, &garbage).unwrap();
        assert!(ext4(&off, &device.path(), &target).is_err());
        drop(device);
        assert!(loop_detaches(&garbage));
        assert_eq!(mount_root(&off, &directory.path().join("absent")).unwrap(), None);
    }

    #[test]
    #[ignore = "needs root: run the Linux suite with --ignored as root"]
    fn removing_a_runtime_directory_keeps_a_surviving_mounts_contents() {
        use crate::sandbox::files::RuntimeDirectory;
        isolate();
        let off = OffLoop::in_test();
        let temporary = tempfile::tempdir().unwrap();
        let directory = RuntimeDirectory::new(temporary.path(), "demi-test");
        directory.create(&off).unwrap();
        tmpfs(&off, &directory.home(), "size=1m").unwrap();
        std::fs::write(directory.home().join("project"), "work").unwrap();
        assert!(directory.remove(&off).is_err());
        assert_eq!(std::fs::read_to_string(directory.home().join("project")).unwrap(), "work");
        unmount(&off, &directory.home()).unwrap();
        directory.remove(&off).unwrap();
    }
}
