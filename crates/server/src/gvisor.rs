//! The gVisor runtime on a server (`builds-and-releases.md` § gVisor
//! runtime): each release's `demi-server` fetches the runsc version its
//! machine manager pins into `gvisor/<version>/`, checked against the pinned
//! SHA-512, with only the programs the manager runs. Another release's
//! version is that release's own `demi-server`'s to fetch and name, so a
//! setup or a move asks each release's program, and a move keeps the versions
//! of its two releases.

use std::{
    collections::HashSet,
    io,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use demi_machine_manager_protocol::runtime::{PROGRAMS, RuntimeRelease};
use semver::Version;
use sha2::{Digest as _, Sha512};
use tokio_util::sync::CancellationToken;

use crate::layout::{Layout, sync_directory};

/// The distribution's archive is far below this.
const LARGEST: u64 = 1 << 30;

/// Fetches the version this program's release pins, unless it is there, and
/// returns its directory.
pub async fn fetch(layout: &Layout, cancel: &CancellationToken) -> io::Result<PathBuf> {
    let pinned = RuntimeRelease::pinned();
    let gvisor = layout.gvisor();
    let destination = gvisor.join(pinned.version());
    if destination.exists() {
        return Ok(destination);
    }
    std::fs::create_dir_all(&gvisor)?;
    // A stage a killed run leaves behind goes with the next move's prune.
    let stage = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&gvisor)?;
    let archive = stage.path().join("gvisor.tar.bz2");
    let url = pinned.archive_url();
    let client = demi_shared_artifacts::client_following_redirects().map_err(io::Error::other)?;
    let mut file = tokio::fs::File::create(&archive).await?;
    demi_shared_artifacts::download_measured(&client, &url, LARGEST, &mut file, cancel)
        .await
        .map_err(|error| io::Error::other(format!("{url}: {error}")))?;
    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    drop(file);
    let root = stage.path().join("root");
    let unpacked = root.clone();
    let expected = pinned.archive_sha512().to_owned();
    tokio::task::spawn_blocking(move || {
        let found = sha512(&archive)?;
        if found != expected {
            return Err(io::Error::other(format!(
                "{url} does not match the pinned SHA-512 of gVisor {}",
                pinned.version()
            )));
        }
        unpack(&archive, &unpacked)
    })
    .await
    .map_err(io::Error::other)??;
    match std::fs::rename(&root, &destination) {
        Ok(()) => {}
        // Another run fetched the same version first.
        Err(_) if destination.exists() => return Ok(destination),
        Err(error) => return Err(error),
    }
    sync_directory(&gvisor)?;
    Ok(destination)
}

/// The SHA-512 of the file at `path`, in lowercase hex.
fn sha512(path: &Path) -> io::Result<String> {
    let mut hasher = Sha512::new();
    io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Unpacks the programs the manager runs from the distribution's archive
/// into the new directory `into`; the archive's other programs stay out.
fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    let decoder = bzip2::read::BzDecoder::new(io::BufReader::new(std::fs::File::open(archive)?));
    let mut entries = tar::Archive::new(decoder);
    let mut unpacked = HashSet::new();
    for entry in entries.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let name = path.to_string_lossy();
        let name = name.trim_start_matches("./");
        let Some(program) = PROGRAMS.iter().find(|program| **program == name) else {
            continue;
        };
        if !entry.header().entry_type().is_file() {
            return Err(io::Error::other(format!("{program} in the gVisor archive is no file")));
        }
        let target = into.join(program);
        std::fs::create_dir_all(target.parent().expect("a program lies in a directory"))?;
        entry.unpack(&target)?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
        // Whole before the version's directory is renamed into place.
        std::fs::File::open(&target)?.sync_all()?;
        unpacked.insert(*program);
    }
    for program in PROGRAMS {
        if !unpacked.contains(program) {
            return Err(io::Error::other(format!("the gVisor archive holds no {program}")));
        }
    }
    Ok(())
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
    let printed = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    let directory = PathBuf::from(printed.trim_end_matches('\n'));
    if directory.parent() != Some(layout.gvisor().as_path()) {
        return Err(io::Error::other(format!(
            "{version} named {} as its gVisor runtime, which is not in {}",
            directory.display(),
            layout.gvisor().display()
        )));
    }
    Ok(directory)
}

/// Keeps the gVisor versions of `releases` and removes every other entry,
/// such as an earlier release's version or a killed run's stage.
pub fn prune(layout: &Layout, releases: &[&Version]) -> io::Result<()> {
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
