//! Simple Isolation: each device a directory tree of its own, its system a
//! full copy of a base, run under proot. It needs no root, and it keeps a
//! device's files apart from the others' and from the host's, so a mistaken
//! `rm -rf /` in one Cloud removes that Cloud's tree alone. It is no boundary
//! against a program that tries to break out: proot rewrites the paths of
//! the system calls it knows, and one it misses reaches the host.
//!
//! ```text
//! <data>/simple/<device>/
//!   record.json    the base the system is pinned to, the last reset, the user
//!   process.json   the running proot, while one runs
//!   system/        `/`: a copy of the base, then the user's changes
//!   home/          `/home`, which a reset keeps
//!   shm/           `/dev/shm`, emptied at each boot
//!   proot/         proot's own files, emptied at each boot
//!   proot.log      proot's output
//! ```
//!
//! A boot copies, never binds, what the device must not change into its
//! system: Demi's programs from the configured base's `/opt/demi`, the boot
//! record, the resolver and hosts files. A bound directory is writable
//! through proot, so `rm -rf /` would reach it. The runner reaches the
//! backend on an abstract socket, which has no file to remove
//! (`demi_runner_protocol::boot::abstract_socket`).
//!
//! The first process is `tini`, which starts the runner, as on gVisor. The
//! processes run as a faked root with `/home/demi` as their home: proot fakes
//! every file's owner as the user it runs as, so `sudo` under a faked `demi`
//! would find its configuration owned by `demi` and refuse.
//!
//! A stop signals the first process, `SIGTERM` and then `SIGKILL`. When it
//! ends, `--kill-on-exit` makes proot end every process it traced and exit.
//! Ending proot instead would leave them running, since a tracer's death
//! detaches its tracees, so a proot that dies unasked leaves them, and the
//! manager ends what remains of a device before it boots it again ([`fence`]):
//!
//! - A manager that runs as root runs each device as a user of its own, from
//!   [`USERS`], and ends every process of that user. The device's directories
//!   are that user's, in a directory of root's that only that user's group
//!   may pass through, so no other device reads them.
//! - One that does not runs every device as itself, and ends every process
//!   that runs a program from the device's directory. proot runs every
//!   program of a Cloud through its loader, which it writes into the
//!   device's `proot/`, so the kernel names that file as each one's program
//!   (`/proc/<pid>/exe`), whatever the program does to its environment. A
//!   program that gets past proot and runs one of the host's escapes the
//!   fence.
//!
//! A Cloud on Simple Isolation has no checkpoints, since its files are its
//! storage (a checkpoint syncs them), no storage, CPU or memory limits, and
//! no network of its own: it shares the host's network and ports.

use std::{
    cell::RefCell,
    collections::BTreeSet,
    io,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Stdio,
    rc::Rc,
    time::Duration,
};

