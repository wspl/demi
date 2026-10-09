//! The runner's console (`runner.md` § Installation, pairing and removal):
//! its standard output and standard error, where it writes the lines for
//! the person at the terminal, among them the ones the installers read
//! (`demi_runner_protocol::console`). A runner started in the background
//! writes its console to its installation's log itself: it appends to
//! `runner.log`, and once the file has passed 10 MiB it renames it
//! `runner.log.1`, replacing the one before, and starts a new one. The
//! installer and `run start` read the log from where it ended when they
//! started the runner.

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
