//! Test support (`crates-and-packages.md` § command-protocol) that tests at
//! both ends of the command wire name: the programs a test finds beside
//! itself and the operations of the runner's native fixture service.

use std::path::{Path, PathBuf};

/// The operations of the runner's native fixture service
/// (`crates/runner/tests/fixtures/service.rs`), which the runner's and
/// host-remote's tests name in its descriptors.
pub const FIXTURE_OPERATIONS: [&str; 12] = [
    "where", "echo", "first", "spin", "result", "retain", "stall_release", "held", "crash", "stalled",
    "proceed", "number",
];

/// A program Cargo built into the target directory this test runs from,
/// such as a workspace crate's executable.
pub fn built_program(name: &str) -> PathBuf {
    let executable = std::env::current_exe().expect("the test knows its executable");
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .expect("a test runs from the target directory's deps");
    directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}
