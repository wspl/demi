//! The gVisor runtime on a server: each release's machine manager fetches
//! the runsc version it pins into `gvisor/<version>/`
//! (`crates/machine-manager/src/gvisor_release.rs`), and `demi-server
//! runtime` has the manager of its own release do it.
//!
//! Another release's version is that release's own `demi-server`'s to fetch and
//! name, so a setup or a move asks each release's program, and a move keeps the
//! versions of its two releases.

use std::{
    collections::HashSet,
    io,
    path::PathBuf,
    process::{Command, Stdio},
};

use semver::Version;

use crate::layout::Layout;

/// Has the manager of this program's release fetch the version it pins into
/// `layout`'s gVisor directory, and returns the version's directory.
pub fn fetch(layout: &Layout) -> io::Result<PathBuf> {
    let program = std::env::current_exe()?;
    let manager = program
        .parent()
        .map(|bin| bin.join("demi-machine-manager"))
        .ok_or_else(|| io::Error::other("demi-server lies in no directory"))?;
    let output = Command::new(&manager)
        .arg("--fetch-gvisor")
        .arg(layout.gvisor())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("{} could not fetch gVisor: {}", manager.display(), output.status)));
    }
    named(layout, &output.stdout)
}

/// The version directory a program printed, which must lie in `layout`'s
/// gVisor directory.
fn named(layout: &Layout, printed: &[u8]) -> io::Result<PathBuf> {
    let printed = std::str::from_utf8(printed).map_err(io::Error::other)?;
    let directory = PathBuf::from(printed.trim_end_matches('\n'));
    if directory.parent() != Some(layout.gvisor().as_path()) {
        return Err(io::Error::other(format!(
            "{} is not in {}",
            directory.display(),
            layout.gvisor().display()
        )));
    }
    Ok(directory)
}

/// Whether the server has fetched a gVisor version before, as one that ran
/// gVisor or whose setup needed it has, so its upgrades fetch the next one;
/// a stage a download left is not a version.
pub fn downloaded(layout: &Layout) -> io::Result<bool> {
    match std::fs::read_dir(layout.gvisor()) {
        Ok(entries) => {
            for entry in entries {
                if !entry?.file_name().to_string_lossy().starts_with('.') {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Has `version`'s own `demi-server` fetch the gVisor version its release
/// pins, and returns its directory.
pub fn of_release(layout: &Layout, version: &Version) -> io::Result<PathBuf> {
    let program = layout.release(version).join("bin/demi-server");
    let output = Command::new(&program)
        .arg("runtime")
        .arg("--root")
        .arg(layout.root())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{version} could not fetch its gVisor runtime: {}",
            output.status
        )));
    }
    named(layout, &output.stdout)
}

/// Keeps the gVisor versions of `releases` and removes every other entry,
/// such as an earlier release's version or a killed run's stage.
pub fn prune(layout: &Layout, releases: &[&Version]) -> io::Result<()> {
    // Naming a release's version fetches it, which a server that runs no
    // gVisor must not.
    if !downloaded(layout)? {
        return Ok(());
    }
    let mut kept = HashSet::new();
    for version in releases {
        kept.insert(of_release(layout, version)?);
    }
    let entries = match std::fs::read_dir(layout.gvisor()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let path = entry?.path();
        if !kept.contains(&path) {
            std::fs::remove_dir_all(&path)?;
        }
    }
    Ok(())
}
