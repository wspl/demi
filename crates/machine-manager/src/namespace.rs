//! The saved mount namespace (`managed-hosts.md` § Startup and recovery):
//! the manager pins its private mount namespace with a handle in the
//! execution host's namespace, so the filesystems it froze or mounted stay
//! reachable if it dies, and the next manager recovers them through it.
//!
//! ```text
//! /run/demi-machines/mount-namespace              the handle: a bind of /proc/<pid>/ns/mnt
//! /run/demi-machines/mount-namespace-owner.json   {"dataDir": ...}: whose state it holds
//! ```

use std::{
    ffi::OsString,
    io,
    os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    path::PathBuf,
};

use rustix::thread::LinkNameSpaceType;
use serde::{Deserialize, Serialize};

use crate::{
    blocking,
    config::Config,
    linux::{
        mount,
        thread_ns::{self, Namespace},
    },
    lock::{INHERITED, ManagerLock},
    storage::durable::{sync, write_json},
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Owner {
    data_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum NamespaceError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("{0} is not a valid namespace owner record: {1}")]
    Owner(String, serde_json::Error),
    #[error("Recover the previous Cloud manager with its original state directory: {}", .0.display())]
    OtherOwner(PathBuf),
    #[error("Cloud namespace recovery failed: {0}")]
    Recovery(std::process::ExitStatus),
}

pub struct SavedNamespace {
    runtime: PathBuf,
    handle: PathBuf,
    owner: PathBuf,
    data: PathBuf,
}

impl SavedNamespace {
    pub fn new(config: &Config) -> Self {
        let runtime = config.runtime().to_owned();
        Self {
            handle: runtime.join("mount-namespace"),
            owner: runtime.join("mount-namespace-owner.json"),
            data: config.data.clone(),
            runtime,
        }
    }

    /// Recovers a namespace an earlier manager left: a process of this
    /// executable, started inside it with the state lock, thaws and fences
    /// what it holds and saves every working pair; then the handle goes. A
    /// failed recovery keeps the handle for the next start.
    pub async fn recover(&self, lock: &ManagerLock) -> Result<(), NamespaceError> {
        let handle = self.handle.clone();
        let saved = thread_ns::run(Namespace::HostMount, move |off| -> io::Result<Option<OwnedFd>> {
            if mount::mount_root(off, &handle)? != Some(true) {
                return Ok(None);
            }
            Ok(Some(OwnedFd::from(fs_err::File::open(&handle)?.into_parts().0)))
        })
        .await?;
        let Some(saved) = saved else {
            return Ok(());
        };
        let owner_path = self.owner.clone();
        let owner = blocking::run(move |_| -> Result<Owner, NamespaceError> {
            let bytes = fs_err::read(&owner_path)?;
            serde_json::from_slice(&bytes)
                .map_err(|error| NamespaceError::Owner(owner_path.display().to_string(), error))
        })
        .await?;
        if owner.data_dir != self.data {
            return Err(NamespaceError::OtherOwner(owner.data_dir));
        }
        let status = recovery_process(&saved, lock).status().await?;
        if !status.success() {
            return Err(NamespaceError::Recovery(status));
        }
        // Opened through the handle, the descriptor keeps the handle's mount
        // busy; closed, it lets the release unmount the handle.
        drop(saved);
        self.release().await
    }

    /// Pins this manager's namespace: its runtime directory becomes a
    /// private mount in the host's namespace (nsfs refuses a handle under a
    /// shared mount), the owner record names the state directory, and the
    /// handle binds this process's namespace from the host's side (a
    /// namespace cannot hold a handle to itself).
    pub async fn pin(&self) -> Result<(), NamespaceError> {
        let runtime = self.runtime.clone();
        thread_ns::run(Namespace::HostMount, move |off| {
            if mount::mount_root(off, &runtime)? != Some(true) {
                mount::bind(off, &runtime, &runtime)?;
            }
            mount::make_private(off, &runtime)
        })
        .await?;
        let (handle, owner) = (self.handle.clone(), self.owner.clone());
        let data = self.data.clone();
        blocking::run(move |off| -> io::Result<()> {
            use fs_err::os::unix::fs::OpenOptionsExt;
            fs_err::OpenOptions::new().write(true).create(true).mode(0o600).open(&handle)?;
            write_json(off, &owner, &Owner { data_dir: data })?;
            sync(off, owner.parent().expect("the owner record lies in the runtime directory"))
        })
        .await?;
        let handle = self.handle.clone();
        let own = PathBuf::from(format!("/proc/{}/ns/mnt", std::process::id()));
        thread_ns::run(Namespace::HostMount, move |off| mount::bind(off, &own, &handle)).await?;
        Ok(())
    }

    /// Releases the handle, in the host's namespace and in this one, where a
    /// copy may have been inherited, and removes it with its owner record.
    pub async fn release(&self) -> Result<(), NamespaceError> {
        let handle = self.handle.clone();
        thread_ns::run(Namespace::HostMount, move |off| {
            if mount::mount_root(off, &handle)? == Some(true) {
                mount::unmount(off, &handle)?;
            }
            Ok(())
        })
        .await?;
        let (handle, owner) = (self.handle.clone(), self.owner.clone());
        blocking::run(move |off| -> io::Result<()> {
            if mount::mount_root(off, &handle)? == Some(true) {
                mount::unmount(off, &handle)?;
            }
            for file in [&handle, &owner] {
                match fs_err::remove_file(file) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                    _ => {}
                }
            }
            Ok(())
        })
        .await?;
        Ok(())
    }
}

/// This executable with `--recover-namespace`, entering `namespace` and
/// holding the state lock at descriptor [`INHERITED`] when it starts. The
/// arguments are this process's, less `--recover`; the environment, where
/// the service's settings are, is inherited.
fn recovery_process(namespace: &OwnedFd, lock: &ManagerLock) -> tokio::process::Command {
    let args: Vec<OsString> = std::env::args_os()
        .skip(1)
        .filter(|arg| arg != "--recover")
        .chain([OsString::from("--recover-namespace")])
        .collect();
    // Recovery is the manager's own code, not a tool.
    let mut command = tokio::process::Command::new("/proc/self/exe");
    command.args(args).kill_on_drop(true);
    let namespace = namespace.as_raw_fd();
    let lock = lock.data().as_raw_fd();
    // SAFETY: the closure runs in the child between fork and exec and makes
    // only the setns, dup2 and fcntl system calls, which are
    // async-signal-safe. Both descriptors stay open in the parent until the
    // child has started, and the child owns descriptor INHERITED from here.
    unsafe {
        command.pre_exec(move || {
            let namespace = BorrowedFd::borrow_raw(namespace);
            rustix::thread::move_into_link_name_space(namespace, Some(LinkNameSpaceType::Mount))?;
            if lock == INHERITED {
                // dup2 onto itself would keep close-on-exec set.
                rustix::io::fcntl_setfd(BorrowedFd::borrow_raw(lock), rustix::io::FdFlags::empty())?;
            } else {
                let mut inherited = std::mem::ManuallyDrop::new(OwnedFd::from_raw_fd(INHERITED));
                rustix::io::dup2(BorrowedFd::borrow_raw(lock), &mut inherited)?;
            }
            Ok(())
        });
    }
    command
}
