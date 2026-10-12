//! What every program's command line shares (`crates-and-packages.md` §
//! `shared-cli`): a variable under the program's prefix that none of its
//! settings reads stops startup, so a misspelt setting is never ignored. The
//! backend and the machine manager read one configuration file
//! (`backend.md` § Configuration): the manager checks the names under its
//! own prefix, and the backend every other `DEMI_*` name. And the panic
//! hook every program installs at start (`builds-and-releases.md` § Build
//! profiles). And, on a musl target, the memory allocator of the programs
//! that run on a Host (`builds-and-releases.md` § Memory allocator).

use std::{ffi::OsString, io::Write as _};

/// musl's own allocator unmaps and maps memory around small allocations and
/// serializes threads on one lock, which costs most inside gVisor, where
/// each system call is expensive. The runner and the command programs turn
/// on `host-allocator`; the server's programs keep musl's allocator, which
/// holds less memory. A program uses this allocator only if it links this
/// crate's object, which calling `install_panic_hook` ensures.
#[cfg(all(target_env = "musl", feature = "host-allocator"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The prefix of the machine manager's own settings, which the backend
/// leaves to the manager's check.
pub const MANAGED_PREFIX: &str = "DEMI_MANAGED_";

/// The first variable of `vars` whose name starts with `prefix` and that no
/// argument of `command` reads; a name that is not Unicode is no setting's.
pub fn unknown_variable(
    command: &clap::Command,
    prefix: &str,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Option<String> {
    let known: Vec<_> = command
        .get_arguments()
        .filter_map(clap::Arg::get_env)
        .collect();
    vars.into_iter().find_map(|(name, _)| {
        let name = name.into_string().ok()?;
        let unknown =
            name.starts_with(prefix) && !known.iter().any(|known| *known == name.as_str());
        unknown.then_some(name)
    })
}

/// Makes every panic of this process write its message, its location and a
/// backtrace to standard error, which the program's log keeps, before
/// anything else happens to it: a panic that ends the process, such as one
/// inside a destructor, which Rust turns into an abort, leaves that
/// evidence behind; a contained one leaves it beside what contained it.
/// The backtrace is taken whatever `RUST_BACKTRACE` says, since a program
/// in the background is started without it.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let backtrace = std::backtrace::Backtrace::force_capture();
        // One write, so that another thread's line does not cut into it.
        let report = format!("thread '{name}' {info}\nstack backtrace:\n{backtrace}\n");
        // A standard error that cannot be written leaves nowhere to say so.
        let _ = std::io::stderr().lock().write_all(report.as_bytes());
    }));
}
