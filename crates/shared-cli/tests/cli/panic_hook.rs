//! The panic hook every program installs at start (`builds-and-releases.md`
//! § Build profiles).

use std::process::Command;

/// Set when this test binary, started again by the test, is the program
/// whose panic aborts.
const ABORTING: &str = "DEMI_TEST_PANIC_ABORTS";

/// The test's name as the harness filters it.
const NAME: &str =
    "panic_hook::a_panic_that_aborts_leaves_its_message_location_and_backtrace_on_standard_error";

/// A value whose destructor panics.
struct Exploding;

impl Drop for Exploding {
    fn drop(&mut self) {
        panic!("the destructor panicked too");
    }
}

/// Panics holding an [`Exploding`], whose destructor then panics too, which
/// Rust turns into an abort.
#[inline(never)]
fn panics_holding_an_exploding_value() {
    let _exploding = Exploding;
    panic!("the first panic");
}

/// A program that panics in a destructor while it unwinds is aborted, and
/// it leaves on standard error the first panic's message, its location and
/// a backtrace through the function that panicked, before what the
/// destructor's panic says, and without `RUST_BACKTRACE`, which a program in
/// the background is started without. Rust's own hook prints a backtrace
/// for the destructor's panic only, which shows where the abort happened
/// rather than what caused it.
// Some milliseconds: the test starts this binary a second time.
#[test]
fn a_panic_that_aborts_leaves_its_message_location_and_backtrace_on_standard_error() {
    if std::env::var_os(ABORTING).is_some() {
        demi_shared_cli::install_panic_hook();
        panics_holding_an_exploding_value();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(ABORTING, "1")
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stderr);
    // An abort ends the process by a signal, so it has no exit code.
    #[cfg(unix)]
    assert_eq!(output.status.code(), None, "{said}");
    #[cfg(not(unix))]
    assert!(!output.status.success(), "{said}");
    let (first, _) = said
        .split_once("the destructor panicked too")
        .unwrap_or_else(|| panic!("the destructor's panic is reported: {said}"));
    assert!(first.contains("the first panic"), "{said}");
    assert!(first.contains(&format!("panicked at {}:", file!())), "{said}");
    assert!(first.contains("panics_holding_an_exploding_value"), "{said}");
}
