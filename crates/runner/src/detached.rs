//! Starting a process on Windows that outlives the session its starter
//! runs in (`crates/runner/src/console.rs`: closing the
//! terminal does not stop the runner).

use std::io;

/// A hidden console of its own: Windows PowerShell started without any
/// console, as DETACHED_PROCESS starts it, ends before it runs its command.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// No part in the starter's Ctrl+C.
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// Out of the job the starter's session shares: OpenSSH ends every process
/// of that job when the session ends, so a runner that `run start` started
/// over SSH, or the remover an `uninstall` over SSH started, ended with it.
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// Starts a process with `start`, given the creation flags: out of the
/// starter's job, or in it when the job allows no breakaway and refuses the
/// start, as in a PowerShell window, whose job nothing closes.
pub fn start<T>(mut start: impl FnMut(u32) -> io::Result<T>) -> io::Result<T> {
    let flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
    match start(flags | CREATE_BREAKAWAY_FROM_JOB) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => start(flags),
        started => started,
    }
}
