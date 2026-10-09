//! The integration tests of the runner's shell and its standard utilities,
//! in one process.

mod declared;
mod edit_tracking;
mod gnu;
mod jobs;
mod shell;
mod utilities;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here share one shell runtime and run together, as a
// runner's jobs do, and a tighter bound fails them on a loaded machine.

/// A job's environment that names `root` as its home, so the job reads no
/// login profile of the machine's user (`runner.md` § Shell jobs).
fn home(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::from([("HOME".to_owned(), root.to_string_lossy().into_owned())])
}
