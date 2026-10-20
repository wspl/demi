//! The Linux interfaces the manager uses instead of administration tools:
//! mounts, loop devices, filesystem freezes and trims, and namespace
//! threads.

pub mod freeze;
pub mod loopdev;
pub mod mount;
#[cfg(test)]
pub(crate) mod testing;
pub mod thread_ns;
pub mod trim;

use std::{io, path::Path};

/// A failed system call on `path`, keeping the error's kind.
pub(crate) fn failed(action: &str, path: &Path, error: rustix::io::Errno) -> io::Error {
    let error = io::Error::from(error);
    io::Error::new(
        error.kind(),
        format!("{action} {}: {error}", path.display()),
    )
}
