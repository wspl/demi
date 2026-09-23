//! Support for the tests that need root. They run only when asked for
//! (`--ignored`, as root) and never touch the execution host's state: each
//! moves its thread into new mount and network namespaces first, and the
//! threads and processes it starts inherit them.

use rustix::{
    mount::MountPropagationFlags,
    thread::UnshareFlags,
};

/// Isolates the calling test thread: new mount and network namespaces, every
/// mount private, and fresh tmpfs mounts over `/run` so namespace files and
/// runtime directories of the host's manager stay out of reach.
pub fn isolate() {
    assert!(rustix::process::geteuid().is_root(), "root tests run as root");
    // SAFETY: CLONE_FS, CLONE_NEWNS and CLONE_NEWNET change only this
    // thread's filesystem context and namespaces, not its descriptor table.
    unsafe { rustix::thread::unshare_unsafe(UnshareFlags::FS | UnshareFlags::NEWNS | UnshareFlags::NEWNET) }
        .expect("new namespaces");
    rustix::mount::mount_change("/", MountPropagationFlags::PRIVATE | MountPropagationFlags::REC)
        .expect("private mounts");
    rustix::mount::mount("tmpfs", "/run", "tmpfs", rustix::mount::MountFlags::empty(), None::<&std::ffi::CStr>)
        .expect("a private /run");
}
