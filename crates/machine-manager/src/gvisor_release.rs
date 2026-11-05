//! The gVisor release a manager runs: the runsc version it pins, fetched
//! into `<gvisor>/<version>/`, checked against the pinned SHA-512, with only
//! the programs the manager runs.
//!
//! The manager fetches it when the master first chooses gVisor, and
//! `demi-server runtime`, which a setup and an upgrade run, has a release's
//! manager fetch it (`--fetch-gvisor`), so both use this one implementation.
//!
//! The archive is upstream's release of the pinned version for the server's
//! architecture. Only what the manager runs is unpacked: the distribution's
//! other programs, the containerd shim, the metric server and the memory
//! checkpoint gofer, stay out, and leaving them halves the size.

use std::{
    collections::HashSet,
    io,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use demi_machine_manager_protocol::runtime::{PROGRAMS, RuntimeRelease};
use sha2::{Digest as _, Sha512};
use tokio_util::sync::CancellationToken;

use crate::{
    blocking::{self, OffLoop},
    storage::durable::{remove_tree, sync},
};

/// The distribution's archive is far below this; a download is written to
/// the disk, which a wrong or hostile source would otherwise fill.
const LARGEST: u64 = 1 << 30;

/// What a download's stage in the gVisor directory is named after, before a
/// UUID of its own.
const STAGE: &str = ".staging-";

/// Fetches the version this manager pins into `gvisor`, unless it is there,
/// and returns its directory. `progress` hears the bytes received, and of
/// how many when the source says.
pub async fn fetch(gvisor: &Path, cancel: &CancellationToken, progress: impl FnMut(u64, Option<u64>)) -> io::Result<PathBuf> {
    let pinned = RuntimeRelease::pinned();
    let destination = gvisor.join(pinned.version());
    let stage = gvisor.join(format!("{STAGE}{}", uuid::Uuid::new_v4()));
    let present = blocking::run({
        let (gvisor, destination, stage) = (gvisor.to_owned(), destination.clone(), stage.clone());
        move |_: &OffLoop| -> io::Result<bool> {
            if destination.exists() {
                return Ok(true);
            }
            fs_err::create_dir_all(&gvisor)?;
            // A killed run leaves its stage behind, and a server that has
            // never finished a download has no version for an upgrade to
            // prune beside it, so the next download removes it.
            for entry in fs_err::read_dir(&gvisor)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with(STAGE) {
                    fs_err::remove_dir_all(entry.path())?;
                }
            }
            fs_err::create_dir(&stage)?;
            Ok(false)
        }
    })
    .await?;
    if present {
        return Ok(destination);
    }
    let fetched = fetch_into(&pinned, &stage, &destination, cancel, progress).await;
    let gvisor = gvisor.to_owned();
    blocking::run(move |off| -> io::Result<()> {
        let removed = remove_tree(off, &stage);
        fetched?;
        removed?;
        sync(off, &gvisor)
    })
    .await?;
    Ok(destination)
}

async fn fetch_into(
    pinned: &RuntimeRelease,
    stage: &Path,
    destination: &Path,
    cancel: &CancellationToken,
    progress: impl FnMut(u64, Option<u64>),
) -> io::Result<()> {
    let archive = stage.join("gvisor.tar.bz2");
    let url = pinned.archive_url();
    let client = demi_shared_artifacts::download_client().map_err(io::Error::other)?;
    let request = demi_shared_artifacts::Download {
        limit: LARGEST,
        redirects: 10,
        ..demi_shared_artifacts::Download::new(&url)
    };
    demi_shared_artifacts::download(&client, request, &archive, cancel, progress)
        .await
        .and_then(|received| received.plain())
        .map_err(|error| io::Error::other(format!("{url}: {error}")))?;
    let root = stage.join("root");
    let (expected, version) = (pinned.archive_sha512().to_owned(), pinned.version());
    let destination = destination.to_owned();
    blocking::run(move |_| {
        let found = sha512(&archive)?;
        if found != expected {
            return Err(io::Error::other(format!("{url} does not match the pinned SHA-512 of gVisor {version}")));
        }
        unpack(&archive, &root)?;
        match fs_err::rename(&root, &destination) {
            Ok(()) => Ok(()),
            // Another run fetched the same version first.
            Err(_) if destination.exists() => Ok(()),
            Err(error) => Err(error),
        }
    })
    .await
}

/// The SHA-512 of the file at `path`, in lowercase hex.
fn sha512(path: &Path) -> io::Result<String> {
    let mut hasher = Sha512::new();
    io::copy(&mut fs_err::File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Unpacks the programs the manager runs from the distribution's archive
/// into the new directory `into`; the archive's other programs stay out.
fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    let decoder = bzip2::read::BzDecoder::new(io::BufReader::new(fs_err::File::open(archive)?));
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
        fs_err::create_dir_all(target.parent().expect("a program lies in a directory"))?;
        entry.unpack(&target)?;
        fs_err::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
        // Whole before the version's directory is renamed into place.
        fs_err::File::open(&target)?.sync_all()?;
        unpacked.insert(*program);
    }
    for program in PROGRAMS {
        if !unpacked.contains(program) {
            return Err(io::Error::other(format!("the gVisor archive holds no {program}")));
        }
    }
    Ok(())
}
