//! The saved mount namespace: the manager pins its private mount namespace with
//! a handle in the execution host's namespace, so the filesystems it froze or
//! mounted stay reachable if it dies, and the next manager recovers them
//! through it.
//!
//! The manager binds the handle from PID 1's mount namespace. Linux binds a
//! mount namespace only into one with a lower id, and Linux 6.18 hands each
//! CPU its ids in batches of 4096: a namespace made after PID 1's, on a CPU
//! whose batch began before it, has a lower id, and the bind fails with
//! EINVAL. PID 1 runs in the initial namespace, numbered first, on most
//! hosts, but in a namespace of its own in WSL and in a container, so the
//! manager first gives itself a namespace numbered above PID 1's
//! ([`number_after_host`]). Linux 6.19 numbers namespaces in order again.
//!
//! The owner record must name the next manager's own state directory. Otherwise
//! startup stops and asks for the earlier manager to be recovered with its
//! original directory, so that changing directories cannot abandon that
//! manager's writers.
//!
//! Recovery runs as a separate process of the manager's own executable, started
//! inside the saved namespace. It inherits the manager's lock on the state
//! directory and refuses to run without it, so nobody can start it by hand
//! beside a live manager. Only after it succeeds does the manager release the
//! handle. If it fails, the handle stays and the next start retries. Recovery
//! has no deadline, for the reason filesystem checks have none.
//!
//! The files:
//!
//! ```text
//! /run/demi-machine-manager/mount-namespace              the handle: a bind of /proc/<pid>/ns/mnt
//! /run/demi-machine-manager/mount-namespace-owner.json   {"dataDir": ...}: whose state it holds
//! ```

use std::{
    ffi::OsString,
    io,
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
};

use rustix::{
    io::Errno,
    thread::{CpuSet, LinkNameSpaceType, UnshareFlags},
};
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

/// `NS_GET_ID` (`linux/nsfs.h`): the 64-bit id of a namespace, the number
/// Linux compares to refuse a bind that could loop.
const NS_GET_ID: rustix::ioctl::Opcode = rustix::ioctl::opcode::read::<u64>(0xb7, 0x0d);

/// The ids a CPU takes from the shared counter at once
/// (`COOKIE_LOCAL_BATCH`): the most namespaces one CPU makes before it hands
/// out ids above every namespace made so far.
const ID_BATCH: usize = 4096;

/// The id of the namespace at `path`, or none on a kernel without
/// `NS_GET_ID`, which numbers mount namespaces in the order it makes them.
fn namespace_id(path: &str) -> io::Result<Option<u64>> {
    let file = fs_err::File::open(path)?;
    // SAFETY: NS_GET_ID writes one u64.
    let getter = unsafe { rustix::ioctl::Getter::<NS_GET_ID, u64>::new() };
    // SAFETY: the descriptor is a namespace's, and the getter matches the opcode.
    match unsafe { rustix::ioctl::ioctl(file.file().as_fd(), getter) } {
        Ok(id) => Ok(Some(id)),
        Err(Errno::NOTTY) => Ok(None),
        Err(error) => Err(failed(path, error)),
    }
}

fn failed(path: &str, error: Errno) -> io::Error {
    io::Error::new(io::Error::from(error).kind(), format!("reading the id of {path}: {error}"))
}

