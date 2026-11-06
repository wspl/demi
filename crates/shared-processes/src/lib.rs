//! The host's processes: what `/proc` shows of them, and the signals that
//! reach them, for the programs that run a server as root.
//!
//! A container may run root without `CAP_KILL`, and then the kernel refuses
//! root's signal to another user's process. It always lets a process signal
//! its own user's, so a child that becomes that user sends it instead:
//!
//! ```text
//! root -- kill(a process of alex) --> refused
//! root --fork--> child: setuid(alex) -- kill(the process) --> delivered
//! ```
//!
//! The child is waited for before a signal function returns. A caller that
//! also waits for any child does so on the thread it signals from, or it may
//! take that child's end.

#![cfg(target_os = "linux")]

use std::{collections::BTreeSet, io, path::Path};

use rustix::{
    io::Errno,
    process::{Pid, Signal, WaitOptions},
    thread::CapabilitySet,
};

/// Sends `signal` to `pid`, whichever user it runs as.
pub fn signal_process(pid: Pid, signal: Signal) -> io::Result<()> {
    match rustix::process::kill_process(pid, signal) {
        Err(Errno::PERM) => {
            let uid = user(pid.as_raw_nonzero().get()).ok_or_else(|| io::Error::from(Errno::SRCH))?;
            as_user(uid, || rustix::process::kill_process(pid, signal))
        }
        sent => Ok(sent?),
    }
}

/// Sends `signal` to every process of `group`, whichever users they run as.
pub fn signal_group(group: Pid, signal: Signal) -> io::Result<()> {
    let sent = rustix::process::kill_process_group(group, signal);
    if rustix::thread::capabilities(None)?.effective.contains(CapabilitySet::KILL) {
        return Ok(sent?);
    }
    // The kernel answers success when any one member took the signal, so a
    // refusal of the others is silent, and the group's first process may be
    // gone: its members say which users it holds.
    let me = rustix::process::geteuid().as_raw();
    let id = group.as_raw_nonzero().get();
    let users: BTreeSet<u32> = pids()?
        .into_iter()
        .filter(|pid| self::group(*pid) == Some(id))
        .filter_map(user)
        .filter(|uid| *uid != me)
        .collect();
    let mut result = sent.map_err(io::Error::from);
    for uid in users {
        let other = as_user(uid, || rustix::process::kill_process_group(group, signal));
        if result.is_err() {
            result = other;
        }
    }
    result
}

/// Runs `send` in a child that has become the user `uid`.
fn as_user(uid: u32, send: impl FnOnce() -> rustix::io::Result<()>) -> io::Result<()> {
    // SAFETY: the child only makes system calls and leaves with `_exit`, so
    // it needs no lock another thread held at the fork, and runs no exit
    // handler of this program.
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => {
            let sent = rustix::thread::set_thread_uid(rustix::process::Uid::from_raw(uid)).and_then(|()| send());
            let code = sent.map_or_else(|error| error.raw_os_error(), |()| 0);
            unsafe { libc::_exit(code) }
        }
        child => {
            let child = Pid::from_raw(child).ok_or_else(|| io::Error::other("no process id"))?;
            let ended = rustix::process::waitpid(Some(child), WaitOptions::empty())?;
            match ended.and_then(|(_, status)| status.exit_status()) {
                Some(0) => Ok(()),
                Some(code) => Err(io::Error::from_raw_os_error(code)),
                None => Err(io::Error::other("the signal's sender was killed")),
            }
        }
    }
}

/// Every process's id.
pub fn pids() -> io::Result<Vec<i32>> {
    let mut pids = Vec::new();
    for entry in fs_err::read_dir("/proc")? {
        if let Some(pid) = entry?.file_name().to_str().and_then(|name| name.parse().ok()) {
            pids.push(pid);
        }
    }
    Ok(pids)
}

/// The fields of `/proc/<pid>/stat` after the command's name, which is in
/// parentheses and may hold spaces: the state is the first. None for a
/// process that is gone.
fn stat(pid: i32) -> Option<Vec<String>> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    Some(stat.rsplit_once(')')?.1.split_whitespace().map(str::to_owned).collect())
}

/// The process's parent.
pub fn parent(pid: i32) -> Option<i32> {
    stat(pid)?.get(1)?.parse().ok()
}

/// The process's process group.
pub fn group(pid: i32) -> Option<i32> {
    stat(pid)?.get(2)?.parse().ok()
}

/// When the process started, in clock ticks since the host booted: with the
/// boot's id, what tells it from a later process with its id.
pub fn started(pid: i32) -> Option<u64> {
    stat(pid)?.get(19)?.parse().ok()
}

/// Whether the process has exited and waits only to be reaped.
pub fn zombie(pid: i32) -> bool {
    stat(pid).is_some_and(|fields| fields.first().is_some_and(|state| state == "Z"))
}

/// The process's real user.
pub fn user(pid: i32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Whether the program the process runs is a file under `directory`, one
/// removed since included, whose link ends in ` (deleted)`. One this process
/// may not inspect runs none.
pub fn runs_from(pid: i32, directory: &Path) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|program| program.starts_with(directory))
}

/// The id of the host's current boot.
pub fn boot() -> io::Result<String> {
    Ok(fs_err::read_to_string("/proc/sys/kernel/random/boot_id")?.trim().to_owned())
}
