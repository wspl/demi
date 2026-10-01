//! The manager's exclusive locks (`managed-hosts.md` § Control and
//! ownership): one on its state directory, one on its runtime directory, so
//! a second manager on either is refused. The kernel releases them when the
//! process ends, however it ends.

use std::{
    io,
    os::fd::{AsFd, BorrowedFd},
    path::{Path, PathBuf},
};

use fs_err::os::unix::fs::OpenOptionsExt;
use rustix::{fs::FlockOperation, io::Errno};

use crate::blocking::OffLoop;

/// The descriptor the recovery process receives the state lock on.
pub const INHERITED: i32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Another Cloud manager owns {}", .0.display())]
    Owned(PathBuf),
    #[error("the recovery process needs the manager's lock on {}", .0.display())]
    NotInherited(PathBuf),
}

/// Both locks, held while this value lives.
pub struct ManagerLock {
    data: fs_err::File,
    _runtime: fs_err::File,
}

fn lock_file(directory: &Path) -> PathBuf {
    directory.join("manager.lock")
}

fn take(path: &Path) -> Result<fs_err::File, LockError> {
    let file = fs_err::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    match rustix::fs::flock(file.file(), FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(file),
        Err(Errno::WOULDBLOCK) => Err(LockError::Owned(path.to_owned())),
        Err(error) => Err(crate::linux::failed("locking", path, error).into()),
    }
}

impl ManagerLock {
    pub fn acquire(_: &OffLoop, data: &Path, runtime: &Path) -> Result<Self, LockError> {
        Ok(Self {
            data: take(&lock_file(data))?,
            _runtime: take(&lock_file(runtime))?,
        })
    }

    /// The state lock's descriptor, which the recovery process inherits.
    pub fn data(&self) -> BorrowedFd<'_> {
        self.data.file().as_fd()
    }
}

/// Checks that descriptor [`INHERITED`] is this state directory's lock file
/// and that this process may hold the lock: it shares the lock of the
/// manager that started it, or no manager holds it. A process started by
/// hand beside a running manager has neither, so it cannot recover live
/// devices.
pub fn verify_inherited(_: &OffLoop, data: &Path) -> Result<(), LockError> {
    let path = lock_file(data);
    // SAFETY: the manager placed its lock descriptor at INHERITED before it
    // executed this process, which never closes it.
    let inherited = unsafe { BorrowedFd::borrow_raw(INHERITED) };
    let refused = || LockError::NotInherited(path.clone());
    let held = rustix::fs::fstat(inherited).map_err(|_| refused())?;
    let expected = rustix::fs::stat(&path).map_err(|_| refused())?;
    if (held.st_dev, held.st_ino) != (expected.st_dev, expected.st_ino) {
        return Err(refused());
    }
    rustix::fs::flock(inherited, FlockOperation::NonBlockingLockExclusive).map_err(|_| refused())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_manager_is_refused_with_the_path() {
        let off = OffLoop::in_test();
        let data = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let first = ManagerLock::acquire(&off, data.path(), runtime.path()).unwrap();
        let error = ManagerLock::acquire(&off, data.path(), runtime.path())
            .err()
            .expect("the second manager is refused");
        assert_eq!(
            error.to_string(),
            format!("Another Cloud manager owns {}", data.path().join("manager.lock").display())
        );
        drop(first);
        ManagerLock::acquire(&off, data.path(), runtime.path()).unwrap();
    }
}
