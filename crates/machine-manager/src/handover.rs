//! Leaving the running sandboxes to the next manager, and taking them over:
//! how a Cloud keeps its commands through an upgrade.
//!
//! An upgrade to a release that keeps the manager's state format writes the
//! leave request ([`LEAVE_REQUEST`]) before it stops the manager. The
//! manager that stops then saves no sandbox: it waits for its operations,
//! records each sandbox that runs ([`LEFT_RECORD`]) and exits with its saved
//! mount namespace still pinned, where the sandboxes' storage and network
//! namespaces are mounted. The unit's `KillMode=process` leaves the
//! sandboxes' processes to run.
//!
//! ```text
//! upgrade      old manager                     new manager
//!   leave ----> on SIGTERM: wait, record, exit
//!   start ----------------------------------->  enter the saved namespace,
//!                                               end what no record owns,
//!                                               take each recorded one over
//! ```
//!
//! The next manager finds the record before anything else and, when the
//! saved namespace holds its own state directory, enters it instead of
//! making a new one ([`enter_saved_namespace`]), and takes over each recorded
//! sandbox that `runsc` says runs. One that ended meanwhile is fenced and
//! saved, as after a crash. The stop-post recovery leaves alone the namespace
//! of a manager that left its sandboxes.
//!
//! systemd's stop no longer ends the unit's leftover processes, so every
//! start and stop-post recovery ends them first ([`sweep`]) and waits until
//! they exited, before anything writes a disk image: every process of the
//! unit's cgroup but the manager's own and those of the sandboxes it takes
//! over, such as an `e2fsck` of a manager killed while it repaired an image,
//! an earlier manager's `runsc wait`, or a sandbox that `runsc` was killed
//! while making, which no record names.

use std::{
    io,
    os::fd::AsFd as _,
    path::{Path, PathBuf},
};

use demi_machine_manager_protocol::{LEAVE_REQUEST, LEFT_RECORD, LeftRecord, LeftSandbox};
use rustix::thread::LinkNameSpaceType;

use crate::{
    blocking,
    manager::Core,
    sandbox::{Sandbox, files::SandboxRecord},
    storage::{durable::write_json, working::WorkingPair},
};

/// A sandbox taken over, with what [`sweep`] keeps of its processes.
pub struct Taken {
    pub sandbox: Sandbox,
    pub owner: Owner,
}

/// What tells a sandbox's processes on the host: its bundle, which its
/// Sentry's and Gofer's arguments name, and its Sentry's process id, which
/// its `runsc-fd-parking` holds a descriptor of.
pub struct Owner {
    pub bundle: PathBuf,
    pub sentry: i32,
}

/// Takes over the sandbox `left` names, which a manager before this one left
/// running: none when it has ended since, or when the device's working pair
/// records another boot, either of which recovery then fences.
pub async fn take_over(core: &Core, left: &LeftSandbox) -> io::Result<Option<Taken>> {
    let path = WorkingPair::new(&core.config.working(), &left.device).sandbox_record();
    let record = blocking::run(move |_| -> io::Result<Option<SandboxRecord>> {
        match fs_err::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    })
    .await?;
    let Some(record) = record.filter(|record| record.id.as_str() == left.id && record.slot == left.slot) else {
        return Ok(None);
    };
    let Some(sentry) = core.runsc.sentry(&record.id).await.map_err(io::Error::other)? else {
        return Ok(None);
    };
    let lease = core.slots.reserve(record.slot).map_err(|_| io::Error::other("the sandbox's network slot is taken"))?;
    let sandbox = Sandbox::adopted(core, record, lease, left.programs.clone());
    let owner = Owner {
        bundle: sandbox.bundle().to_owned(),
        sentry,
    };
    Ok(Some(Taken { sandbox, owner }))
}

/// Whether an upgrade asked this manager to leave its sandboxes; the request
/// goes as it is taken.
pub async fn take_request() -> io::Result<bool> {
    blocking::run(|_| match fs_err::remove_file(LEAVE_REQUEST) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    })
    .await
}

/// Records the sandboxes this manager leaves running.
pub async fn write_left(record: LeftRecord) -> io::Result<()> {
    blocking::run(move |off| write_json(off, Path::new(LEFT_RECORD), &record)).await
}

/// The sandboxes a manager before this one left running, if it left any.
pub fn read_left() -> io::Result<Option<LeftRecord>> {
    match fs_err::read(LEFT_RECORD) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| io::Error::other(format!("{LEFT_RECORD}: {error}"))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The record goes once its sandboxes are taken over or fenced.
pub async fn remove_left() -> io::Result<()> {
    blocking::run(|_| match fs_err::remove_file(LEFT_RECORD) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    })
    .await
}