use demi_machine_manager_protocol::{
    BaseVersion, DeviceId, LeftSimple, SandboxMachine,
    image::{INIT_PATH, PROGRAMS_PATH, RUNNER_PATH},
};
use demi_runner_protocol::{
    boot::{ManagedBoot, TEMPORARY_DIRECTORY, abstract_socket},
    release::RELEASE_ENV,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::DeviceOp;
use crate::{
    blocking::{self, OffLoop},
    config::Config,
    linux::process,
    manager::{Core, OpError},
    sandbox::{files, oci::BOOT_RECORD},
    storage::{
        base,
        durable::{remove_tree, sync, write_json},
        tree,
    },
    tools::Tool,
};

/// The users devices run as under a manager that runs as root: Debian
/// reserves 60000 to 64999 for no package or person, and a container whose
/// users map only the first 65536 has them.
const USERS: std::ops::Range<u32> = 61000..65000;

/// The resolver file the host's own programs read.
const HOST_RESOLVER: &str = "/etc/resolv.conf";

/// The first process gets this long to end when asked, as a gVisor
/// sandbox's processes do.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// How long a process that was killed gets to exit.
const KILLED_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum SimpleError {
    #[error("The users for Clouds ({}–{}) are all taken", USERS.start, USERS.end - 1)]
    NoUser,
    #[error("A Cloud's processes did not end: {0:?}")]
    Fence(Vec<i32>),
    #[error("proot could not run a Cloud: {0}")]
    Probe(String),
}

/// What Simple Isolation's devices share.
pub struct Simple {
    pub trees: Trees,
    proot: PathBuf,
    /// Whether each device runs as a user of its own: the manager runs as
    /// root.
    users: bool,
    /// The users devices hold, from their records.
    taken: RefCell<BTreeSet<u32>>,
}

/// Where the devices' trees are: `<data>/simple`.
#[derive(Debug, Clone)]
pub struct Trees {
    directory: PathBuf,
}

impl Trees {
    pub fn tree(&self, device: &DeviceId) -> Tree {
        Tree {
            root: self.directory.join(device.as_str()),
        }
    }

    /// The devices whose systems are on Simple Isolation.
    pub fn devices(&self, off: &OffLoop) -> io::Result<Vec<DeviceId>> {
        let entries = match fs_err::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut devices = Vec::new();
        for entry in entries {
            let entry = entry?;
            // A name that is no device's was not made by the manager.
            let Some(device) = entry.file_name().to_str().and_then(|name| DeviceId::parse(name).ok()) else {
                continue;
            };
            if self.read(off, &device)?.is_some() {
                devices.push(device);
            }
        }
        Ok(devices)
    }

    /// The device's record, none when its system is not on Simple Isolation.
    pub fn read(&self, off: &OffLoop, device: &DeviceId) -> io::Result<Option<TreeRecord>> {
        read_json(off, &self.tree(device).record())
    }
}

/// A device's directories and files.
#[derive(Debug, Clone)]
pub struct Tree {
    root: PathBuf,
}

impl Tree {
    fn record(&self) -> PathBuf {
        self.root.join("record.json")
    }
    fn process(&self) -> PathBuf {
        self.root.join("process.json")
    }
    fn system(&self) -> PathBuf {
        self.root.join("system")
    }
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn shm(&self) -> PathBuf {
        self.root.join("shm")
    }
    fn scratch(&self) -> PathBuf {
        self.root.join("proot")
    }
    fn log(&self) -> PathBuf {
        self.root.join("proot.log")
    }
}

/// `record.json`: what the device's system is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeRecord {
    /// The base the system was copied from.
    pub base_version: BaseVersion,
    /// The reset that copied it, which a retry of the same reset finds.
    pub reset_id: Option<String>,
    /// The user the device runs as, under a manager that runs as root.
    pub user: Option<u32>,
}

/// `process.json`: the running proot, which recovery fences and the next
/// manager may take over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessRecord {
    pid: i32,
    started: u64,
    /// The host's boot, after which the pid names no process of the device.
    boot: String,
    machine: SandboxMachine,
}

