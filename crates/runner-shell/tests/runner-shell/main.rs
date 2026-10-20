//! The integration tests of the runner's shell and its standard utilities,
//! in one process.

mod declared;
mod edit_tracking;
mod gnu;
mod jobs;
mod shell;
mod tasks;
mod utilities;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here share one shell runtime and run together, as a
// runner's jobs do, and a tighter bound fails them on a loaded machine.

/// Whether `actual`, what a utility or builtin printed on standard error,
/// says what `expected` says: the same text, or on Windows, which words the
/// system's own errors, the same lines with Windows' reason where a line
/// ends in the reason of a Unix error after GNU's prefix, as
/// `diff: nope: The system cannot find the file specified.` stands for
/// `diff: nope: No such file or directory`. The reason never shows Rust's
/// `(os error N)`.
fn same_errors(expected: &str, actual: &str) -> bool {
    const UNIX_REASONS: &[&str] = &[
        "No such file or directory",
        "Permission denied",
        "File name too long",
        "Not a directory",
        "Is a directory",
    ];
    if expected == actual {
        return true;
    }
    let lines = |text: &str| text.lines().map(str::to_owned).collect::<Vec<_>>();
    let (expected_lines, actual_lines) = (lines(expected), lines(actual));
    cfg!(windows)
        && expected.ends_with('\n') == actual.ends_with('\n')
        && expected_lines.len() == actual_lines.len()
        && expected_lines.iter().zip(&actual_lines).all(|(expected, actual)| {
            expected == actual
                || expected.rsplit_once(": ").is_some_and(|(prefix, reason)| {
                    UNIX_REASONS.contains(&reason)
                        && actual
                            .strip_prefix(prefix)
                            .and_then(|rest| rest.strip_prefix(": "))
                            .is_some_and(|reason| !reason.is_empty() && !reason.contains("os error"))
                })
        })
}

/// A job's environment that names `root` as its home, so the job reads no
/// login profile of the machine's user (`crates/runner-shell/src/lib.rs`).
fn home(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::from([("HOME".to_owned(), root.to_string_lossy().into_owned())])
}
