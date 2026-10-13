//! The programs a test starts from the target directory are the ones its
//! sources build now: a test never runs a `demi-runner` an earlier build
//! left, which a test selection of another package does not rebuild.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, SystemTime};

use demi_command_protocol::testing::{BUILD_PROGRAMS, current_program};

/// Gives `path` the modification time `seconds` after the epoch.
fn touch(path: &Path, seconds: u64) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

/// What `current_program` panicked with, if it did.
fn refusal(directory: &Path) -> Option<String> {
    std::panic::catch_unwind(|| current_program(directory, "demi-runner"))
        .err()
        .map(|payload| *payload.downcast::<String>().unwrap())
}

#[test]
fn a_program_older_than_its_sources_stops_the_test_with_both_times_and_the_build_that_renews_it() {
    let directory = tempfile::tempdir().unwrap();
    let missing = refusal(directory.path()).expect("a missing program is refused");
    assert!(missing.contains("is not built") && missing.contains(BUILD_PROGRAMS), "{missing}");

    // As Cargo leaves a program and its dep-info, with a space in a
    // source's path escaped. Two sources changed after the build; the
    // report names the newer, which the dep-info lists last.
    let program = directory
        .path()
        .join(format!("demi-runner{}", std::env::consts::EXE_SUFFIX));
    let wire = directory.path().join("wire messages.rs");
    let main = directory.path().join("main.rs");
    let hello = directory.path().join("hello.rs");
    for path in [&program, &wire, &main, &hello] {
        File::create(path).unwrap();
    }
    std::fs::write(
        directory.path().join("demi-runner.d"),
        format!(
            "{}: {} {} {}\n",
            program.display(),
            main.display(),
            hello.display(),
            wire.display().to_string().replace(' ', "\\ ")
        ),
    )
    .unwrap();
    touch(&main, 100);
    touch(&program, 200);
    touch(&hello, 250);
    touch(&wire, 300);
    let stale = refusal(directory.path()).expect("a stale program is refused");
    let seen = format!(
        "{} (modified 1970-01-01T00:03:20Z) is older than {} (modified 1970-01-01T00:05:00Z)",
        program.display(),
        wire.display()
    );
    assert!(stale.contains(&seen) && stale.contains(BUILD_PROGRAMS), "{stale}");

    // Built again after the change.
    touch(&program, 400);
    assert_eq!(refusal(directory.path()), None);
    assert_eq!(current_program(directory.path(), "demi-runner"), program);
}