/// Gives this process a copy of its mount namespace numbered above PID 1's,
/// so that [`SavedNamespace::pin`] can bind it into PID 1's: the copy made on
/// a CPU whose batch of ids began after PID 1's namespace is, as is any once
/// a CPU's batch runs out. The manager tries every CPU it may run on, as
/// snap-confine does, then makes copies on the last until its batch runs out,
/// for one whose CPUs all hold older batches. A process in PID 1's own
/// namespace keeps it, which the start then refuses.
///
/// Runs before the manager starts any thread: unshare(CLONE_NEWNS) moves
/// only the calling thread, so every thread must start after it to work in
/// the namespace that [`SavedNamespace::pin`] pins.
pub fn number_after_host() -> io::Result<()> {
    const HOST: &str = "/proc/1/ns/mnt";
    const OWN: &str = "/proc/thread-self/ns/mnt";
    let (Some(host), Some(own)) = (namespace_id(HOST)?, namespace_id(OWN)?) else {
        return Ok(());
    };
    if own >= host {
        return Ok(());
    }
    let allowed = rustix::thread::sched_getaffinity(None)?;
    let cpus: Vec<usize> = (0..CpuSet::MAX_CPU).filter(|&cpu| allowed.is_set(cpu)).collect();
    let last = *cpus.last().ok_or_else(|| io::Error::other("this process may run on no CPU"))?;
    let tries = cpus.iter().copied().chain(std::iter::repeat_n(last, ID_BATCH));
    let copy_on = |cpu: usize| -> io::Result<u64> {
        let mut one = CpuSet::new();
        one.set(cpu);
        rustix::thread::sched_setaffinity(None, &one)?;
        // SAFETY: CLONE_NEWNS moves only this thread, the process's only one,
        // into a copy of its mount namespace; it does not unshare the
        // descriptor table.
        unsafe { rustix::thread::unshare_unsafe(UnshareFlags::NEWNS) }?;
        namespace_id(OWN)?.ok_or_else(|| io::Error::other("the kernel stopped reporting namespace ids"))
    };
    let mut numbered = Ok(own);
    for cpu in tries {
        numbered = copy_on(cpu);
        if numbered.as_ref().map_or(true, |&own| own > host) {
            break;
        }
    }
    rustix::thread::sched_setaffinity(None, &allowed)?;
    let own = numbered?;
    if own <= host {
        return Err(io::Error::other(format!(
            "no copy of the manager's mount namespace is numbered above PID 1's: {own}, PID 1's {host}"
        )));
    }
    tracing::info!(event = "machine_manager.namespace_renumbered", host, own, "the manager's mount namespace was copied to one numbered above PID 1's");
    Ok(())
}

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

    /// Where the handle of the saved namespace lies, in the host's namespace.
    pub fn handle(&self) -> &Path {
        &self.handle
    }

    /// Whether the saved namespace holds this manager's state directory, as
    /// its owner record says; not when there is no record.
    pub fn owned(&self) -> io::Result<bool> {
        Ok(self.owner()?.is_some_and(|owner| owner == self.data))
    }

    /// Refuses a saved namespace that holds another state directory's
    /// sandboxes, before anything ends their processes: a manager with
    /// another configuration leaves them for one with theirs.
    pub fn refuse_other(&self) -> Result<(), NamespaceError> {
        match self.owner()? {
            Some(owner) if owner != self.data => Err(NamespaceError::OtherOwner(owner)),
            _ => Ok(()),
        }
    }

    /// The state directory the owner record names; none without a record.
    fn owner(&self) -> io::Result<Option<PathBuf>> {
        let bytes = match fs_err::read(&self.owner) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let owner: Owner = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::other(NamespaceError::Owner(self.owner.display().to_string(), error)))?;
        Ok(Some(owner.data_dir))
    }

    /// Recovers a namespace an earlier manager left: a process of this
    /// executable, started inside it with the state lock, thaws and fences
    /// what it holds and saves every working pair; then the handle goes. A
    /// failed recovery keeps the handle for the next start.
    pub async fn recover(&self, lock: &ManagerLock) -> Result<(), NamespaceError> {
        let handle = self.handle.clone();
        let saved = thread_ns::run(
            Namespace::HostMount,
            move |off| -> io::Result<Option<OwnedFd>> {
                if mount::mount_root(off, &handle)? != Some(true) {
                    return Ok(None);
                }
                Ok(Some(OwnedFd::from(
                    fs_err::File::open(&handle)?.into_parts().0,
                )))
            },
        )
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
            fs_err::OpenOptions::new()
                .write(true)
                .create(true)
                .mode(0o600)
                .open(&handle)?;
            write_json(off, &owner, &Owner { data_dir: data })?;
            sync(
                off,
                owner
                    .parent()
                    .expect("the owner record lies in the runtime directory"),
            )
        })
        .await?;
        let handle = self.handle.clone();
        let own = PathBuf::from(format!("/proc/{}/ns/mnt", std::process::id()));
        thread_ns::run(Namespace::HostMount, move |off| {
            mount::bind(off, &own, &handle)
        })
        .await?;
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
                rustix::io::fcntl_setfd(
                    BorrowedFd::borrow_raw(lock),
                    rustix::io::FdFlags::empty(),
                )?;
            } else {
                let mut inherited = std::mem::ManuallyDrop::new(OwnedFd::from_raw_fd(INHERITED));
                rustix::io::dup2(BorrowedFd::borrow_raw(lock), &mut inherited)?;
            }
            Ok(())
        });
    }
    command
}
