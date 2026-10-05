//! Removing a paired device's runner (`runner.md` § Installation, pairing
//! and removal): the command that does it, which the runner tells the person
//! at its console once it is paired.

use std::path::Path;

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