/// Moves this process into the mount namespace a manager that left its
/// sandboxes pinned at `handle`, which it opens from the host's namespace,
/// where the handle is mounted. Runs before the manager starts any thread,
/// as `setns(CLONE_NEWNS)` needs, so every thread works in it. When it
/// fails, the process is back in its own namespace, which the start then
/// copies as it does without a handover.
pub fn enter_saved_namespace(handle: &Path) -> io::Result<()> {
    let enter = |namespace: &fs_err::File| {
        rustix::thread::move_into_link_name_space(namespace.file().as_fd(), Some(LinkNameSpaceType::Mount))
    };
    let own = fs_err::File::open("/proc/thread-self/ns/mnt")?;
    let host = fs_err::File::open("/proc/1/ns/mnt")?;
    enter(&host)?;
    let entered = fs_err::File::open(handle).and_then(|saved| enter(&saved).map_err(io::Error::from));
    if let Err(error) = entered {
        enter(&own).map_err(|back| io::Error::other(format!("{error}; and the manager could not return to its own mount namespace: {back}")))?;
        return Err(error);
    }
    tracing::info!(event = "machine_manager.namespace_entered", "the manager entered the namespace of the manager before it");
    Ok(())
}

/// Ends every process of this manager's unit's cgroup but itself, what it
/// started, such as its `runsc wait` of each sandbox it took over, and the
/// processes of the sandboxes `keep` owns: their Sentry and Gofer, which name
/// the bundle, their descendants, and the `runsc-fd-parking` that holds a
/// descriptor of the Sentry's process. With the resource limits on, the
/// Sentry and Gofer run in the sandbox's own cgroup and only the fd-parking
/// process is in the unit's, so the Sentry's process id comes from `runsc`,
/// not from the unit. A manager outside a service unit of its own, as one
/// started by hand is, ends nothing: its cgroup holds other programs.
pub async fn sweep(keep: Vec<Owner>) -> io::Result<()> {
    blocking::run(move |_| sweep_blocking(&keep)).await
}

fn sweep_blocking(keep: &[Owner]) -> io::Result<()> {
    let own = fs_err::read_to_string("/proc/self/cgroup")?;
    let Some(path) = own.lines().find_map(|line| line.strip_prefix("0::")) else {
        return Ok(());
    };
    if std::env::var_os("INVOCATION_ID").is_none() || !path.ends_with(".service") {
        return Ok(());
    }
    let procs_path = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/')).join("cgroup.procs");
    let procs = fs_err::read_to_string(&procs_path)?;
    let pids: Vec<i32> = procs.lines().filter_map(|line| line.trim().parse().ok()).collect();
    let me = std::process::id() as i32;
    let parent = |pid: i32| -> Option<i32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command's name, in parentheses, may hold spaces.
        stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
    };
    let arguments = |pid: i32| -> String {
        std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
            .unwrap_or_default()
    };
    let mut roots: Vec<i32> = pids
        .iter()
        .copied()
        .filter(|&pid| {
            let arguments = arguments(pid);
            keep.iter().any(|owner| arguments.contains(&*owner.bundle.to_string_lossy()))
        })
        .collect();
    roots.push(me);
    let sentries: Vec<i32> = keep.iter().map(|owner| owner.sentry).collect();
    let kept = |pid: i32| -> bool {
        let mut current = pid;
        for _ in 0..64 {
            if roots.contains(&current) {
                return true;
            }
            match parent(current) {
                Some(next) if next > 1 => current = next,
                _ => break,
            }
        }
        // A sandbox's `runsc-fd-parking` holds its Sentry's process.
        std::fs::read_to_string(format!("/proc/{pid}/fdinfo/3"))
            .ok()
            .and_then(|info| info.lines().find_map(|line| line.strip_prefix("Pid:")).map(|pid| pid.trim().to_owned()))
            .and_then(|pid| pid.parse::<i32>().ok())
            .is_some_and(|holder| sentries.contains(&holder) || roots.contains(&holder))
    };
    let mut ended = Vec::new();
    for pid in pids {
        if kept(pid) {
            continue;
        }
        tracing::info!(event = "machine_manager.leftover_ended", pid, arguments = %arguments(pid), "a process no sandbox owns was left in the manager's unit");
        if let Some(pid) = rustix::process::Pid::from_raw(pid) {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            ended.push(pid.as_raw_nonzero().get());
        }
    }
    // A killed process still writes until it exits, and leaves the cgroup
    // only then. No limit: the recovery that follows must not write beside
    // it, however long it takes.
    let started = std::time::Instant::now();
    let mut told = false;
    loop {
        let procs = fs_err::read_to_string(&procs_path)?;
        let left: Vec<i32> = procs
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .filter(|pid| ended.contains(pid))
            .collect();
        if left.is_empty() {
            return Ok(());
        }
        if !told && started.elapsed() > std::time::Duration::from_secs(5) {
            tracing::warn!(event = "machine_manager.leftover_waited", pids = ?left, "the manager waits for processes it ended to exit");
            told = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
