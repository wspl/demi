//! The runner's console: its standard output and error, where it writes the lines
//! for the person at the terminal, among them the pairing code and result the
//! installers read (`demi_runner_protocol::console`). A runner started in the
//! background writes it to its installation's log itself (`runner.log`, rotated at
//! 10 MiB to `runner.log.1`) rather than the installer redirecting it, so what a
//! runner printed before a crash survives and a person who closed the terminal
//! early finds the lines there. The log holds pairing codes, so it is private to its
//! owner, and the Host log never holds one.
//!
//! A runner started with `run --log` leaves its terminal first of all
//! ([`leave_terminal`]): a session of its own has no controlling terminal, so
//! Ctrl-C at the installer or a closed SSH connection never reaches the runner or
//! its jobs. A process that leads its process group cannot make a session; it asks
//! whether it still has a terminal (opening `/dev/tty` fails with `ENXIO` only when
//! it has none) and if so starts itself again in a session of its own, made between
//! fork and exec so that nothing of the installation is open yet, and exits 0.
//!
//! The installer leaves this to the runner and never turns on the shell's job
//! control (`set -m`): with `curl … | sh`, a shell turning it off again hands the
//! terminal back to curl's already ended process group, and dash fails with exit
//! code 2 after the runner has started.

use std::{
    fmt::Display,
    fs::File,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

/// The log of a runner in the background, in its installation's directory.
pub const LOG: &str = "runner.log";
/// The log before it.
pub const PREVIOUS_LOG: &str = "runner.log.1";
/// The size past which the log starts again.
const LIMIT: u64 = 10 * 1024 * 1024;

/// The log the console goes to, once [`to_log`] made it so.
static CURRENT: Mutex<Option<Log>> = Mutex::new(None);

/// From now on, writes this process's standard output and standard error,
/// and so whatever else writes there, such as a panic's report, to the log
/// in `directory`.
pub fn to_log(directory: &Path) -> io::Result<()> {
    let log = Log::open(directory)?;
    *CURRENT.lock().unwrap_or_else(PoisonError::into_inner) = Some(log);
    Ok(())
}

/// How a runner in the background left the terminal it was started from.
#[cfg(unix)]
pub enum Departure {
    /// This process has no controlling terminal.
    Left,
    /// The runner goes on in another process, in a session of its own, and
    /// this one ends.
    Restarted,
}

/// Makes this process a runner in the background of its own, with no
/// controlling terminal, so that neither an interrupt at the terminal it
/// was started from, nor that terminal's closing, as when an SSH connection
/// ends, reaches it or the jobs it starts. It makes a new session, which has none. A process
/// group's leader cannot: one that `run start` started in a session of its
/// own, one that replaced itself with its update, or one a shell with job
/// control started, at a terminal's foreground or not. When such a process
/// still has a terminal, it starts the runner again in a session of its own
/// and ends.
///
/// Only once it has no terminal do the signals its starter ignored to keep
/// the terminal from it, `HUP` under `nohup` and `INT` and `QUIT` for a
/// background command of a shell without job control, take their default
/// action again: its jobs would inherit the ignoring, and a job stopped
/// with one of them would go on.
#[cfg(unix)]
pub fn leave_terminal() -> io::Result<Departure> {
    match rustix::process::setsid() {
        // A new session has no controlling terminal.
        Ok(_) => {}
        Err(rustix::io::Errno::PERM) => {
            if has_terminal()? {
                start_apart()?;
                line(runner_line(
                    "the runner goes on in a session of its own, apart from this terminal",
                ));
                return Ok(Departure::Restarted);
            }
        }
        Err(error) => return Err(error.into()),
    }
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
        // SAFETY: the default action runs no code in this process, and it
        // replaces no handler: the runner installs its own, for `INT` and
        // `TERM`, only later. rustix offers no safe call that sets a
        // signal's action, nor does the standard library.
        if unsafe { libc::signal(signal, libc::SIG_DFL) } == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(Departure::Left)
}

/// Whether this process has a controlling terminal: `/dev/tty` names it,
/// and opening it fails with `ENXIO` only when there is none.
#[cfg(unix)]
fn has_terminal() -> io::Result<bool> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOCTTY)
        .open("/dev/tty");
    match opened {
        Ok(_) => Ok(true),
        Err(error) => match error.raw_os_error() {
            Some(libc::ENXIO) => Ok(false),
            // The terminal is there, only not this account's to open.
            Some(libc::EACCES) => Ok(true),
            _ => Err(io::Error::new(
                error.kind(),
                format!("cannot tell whether the runner left its terminal: /dev/tty: {error}"),
            )),
        },
    }
}

