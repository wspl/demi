//! Removing a paired device's runner (`runner.md` § Installation, pairing
//! and removal): the command that does it, which the runner tells the person
//! at its console once it is paired, and the removal of its installation's
//! directory, with everything in it: the device token, the releases, the
//! log, and the artifact cache unless `DEMI_ARTIFACTS` named one elsewhere.
//! Other backends' installations each have a directory of their own.

use std::{io, path::Path};

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

/// Removes the installation's directory, which nothing of this runner uses
/// any more. A running program's file can be deleted on Unix, so the
/// directory goes at once.
#[cfg(unix)]
pub fn remove(directory: &Path) -> io::Result<()> {
    installation(directory)?;
    std::fs::remove_dir_all(directory)
}

/// Removes the installation's directory. Windows deletes no running
/// program's file, and this runner's executable, like that of an `uninstall`
/// that waits for it, lies in the directory: a detached PowerShell process
/// removes the directory once they exited, trying for a minute.
#[cfg(windows)]
pub fn remove(directory: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt as _;
    installation(directory)?;
    // A process of its own: no console, and no part in this one's Ctrl+C.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const SCRIPT: &str = "$d = $env:DEMI_REMOVED_INSTALLATION; \
        for ($i = 0; $i -lt 120; $i++) { \
        if (-not (Test-Path -LiteralPath $d)) { exit 0 }; \
        try { Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction Stop } \
        catch { Start-Sleep -Milliseconds 500 } }; \
        exit 1";
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("DEMI_REMOVED_INSTALLATION", directory)
        // A process's working directory cannot be removed while it runs.
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(drop)
}
