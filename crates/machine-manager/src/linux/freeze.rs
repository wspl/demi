//! Filesystem freezes for a checkpoint: `FIFREEZE` flushes a mounted
//! filesystem and blocks its writers; `FITHAW` releases it.

use std::{
    io,
    os::fd::AsFd,
    path::{Path, PathBuf},
};

use linux_raw_sys::ioctl::{FIFREEZE, FITHAW};
use rustix::{io::Errno, ioctl::NoArg};

use super::failed;
use crate::blocking::OffLoop;

/// What a thaw found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thawed {
    Thawed,
    /// The filesystem was not frozen, as after a freeze that failed or a
    /// thaw that ran already.
    NotFrozen,
}

fn root(mount: &Path) -> io::Result<fs_err::File> {
    fs_err::File::open(mount)
}

/// Freezes the filesystem mounted at `mount`.
pub fn freeze(_: &OffLoop, mount: &Path) -> io::Result<()> {
    let directory = root(mount)?;
    // SAFETY: FIFREEZE ignores its argument; the descriptor is a filesystem's.
    unsafe { rustix::ioctl::ioctl(directory.file().as_fd(), NoArg::<{ FIFREEZE }>::new()) }
        .map_err(|error| failed("freezing", mount, error))
}

/// Thaws the filesystem mounted at `mount`.
pub fn thaw(_: &OffLoop, mount: &Path) -> io::Result<Thawed> {
    let directory = root(mount)?;
    // SAFETY: FITHAW ignores its argument; the descriptor is a filesystem's.
    match unsafe { rustix::ioctl::ioctl(directory.file().as_fd(), NoArg::<{ FITHAW }>::new()) } {
        Ok(()) => Ok(Thawed::Thawed),
        Err(Errno::INVAL) => Ok(Thawed::NotFrozen),
        Err(error) => Err(failed("thawing", mount, error)),
    }
}

/// Filesystems frozen for one checkpoint. Each freeze is recorded before it
/// is issued, because a failed one may still have frozen the filesystem;
/// [`Frozen::thaw_all`] thaws them all, and dropping the guard without it,
/// as a panic does, thaws what is left.
pub struct Frozen<'a> {
    off: &'a OffLoop,
    mounts: Vec<PathBuf>,
}

impl<'a> Frozen<'a> {
    pub fn new(off: &'a OffLoop) -> Self {
        Self {
            off,
            mounts: Vec::new(),
        }
    }

    pub fn freeze(&mut self, mount: &Path) -> io::Result<()> {
        self.mounts.push(mount.to_owned());
        freeze(self.off, mount)
    }

    /// Thaws every recorded filesystem and returns what failed.
    pub fn thaw_all(mut self) -> Vec<io::Error> {
        self.thaw_recorded()
    }

    fn thaw_recorded(&mut self) -> Vec<io::Error> {
        self.mounts
            .drain(..)
            .filter_map(|mount| thaw(self.off, &mount).err())
            .collect()
    }
}

impl Drop for Frozen<'_> {
    fn drop(&mut self) {
        for error in self.thaw_recorded() {
            tracing::error!("machines: {error}");
        }
    }
}
