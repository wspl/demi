//! The host's processes as `/proc` shows them: what the manager needs to stop
//! a Simple Isolation Cloud whole and to tell a process it took over from one
//! that reused its id.

use std::{io, path::Path};

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
