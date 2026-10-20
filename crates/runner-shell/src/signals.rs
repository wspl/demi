//! Signals by the names and numbers a command line gives them, for `kill`
//! and `timeout` (`crates/runner-shell/src/process_builtins.rs`). Windows has
//! no signals, but the job's shell takes every Unix name and number there
//! too, numbered as on Linux, as uucore's table for Windows numbers them, so
//! a status of 128 plus a signal's number is the one a script tests on
//! Unix: `143` for `TERM`, `137` for `KILL`.

/// `TERM`, which `kill` and `timeout` send unless told otherwise.
#[cfg(unix)]
pub(crate) const TERM: i32 = libc::SIGTERM;
/// `KILL`, which nothing can catch.
#[cfg(unix)]
pub(crate) const KILL: i32 = libc::SIGKILL;
#[cfg(windows)]
pub(crate) const TERM: i32 = 15;
#[cfg(windows)]
pub(crate) const KILL: i32 = 9;

/// A signal's number by the name or number `spec` gives it, as bash reads
/// it: any case, with or without `SIG`; 0 and `EXIT` ask only whether the
/// target exists.
pub(crate) fn number(spec: &str) -> Option<i32> {
    #[cfg(unix)]
    return spec
        .parse::<brush_core::traps::TrapSignal>()
        .ok()
        .and_then(|signal| i32::try_from(signal).ok());
    #[cfg(windows)]
    return uucore::signals::signal_by_name_or_value(spec)
        .and_then(|number| i32::try_from(number).ok());
}

/// The name of signal `number`, without `SIG`, as `kill -l` and `timeout
/// -v` write it.
pub(crate) fn name(number: i32) -> Option<String> {
    #[cfg(unix)]
    return brush_core::traps::TrapSignal::try_from(number)
        .ok()
        .map(|signal| signal.as_str().trim_start_matches("SIG").to_owned());
    #[cfg(windows)]
    return usize::try_from(number)
        .ok()
        .and_then(uucore::signals::signal_name_by_value);
}