fn read_json<T: serde::de::DeserializeOwned>(_: &OffLoop, path: &Path) -> io::Result<Option<T>> {
    match fs_err::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| io::Error::other(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Replaces the directory `path` with an empty one of mode `mode` owned by
/// `owner`.
fn empty_directory(off: &OffLoop, path: &Path, owner: Option<u32>, mode: u32) -> io::Result<()> {
    remove_tree(off, path)?;
    fs_err::create_dir(path)?;
    if let Some(owner) = owner {
        fs_err::os::unix::fs::chown(path, Some(owner), Some(owner))?;
    }
    fs_err::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// The path of `inside` in `tree`'s system, each directory above it made a
/// directory of the system's own: a link or a file there is replaced by an
/// empty directory owned by `owner`. The device may have made one a link,
/// such as `/opt` to `/srv`, which the manager on the host would otherwise
/// follow out of the tree, to the host's `/srv`. Nothing of the device runs
/// while it boots, so nothing changes them meanwhile.
fn within(tree: &Tree, inside: &str, owner: Option<u32>) -> io::Result<PathBuf> {
    let relative = Path::new(inside.trim_start_matches('/'));
    let mut path = tree.system();
    for component in relative.parent().into_iter().flat_map(Path::components) {
        path.push(component);
        match fs_err::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => continue,
            Ok(_) => fs_err::remove_file(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs_err::create_dir(&path)?;
        if let Some(owner) = owner {
            fs_err::os::unix::fs::chown(&path, Some(owner), Some(owner))?;
        }
        fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path.join(relative.file_name().unwrap_or_default()))
}

/// Writes `bytes` to `path`, replacing what was there, a link included, with
/// a file of `mode` owned by `owner`.
fn place(path: &Path, bytes: &[u8], owner: Option<u32>, mode: u32) -> io::Result<()> {
    match fs_err::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs_err::remove_dir_all(path)?,
        Ok(_) => fs_err::remove_file(path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = path.parent() {
        fs_err::create_dir_all(parent)?;
    }
    fs_err::write(path, bytes)?;
    if let Some(owner) = owner {
        fs_err::os::unix::fs::chown(path, Some(owner), Some(owner))?;
    }
    fs_err::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

impl Simple {
    pub fn new(config: &Config) -> Self {
        Self {
            trees: Trees {
                directory: config.data.join("simple"),
            },
            proot: config.proot.clone(),
            users: rustix::process::geteuid().is_root(),
            taken: RefCell::default(),
        }
    }

    pub fn tree(&self, device: &DeviceId) -> Tree {
        self.trees.tree(device)
    }

    /// Makes the directory, and learns the users devices hold. Under root,
    /// the data directory and this one let the devices' users pass through
    /// to their own directories, and nothing more.
    pub async fn prepare(&self, data: &Path) -> io::Result<()> {
        let (directory, data, users) = (self.trees.directory.clone(), data.to_owned(), self.users);
        let taken = blocking::run(move |off| -> io::Result<BTreeSet<u32>> {
            let mode = if users { 0o711 } else { 0o700 };
            match fs_err::create_dir(&directory) {
                Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
                _ => {}
            }
            for path in [&data, &directory] {
                fs_err::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
            }
            let mut taken = BTreeSet::new();
            for entry in fs_err::read_dir(&directory)? {
                let path = entry?.path().join("record.json");
                if let Some(user) = read_json::<TreeRecord>(off, &path)?.and_then(|record| record.user) {
                    taken.insert(user);
                }
            }
            Ok(taken)
        })
        .await?;
        *self.taken.borrow_mut() = taken;
        Ok(())
    }

    /// Runs the host's `/bin/true` under proot with a faked root, as a
    /// device's processes run: whether this host lets proot trace. Its own
    /// files go to `scratch`, which it empties.
    pub async fn probe(&self, scratch: &Path) -> Result<(), SimpleError> {
        let made = scratch.to_owned();
        blocking::run(move |off| empty_directory(off, &made, None, 0o700))
            .await
            .map_err(|error| SimpleError::Probe(error.to_string()))?;
        let output = tokio::process::Command::new(&self.proot)
            .args(["-0", "/bin/true"])
            .env_clear()
            .env("PROOT_TMP_DIR", scratch)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| SimpleError::Probe(format!("{}: {error}", self.proot.display())))?;
        let removed = scratch.to_owned();
        let _ = blocking::run(move |off| remove_tree(off, &removed)).await;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr);
            return Err(SimpleError::Probe(message.trim().to_owned()));
        }
        Ok(())
    }

    /// The lowest user no device holds.
    fn take_user(&self) -> Result<u32, SimpleError> {
        let mut taken = self.taken.borrow_mut();
        let user = USERS.clone().find(|user| !taken.contains(user)).ok_or(SimpleError::NoUser)?;
        taken.insert(user);
        Ok(user)
    }

    /// Ends every process of `device` but the manager's own: its user's, or
    /// those that run a program from its directory, and waits for them to
    /// exit.
    pub async fn fence(&self, device: &DeviceId, user: Option<u32>) -> Result<(), OpError> {
        // The kernel names a program by its real path, with no link in it,
        // whatever path proot was given.
        let directory = self.tree(device).root;
        let directory = blocking::run(move |_| match fs_err::canonicalize(&directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(directory),
            real => real,
        })
        .await?;
        let me = std::process::id() as i32;
        let owned = move |pid: i32| {
            pid != me
                && !process::zombie(pid)
                && match user {
                    Some(user) => process::user(pid) == Some(user),
                    None => process::runs_from(pid, &directory),
                }
        };
        let deadline = tokio::time::Instant::now() + KILLED_GRACE;
        loop {
            let left: Vec<i32> = blocking::run({
                let owned = owned.clone();
                move |_| -> io::Result<Vec<i32>> { Ok(process::pids()?.into_iter().filter(|pid| owned(*pid)).collect()) }
            })
            .await?;
            if left.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(io::Error::other(SimpleError::Fence(left)).into());
            }
            for pid in &left {
                signal(*pid, rustix::process::Signal::KILL);
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Ends what remains of each device's proot that is not in `running`:
    /// what a manager that stopped left of them. Runs before any device
    /// operation.
    pub async fn recover(&self, running: &[DeviceId]) -> Result<(), OpError> {
        let simple = self.trees.directory.clone();
        let devices = blocking::run({
            let simple = simple.clone();
            move |off| -> io::Result<Vec<(DeviceId, Option<u32>)>> {
                let mut found = Vec::new();
                let entries = match fs_err::read_dir(&simple) {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
                    Err(error) => return Err(error),
                };
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let Ok(device) = DeviceId::parse(name.clone()) else {
                        continue;
                    };
                    // A stage of a reset that stopped midway has replaced nothing.
                    for inner in fs_err::read_dir(entry.path())? {
                        let inner = inner?;
                        if inner.file_name().to_string_lossy().starts_with('.') {
                            remove_tree(off, &inner.path())?;
                        }
                    }
                    let tree = Tree { root: entry.path() };
                    if tree.process().exists() {
                        let user = read_json::<TreeRecord>(off, &tree.record())?.and_then(|record| record.user);
                        found.push((device, user));
                    }
                }
                Ok(found)
            }
        })
        .await?;
        for (device, user) in devices {
            if running.contains(&device) {
                continue;
            }
            self.fence(&device, user).await?;
            let process = self.tree(&device).process();
            blocking::run(move |_| match fs_err::remove_file(&process) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            })
            .await?;
        }
        Ok(())
    }
}

/// A device's running proot.
pub struct Process {
    pid: i32,
    started: u64,
    machine: SandboxMachine,
    /// Ends when proot exits.
    waiter: Option<JoinHandle<io::Result<()>>>,
}

impl Drop for Process {
    /// A process is dropped without a stop only when the manager exits or
    /// leaves it to the next one: its waiter ends, and proot runs on.
    fn drop(&mut self) {
        if let Some(waiter) = &self.waiter {
            waiter.abort();
        }
    }
}

impl Process {
    /// The process `left` names, which a manager before this one left
    /// running: none when it ended or its id names another process now, or
    /// when the device's record names another.
    pub async fn take_over(simple: &Simple, left: &LeftSimple) -> io::Result<Option<Self>> {
        let path = simple.trees.tree(&left.device).process();
        let record = blocking::run(move |off| read_json::<ProcessRecord>(off, &path)).await?;
        let Some(record) = record.filter(|record| record.pid == left.pid && record.started == left.started) else {
            return Ok(None);
        };
        if record.boot != process::boot()? || process::started(record.pid) != Some(record.started) {
            return Ok(None);
        }
        let Some(pid) = rustix::process::Pid::from_raw(record.pid) else {
            return Ok(None);
        };
        let descriptor = match rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()) {
            Ok(descriptor) => descriptor,
            Err(rustix::io::Errno::SRCH) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let waiter = tokio::task::spawn_local(async move {
            let readable = tokio::io::unix::AsyncFd::new(descriptor)?;
            // A process's descriptor becomes readable when it exits.
            let _ = readable.readable().await?;
            Ok(())
        });
        Ok(Some(Self {
            pid: record.pid,
            started: record.started,
            machine: record.machine,
            waiter: Some(waiter),
        }))
    }

    /// The proot's id, whose descendants the sweep of a manager's unit keeps.
    pub fn pid(&self) -> i32 {
        self.pid
    }
}

/// Sends `signal` to `pid`; one that is gone needs none.
fn signal(pid: i32, signal: rustix::process::Signal) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid) {
        let _ = rustix::process::kill_process(pid, signal);
    }
}

/// Whether the process `waiter` waits for ends within `within`; one already
/// waited for has.
async fn ended(waiter: &mut Option<JoinHandle<io::Result<()>>>, within: Duration) -> bool {
    match waiter {
        Some(waiter) => tokio::time::timeout(within, waiter).await.is_ok(),
        None => true,
    }
}

/// One device on Simple Isolation.
pub struct Device {
    device: DeviceId,
    core: Rc<Core>,
    /// The base a device's first system is copied from.
    base: BaseVersion,
    process: Option<Process>,
}

impl Device {
    pub fn new(device: DeviceId, core: Rc<Core>, base: BaseVersion) -> Self {
        Self {
            device,
            core,
            base,
            process: None,
        }
    }

    /// `device`, whose proot a manager before this one left running and this
    /// one took over.
    pub fn adopted(device: DeviceId, core: Rc<Core>, base: BaseVersion, process: Process) -> Self {
        let mut adopted = Self::new(device, core, base);
        adopted.process = Some(process);
        adopted
    }

    fn simple(&self) -> &Simple {
        &self.core.simple
    }

    fn tree(&self) -> Tree {
        self.simple().tree(&self.device)
    }

    pub async fn operate(&mut self, op: DeviceOp) -> Result<(), OpError> {
        match op {
            DeviceOp::Wake { boot, machine, .. } => self.wake(boot, machine).await,
            DeviceOp::Hibernate { .. } => self.stop().await,
            // Its files are its storage: a checkpoint makes them durable.
            DeviceOp::Checkpoint { .. } => self.sync().await,
            DeviceOp::Reset { operation, base, .. } => self.reset(&operation, &base).await,
        }
    }

    /// Waits for proot's exit; without one, forever.
    pub async fn exited(&mut self) -> io::Result<()> {
        let Some(waiter) = self.process.as_mut().and_then(|process| process.waiter.as_mut()) else {
            return std::future::pending().await;
        };
        let outcome = waiter.await;
        if let Some(process) = self.process.as_mut() {
            process.waiter = None;
        }
        outcome.map_err(io::Error::other)?
    }

    /// proot exited without being asked to: end what remains of the device.
    pub async fn recover(&mut self, outcome: io::Result<()>) {
        if let Err(error) = outcome {
            tracing::error!(event = "cloud_runtime.wait_failed", host_id = %self.device, %error, "a Cloud's runtime could not be waited for");
        }
        if let Err(error) = self.stop().await {
            tracing::error!(event = "cloud_recovery.failed", host_id = %self.device, error = %crate::server::chain(&error), "a Cloud's recovery after its runtime exited failed");
        }
    }

    /// The running device's machine; none when it is stopped.
    pub fn running(&self) -> Option<SandboxMachine> {
        self.process.as_ref().map(|process| process.machine)
    }

    /// What the next manager takes over of the running device.
    pub fn leave(mut self) -> Option<LeftSimple> {
        let process = self.process.take()?;
        Some(LeftSimple {
            device: self.device.clone(),
            pid: process.pid,
            started: process.started,
        })
    }

    async fn record(&self) -> Result<Option<TreeRecord>, OpError> {
        let path = self.tree().record();
        Ok(blocking::run(move |off| read_json(off, &path)).await?)
    }

    async fn sync(&self) -> Result<(), OpError> {
        let root = self.tree().root;
        blocking::run(move |_| -> io::Result<()> {
            let directory = fs_err::File::open(&root)?;
            rustix::fs::syncfs(directory.file()).map_err(io::Error::from)
        })
        .await?;
        Ok(())
    }

    /// Makes the device's tree: its system a copy of `base`, its home the
    /// base's `/etc/skel`, and a user of its own under root.
    async fn initialize(&self, base: &BaseVersion) -> Result<TreeRecord, OpError> {
        let user = match self.simple().users {
            true => Some(self.simple().take_user()?),
            false => None,
        };
        let rootfs = self.core.bases().join(base).join("rootfs");
        let tree = self.tree();
        let record = TreeRecord {
            base_version: base.clone(),
            reset_id: None,
            user,
        };
        let written = record.clone();
        blocking::run(move |off| -> io::Result<()> {
            remove_tree(off, &tree.root)?;
            fs_err::create_dir(&tree.root)?;
            // Root's, so the device's user cannot change the manager's
            // records in it; the device's user, by its group, passes through
            // to the directories it owns, and no other device's user does.
            let mode = match user {
                Some(user) => {
                    fs_err::os::unix::fs::chown(&tree.root, None, Some(user))?;
                    0o710
                }
                None => 0o700,
            };
            fs_err::set_permissions(&tree.root, std::fs::Permissions::from_mode(mode))?;
            tree::copy(off, &rootfs, &tree.system(), user)?;
            empty_directory(off, &tree.home(), user, 0o755)?;
            tree::copy(off, &rootfs.join("etc/skel"), &tree.home().join("demi"), user)?;
            for directory in [tree.system(), tree.home()] {
                fs_err::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))?;
            }
            sync(off, &tree.root)?;
            // Last, so a tree without a record is one recovery may remove.
            write_json(off, &tree.record(), &written)?;
            sync(off, &tree.root)
        })
        .await?;
        Ok(record)
    }

    /// Starts proot on the device's tree, made first from the configured base
    /// when there is none.
    async fn wake(&mut self, boot: ManagedBoot, machine: SandboxMachine) -> Result<(), OpError> {
        if self.process.is_some() {
            return Ok(());
        }
        let record = match self.record().await? {
            Some(record) => record,
            None => self.initialize(&self.base.clone()).await?,
        };
        self.simple().fence(&self.device, record.user).await?;
        // Demi's own programs come from the configured base, whatever base
        // the device is pinned to.
        let configured = self.core.bases().join(&self.base);
        let runner_release = base::runner_release(&configured).await?;
        let programs = configured.join("rootfs").join(PROGRAMS_PATH.trim_start_matches('/'));
        let boot = ManagedBoot {
            abstract_socket: Some(abstract_socket(&self.core.config.runner_socket)),
            ..boot
        };
        let tree = self.tree();
        let (dns, user) = (self.core.config.dns.clone(), record.user);
        blocking::run({
            let tree = tree.clone();
            move |off| -> io::Result<()> {
                let opt = within(&tree, PROGRAMS_PATH, user)?;
                remove_tree(off, &opt)?;
                tree::copy(off, &programs, &opt, user)?;
                // What a gVisor boot makes anew in memory.
                for (path, mode) in [("/run", 0o755), ("/tmp", 0o1777)] {
                    empty_directory(off, &within(&tree, path, user)?, user, mode)?;
                }
                empty_directory(off, &tree.shm(), user, 0o1777)?;
                empty_directory(off, &tree.scratch(), user, 0o700)?;
                place(&within(&tree, BOOT_RECORD, user)?, &serde_json::to_vec(&boot)?, user, 0o400)?;
                // The host's own resolver file, unless the configuration
                // names others: a Cloud on the host's network reaches what
                // the host does, a stub on loopback included.
                let resolver = match &dns {
                    Some(dns) => files::resolver(dns),
                    None => match fs_err::read_to_string(HOST_RESOLVER) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
                        read => read?,
                    },
                };
                place(&within(&tree, "/etc/resolv.conf", user)?, resolver.as_bytes(), user, 0o444)?;
                // The host's name too, which a Cloud on the host's network
                // has, so `sudo` resolves it.
                let host = fs_err::read_to_string("/proc/sys/kernel/hostname")?;
                place(&within(&tree, "/etc/hosts", user)?, files::hosts(&[host.trim()]).as_bytes(), user, 0o444)?;
                sync(off, &tree.root)
            }
        })
        .await?;
        let process = self.start(&tree, user, &runner_release, machine).await?;
        let record = ProcessRecord {
            pid: process.pid,
            started: process.started,
            boot: blocking::run(|_| process::boot()).await?,
            machine,
        };
        let path = tree.process();
        self.process = Some(process);
        blocking::run(move |off| write_json(off, &path, &record)).await?;
        Ok(())
    }

    /// Starts proot, in a session of its own, as the device's user when it
    /// has one.
    async fn start(
        &self,
        tree: &Tree,
        user: Option<u32>,
        runner_release: &str,
        machine: SandboxMachine,
    ) -> Result<Process, OpError> {
        let path = tree.log();
        // Each boot's log replaces the last, so the file never outgrows one.
        let log = blocking::run(move |_| fs_err::OpenOptions::new().create(true).write(true).truncate(true).open(path)).await?;
        let mut command = self.core.tools.command(Tool::Proot);
        command
            .arg("--kill-on-exit")
            .arg("-0")
            .arg("-r")
            .arg(tree.system())
            .arg("-b")
            .arg(format!("{}:/home", tree.home().display()))
            .args(["-b", "/dev", "-b", "/proc", "-b", "/sys"])
            .arg("-b")
            .arg(format!("{}:/dev/shm", tree.shm().display()))
            // tini is not the first process here, as it is on gVisor, so it
            // registers as a subreaper (`-s`) to reap what the Cloud's
            // programs leave.
            .args(["-w", "/home/demi", INIT_PATH, "-s", "--", RUNNER_PATH, "run", "--managed-boot", BOOT_RECORD])
            .env_clear()
            .env("HOME", "/home/demi")
            .env("USER", "root")
            .env("LOGNAME", "root")
            .env("LANG", "C.UTF-8")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin")
            .env(RELEASE_ENV, runner_release)
            .env("TMPDIR", TEMPORARY_DIRECTORY)
            .env("PROOT_TMP_DIR", tree.scratch())
            .stdout(log.file().try_clone()?)
            .stderr(log.into_file())
            // proot outlives the manager that leaves it to the next one.
            .kill_on_drop(false);
        if let Some(user) = user {
            command.uid(user).gid(user);
        }
        // SAFETY: setsid is async-signal-safe and touches no memory.
        unsafe {
            command.pre_exec(|| rustix::process::setsid().map(drop).map_err(io::Error::from));
        }
        let mut child = command.spawn()?;
        let pid = child.id().ok_or_else(|| io::Error::other("proot exited as it started"))? as i32;
        let started = process::started(pid).ok_or_else(|| io::Error::other("proot exited as it started"))?;
        let waiter = tokio::task::spawn_local(async move { child.wait().await.map(drop) });
        tracing::info!(event = "cloud_runtime.started", host_id = %self.device, pid, "a Cloud started on Simple Isolation");
        Ok(Process {
            pid,
            started,
            machine,
            waiter: Some(waiter),
        })
    }

    /// Ends the device's processes: its first process, so proot ends the
    /// rest, then whatever remains, and syncs its tree.
    async fn stop(&mut self) -> Result<(), OpError> {
        let record = self.record().await?;
        let user = record.as_ref().and_then(|record| record.user);
        if let Some(mut process) = self.process.take() {
            let (proot, started) = (process.pid, process.started);
            // A proot that exited already may have left its id to another
            // process, which nothing here signals.
            let first = blocking::run(move |_| -> io::Result<Option<Option<i32>>> {
                if process::started(proot) != Some(started) {
                    return Ok(None);
                }
                Ok(Some(process::pids()?.into_iter().find(|pid| process::parent(*pid) == Some(proot))))
            })
            .await?;
            if let Some(first) = first {
                let target = first.unwrap_or(proot);
                signal(target, rustix::process::Signal::TERM);
                if !ended(&mut process.waiter, STOP_GRACE).await {
                    signal(target, rustix::process::Signal::KILL);
                    if !ended(&mut process.waiter, KILLED_GRACE).await {
                        signal(proot, rustix::process::Signal::KILL);
                        ended(&mut process.waiter, KILLED_GRACE).await;
                    }
                }
            }
            process.waiter = None;
        }
        if record.is_none() {
            return Ok(());
        }
        self.simple().fence(&self.device, user).await?;
        let path = self.tree().process();
        blocking::run(move |_| match fs_err::remove_file(&path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        })
        .await?;
        self.sync().await
    }

    /// Replaces the device's system with a copy of `base`, keeping its home,
    /// once per `operation`; it does not boot.
    async fn reset(&mut self, operation: &str, base: &BaseVersion) -> Result<(), OpError> {
        let rootfs = self.core.bases().join(base).join("rootfs");
        let manifest = self.core.bases().join(base).join("manifest.json");
        match blocking::run(move |_| fs_err::metadata(manifest)).await {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(OpError::MissingBase(base.clone())),
            Err(error) => return Err(error.into()),
        }
        self.stop().await?;
        let record = match self.record().await? {
            Some(record) => record,
            None => self.initialize(base).await?,
        };
        if record.reset_id.as_deref() == Some(operation) {
            return Ok(());
        }
        let reset = TreeRecord {
            base_version: base.clone(),
            reset_id: Some(operation.to_owned()),
            user: record.user,
        };
        let tree = self.tree();
        blocking::run(move |off| -> io::Result<()> {
            let id = uuid::Uuid::new_v4();
            // Staged under names recovery removes, so a reset that stops
            // midway leaves the old system or the new one.
            let staged = tree.root.join(format!(".system-{id}"));
            let old = tree.root.join(format!(".old-{id}"));
            tree::copy(off, &rootfs, &staged, reset.user)?;
            fs_err::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
            fs_err::rename(tree.system(), &old)?;
            fs_err::rename(&staged, tree.system())?;
            write_json(off, &tree.record(), &reset)?;
            sync(off, &tree.root)?;
            remove_tree(off, &old)
        })
        .await?;
        Ok(())
    }
}
