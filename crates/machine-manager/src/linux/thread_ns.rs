//! Work inside another namespace (`concurrency.md` § Machine manager): each
//! job gets a new thread that enters the namespace, runs the job and exits,
//! so no pooled thread ever runs later work in the wrong namespace.

use std::{
    io,
    os::fd::AsFd,
    path::{Path, PathBuf},
};

use rustix::thread::{LinkNameSpaceType, UnshareFlags};

use crate::blocking::{self, OffLoop};

/// The namespace a job runs in.
#[derive(Debug, Clone)]
pub enum Namespace {
    /// The execution host's mount namespace, PID 1's: the namespace handle
    /// lives there.
    HostMount,
    /// The network namespace bound at the path, such as a slot's
    /// `/run/netns/demi-3`.
    Network(PathBuf),
    /// A new network namespace, which the job may bind to a path.
    NewNetwork,
}

/// Runs `job` on a new thread inside `namespace` and returns its result. The
/// thread exits when the job returns.
pub async fn run<T: Send + 'static>(
    namespace: Namespace,
    job: impl FnOnce(&OffLoop) -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    blocking::run_on_new_thread("demi-machines-namespace", move |off| {
        enter(&namespace)?;
        job(off)
    })
    .await
}

fn enter(namespace: &Namespace) -> io::Result<()> {
    match namespace {
        Namespace::HostMount => enter_link(Path::new("/proc/1/ns/mnt"), LinkNameSpaceType::Mount, true),
        Namespace::Network(path) => enter_link(path, LinkNameSpaceType::Network, false),
        Namespace::NewNetwork => {
            // SAFETY: CLONE_NEWNET moves only this thread into a new network
            // namespace; it does not unshare the descriptor table.
            unsafe { rustix::thread::unshare_unsafe(UnshareFlags::NEWNET) }.map_err(io::Error::from)
        }
    }
}

fn enter_link(path: &Path, kind: LinkNameSpaceType, own_filesystem: bool) -> io::Result<()> {
    let namespace = fs_err::File::open(path)?;
    if own_filesystem {
        // A thread that shares its filesystem context with others cannot
        // enter a mount namespace.
        // SAFETY: CLONE_FS gives this thread its own root, working directory
        // and umask; it does not unshare the descriptor table.
        unsafe { rustix::thread::unshare_unsafe(UnshareFlags::FS) }?;
    }
    rustix::thread::move_into_link_name_space(namespace.file().as_fd(), Some(kind))?;
    Ok(())
}
