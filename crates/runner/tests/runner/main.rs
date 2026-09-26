//! The runner's integration tests, in one process. Each test binary costs a
//! link and, when it is new, a first-launch check, so the tests live here.
//! Only the open-file test keeps a binary of its own (`tests/open_files.rs`):
//! it lowers the process's open-file limit and holds every descriptor left.

mod artifact_cache;
mod command_client;
mod connection;
mod dispatch;
mod edit_tracking;
mod fs;
mod git;
mod host;
mod load;
mod local;
mod pipes;
mod process;
mod registration;
mod services;
mod shell;
mod tasks;
mod utilities;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here share one shell runtime and run together, as a
// runner's jobs do, and a tighter bound fails them on a loaded machine.

/// A job's environment that names `root` as its home, so the job reads no
/// login profile of the machine's user (`runner.md` § Shell jobs).
fn home(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::from([("HOME".to_owned(), root.to_string_lossy().into_owned())])
}