/// Starts this program again with this process's arguments and
/// environment, as the leader of a new session, which has no controlling
/// terminal. Nothing of the installation is open yet, neither its lock nor
/// its log, so the new runner takes them as this one would have.
#[cfg(unix)]
fn start_apart() -> io::Result<()> {
    use std::os::unix::process::CommandExt as _;
    let mut arguments = std::env::args_os();
    let mut command = std::process::Command::new(std::env::current_exe()?);
    if let Some(name) = arguments.next() {
        command.arg0(name);
    }
    command.args(arguments).stdin(std::process::Stdio::null());
    // SAFETY: the hook runs in the child between fork and exec and makes
    // only a `setsid` call, which is async-signal-safe, and allocates
    // nothing. The child is a new process, never its group's leader, so
    // the call makes its session.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()?;
            Ok(())
        });
    }
    // The new runner outlives this process, which never waits for it.
    command.spawn()?;
    Ok(())
}

/// The console line of a message of the runner's own, such as one its
/// host log's console layer writes.
pub fn runner_line(message: impl Display) -> String {
    format!("demi-runner: {message}")
}

/// Writes `line` to standard error as one write, so that no other line
/// cuts into it; a log that has passed its size starts again first.
pub fn line(line: impl Display) {
    let mut current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    let mut text = format!("{line}\n");
    if let Some(log) = current.as_mut()
        && let Err(error) = log.start_again_if_full()
    {
        // The line still goes to the log that has passed its size.
        text = format!("demi-runner: the log could not start again: {error}\n{text}");
    }
    // A console that nobody reads any more, such as a closed terminal's, is
    // no reason for the runner to stop.
    let _ = io::stderr().lock().write_all(text.as_bytes());
}

/// The log the console goes to.
struct Log {
    directory: PathBuf,
    /// The open log, which standard output and standard error write to.
    file: File,
}

impl Log {
    /// Opens the log in `directory` to append to, after moving it to
    /// [`PREVIOUS_LOG`] when it has passed its size, and makes it the
    /// console.
    fn open(directory: &Path) -> io::Result<Self> {
        let path = directory.join(LOG);
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.len() > LIMIT => {
                std::fs::rename(&path, directory.join(PREVIOUS_LOG))?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut options = std::fs::OpenOptions::new();
        options.append(true).create(true);
        // The log holds pairing codes: its owner's alone.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&path)?;
        redirect(&file)?;
        Ok(Self {
            directory: directory.to_owned(),
            file,
        })
    }

    /// Starts the log again when it has passed its size.
    fn start_again_if_full(&mut self) -> io::Result<()> {
        if self.file.metadata()?.len() > LIMIT {
            // The log before is closed once the console writes to the new one.
            *self = Self::open(&self.directory)?;
        }
        Ok(())
    }
}

/// Makes `file` this process's standard output and standard error.
#[cfg(unix)]
fn redirect(file: &File) -> io::Result<()> {
    rustix::stdio::dup2_stdout(file)?;
    rustix::stdio::dup2_stderr(file)?;
    Ok(())
}

/// Makes `file` this process's standard output and standard error, which
/// the standard library looks up at each write.
#[cfg(windows)]
fn redirect(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    for standard in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: the handle is the open log's, which `Log` keeps open for as
        // long as it is the standard one.
        if unsafe { SetStdHandle(standard, file.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
