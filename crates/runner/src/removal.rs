//! Removing a paired device's runner (`runner.md` § Installation, pairing
//! and removal): the command that does it, which the runner tells the person
//! at its console once it is paired, and the removal of its installation's
//! directory, with everything in it: the device token, the releases, the
//! log, and the artifact cache unless `DEMI_ARTIFACTS` named one elsewhere.
//! Other backends' installations each have a directory of their own. A home
//! directory, a directory that contains it, or a filesystem root keeps
//! everything but the runner's own files.

use std::{
    io,
    path::{Path, PathBuf},
};

/// The command that removes the runner of the installation in `directory`:
/// the installation's launcher when an installer made it, else the runner's
/// own executable naming the installation.
pub fn command(directory: &Path, launched: bool, executable: &Path) -> String {
    if launched {
        let launcher = if cfg!(windows) { "run.ps1" } else { "run" };
        let launcher = directory.join(launcher);
        return if cfg!(windows) {
            format!("& {} uninstall", powershell(&launcher))
        } else {
            format!("{} uninstall", sh(&launcher))
        };
    }
    if cfg!(windows) {
        format!(
            "& {} uninstall --home {}",
            powershell(executable),
            powershell(directory)
        )
    } else {
        format!("{} uninstall --home {}", sh(executable), sh(directory))
    }
}

/// `path` as one POSIX shell word.
fn sh(path: &Path) -> String {
    shlex::try_quote(&path.to_string_lossy())
        .expect("a path holds no NUL byte")
        .into_owned()
}

/// `path` as a PowerShell string literal.
fn powershell(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

/// Refuses `directory` unless it holds an installation: the state files an
/// installer writes there (`backend-url` and `release-id`), or the
/// configuration a runner writes as it starts (`runner.json`). A directory
/// without them, such as a home directory a mistaken `DEMI_HOME` names, is
/// never removed.
pub fn installation(directory: &Path) -> io::Result<()> {
    let installed =
        directory.join("backend-url").is_file() && directory.join("release-id").is_file();
    if installed || directory.join("runner.json").is_file() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{} holds no runner installation, so nothing was removed",
        directory.display()
    )))
}

/// The entries the installers and the runner write into an installation's
/// directory: the state files, the launcher, the releases, the logs, the job
/// output, the declared commands and the installation's own artifact cache.
const RUNNER_ENTRIES: &[&str] = &[
    "active.json",
    "artifacts",
    "backend-url",
    "commands",
    "install.lock",
    "jobs",
    "log",
    "release-id",
    "releases",
    "run",
    "run.next",
    "run.ps1",
    "runner-token",
    "runner.json",
    "runner.lock",
    crate::console::LOG,
    crate::console::PREVIOUS_LOG,
];

/// What a removal took.
pub enum Removed {
    /// The installation's whole directory.
    Directory,
    /// Only the runner's own entries: the directory is the user's home, a
    /// directory that contains it, or a filesystem root, which stays with
    /// everything else in it.
    RunnerFiles,
}

impl Removed {
    /// What the removal took from `directory`, and what it left, for the
    /// person at the console.
    pub fn told(&self, directory: &Path) -> String {
        match self {
            Self::Directory => format!("Removed the runner of {}", directory.display()),
            Self::RunnerFiles => format!(
                "Removed the runner's files from {}, which stays with everything else in it: it is your home directory, contains it, or is a filesystem root",
                directory.display()
            ),
        }
    }
}

/// Whether `directory` must stay whole: the user's home directory, a
/// directory that contains it, or the root of a filesystem
/// (`runner.md` § Installation, pairing and removal).
fn kept_whole(directory: &Path, home: Option<&Path>) -> io::Result<bool> {
    let directory = directory.canonicalize()?;
    let home = home.and_then(|home| home.canonicalize().ok());
    if home.is_some_and(|home| home.starts_with(&directory)) {
        return Ok(true);
    }
    let Some(parent) = directory.parent() else {
        return Ok(true);
    };
    // A mount point is the root of the filesystem mounted there.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if std::fs::metadata(&directory)?.dev() != std::fs::metadata(parent)?.dev() {
            return Ok(true);
        }
    }
    #[cfg(not(unix))]
    let _ = parent;
    Ok(false)
}

/// The one check both removals share (`uninstall` and a revocation): the
/// paths to remove from `directory`, which must hold an installation, and
/// what that removal takes.
fn plan(directory: &Path) -> io::Result<(Vec<PathBuf>, Removed)> {
    installation(directory)?;
    let home = std::env::home_dir().filter(|home| !home.as_os_str().is_empty());
    if !kept_whole(directory, home.as_deref())? {
        return Ok((vec![directory.to_owned()], Removed::Directory));
    }
    let entries = RUNNER_ENTRIES
        .iter()
        .map(|entry| directory.join(entry))
        .filter(|path| path.symlink_metadata().is_ok())
        .collect();
    Ok((entries, Removed::RunnerFiles))
}

/// Removes the installation in `directory`, which nothing of this runner
/// uses any more. A running program's file can be deleted on Unix, so it
/// goes at once.
#[cfg(unix)]
pub fn remove(directory: &Path) -> io::Result<Removed> {
    let (paths, removed) = plan(directory)?;
    for path in paths {
        if path.symlink_metadata()?.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(removed)
}

/// Removes the installation in `directory`. Windows deletes no running
/// program's file, and this runner's executable, like that of an `uninstall`
/// that waits for it, lies in the installation: a detached PowerShell
/// process removes the paths once they exited, trying for a minute.
#[cfg(windows)]
pub fn remove(directory: &Path) -> io::Result<Removed> {
    use std::os::windows::process::CommandExt as _;
    // A process of its own: no console, and no part in this one's Ctrl+C.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const SCRIPT: &str = "$paths = $env:DEMI_REMOVED_PATHS -split \"`n\"; \
        for ($i = 0; $i -lt 120; $i++) { \
        $left = @($paths | Where-Object { Test-Path -LiteralPath $_ }); \
        if ($left.Count -eq 0) { exit 0 }; \
        foreach ($p in $left) { try { Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction Stop } catch {} }; \
        Start-Sleep -Milliseconds 500 }; \
        exit 1";
    let (paths, removed) = plan(directory)?;
    let joined = paths
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("DEMI_REMOVED_PATHS", joined)
        // A process's working directory cannot be removed while it runs.
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()?;
    Ok(removed)
}
