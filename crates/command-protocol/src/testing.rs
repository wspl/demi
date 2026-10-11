//! Test support (`crates-and-packages.md` § command-protocol) that tests at
//! both ends of the command wire name: the programs a test finds beside
//! itself and the operations of the runner's native fixture service.

use std::path::{Path, PathBuf};

/// The operations of the runner's native fixture service
/// (`crates/runner/tests/fixtures/service.rs`), which the runner's and
/// backend-remote-host's tests name in its descriptors.
pub const FIXTURE_OPERATIONS: [&str; 14] = [
    "where",
    "echo",
    "first",
    "spin",
    "result",
    "retain",
    "stall_release",
    "held",
    "crash",
    "stalled",
    "proceed",
    "number",
    "medium",
    "install",
];

/// The build that places every program of the workspace in the target
/// directory, in the one selection the tests use (`AGENTS.md` § Building
/// and Testing).
pub const BUILD_PROGRAMS: &str =
    "cargo build --workspace --all-targets --features demi-runner/test-fixtures";

/// A program Cargo built into the target directory this test runs from,
/// such as a workspace crate's executable: Cargo places the programs in the
/// profile's directory, the nearest one above the test that holds Cargo's
/// lock file, and the test itself somewhere below it. Panics when the
/// program is missing or older than its sources ([`current_program`]).
pub fn built_program(name: &str) -> PathBuf {
    let executable = std::env::current_exe().expect("the test knows its executable");
    let directory = executable
        .ancestors()
        .find(|directory| directory.join(".cargo-lock").is_file())
        .expect("a test runs below its profile's directory");
    current_program(directory, name)
}

/// The program `name` in `directory`, built from the sources as they are
/// now. Cargo builds with a test only the programs of the test's own
/// package, so after `cargo test --test backend` the `demi-runner` beside
/// the test is whatever an earlier build left: a test that started it would
/// fail as a runner of another wire, or pass on its old behavior. Cargo
/// writes beside each program it places the files it was built from, its
/// dep-info `<name>.d`; a program older than one of them is stale, as Cargo
/// itself judges it, and this panics at once with the build that renews it.
/// Neither Cargo's `CARGO_BIN_EXE_<name>`, which only covers the test's own
/// package, nor its artifact dependencies, unstable (`-Z bindeps`), can make
/// Cargo build another package's program for a test.
pub fn current_program(directory: &Path, name: &str) -> PathBuf {
    let program = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if let Some(problem) = staleness(&program) {
        panic!("{problem}; build the workspace's programs first: {BUILD_PROGRAMS}");
    }
    program
}

/// Why `program` does not match the sources it is built from, if it does
/// not.
fn staleness(program: &Path) -> Option<String> {
    let Ok(built) = std::fs::metadata(program).and_then(|metadata| metadata.modified()) else {
        return Some(format!("{} is not built", program.display()));
    };
    // Cargo writes the dep-info beside every program it places; a program
    // without one was put there by hand, and whoever did so answers for it.
    let info = std::fs::read_to_string(program.with_extension("d")).ok()?;
    // The first rule names the program and, after `: `, its sources, each
    // space in a path escaped with a backslash.
    let (_, sources) = info.lines().next()?.split_once(": ")?;
    sources_of(sources).into_iter().find_map(|source| {
        let changed = std::fs::metadata(&source).and_then(|metadata| metadata.modified());
        match changed {
            Ok(changed) if changed <= built => None,
            Ok(_) => Some(format!(
                "{} is older than {}, which it is built from",
                program.display(),
                source.display()
            )),
            Err(_) => Some(format!(
                "{} is built from {}, which is gone",
                program.display(),
                source.display()
            )),
        }
    })
}

/// The paths of a dep-info rule's prerequisites.
fn sources_of(rule: &str) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut path = String::new();
    let mut characters = rule.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some(' ') => path.push(' '),
                // A Windows path's separator.
                Some(next) => {
                    path.push('\\');
                    path.push(next);
                }
                None => path.push('\\'),
            },
            ' ' => {
                if !path.is_empty() {
                    sources.push(PathBuf::from(std::mem::take(&mut path)));
                }
            }
            _ => path.push(character),
        }
    }
    if !path.is_empty() {
        sources.push(PathBuf::from(path));
    }
    sources
}
