//! The backend's databases across a move (`upgrades.md` § Switch, § Rollback):
//!
//! - An upgrade keeps each database its release will migrate in
//!   `<data>/snapshots/<version>/`: a copy of one it migrates in place, and a
//!   hard link to one it converts, which it replaces by a new file and never
//!   writes, so the link costs no space.
//! - A rollback returns what the snapshot keeps. It moves the databases it
//!   replaces to `snapshots/abandoned-<version>/` by a rename, and puts the
//!   snapshot's in their place, linking one that converts and copying one
//!   that migrates in place, so the snapshot stays whole for a return taken
//!   again.
//! - Before a rollback hands the move to the earlier release, each database
//!   the snapshot still shares with the data directory gets a file of its
//!   own ([`separate`]): an earlier release copies a snapshot back onto the
//!   database, which would empty a shared file.
//!
//! Both services are stopped, so the files are consistent. Copies use the file
//! system's copy, which clones where it can.

use std::{
    io,
    path::{Path, PathBuf},
};

use demi_backend_database::{DatabaseKind, SchemaFit, schema_fit};
use semver::Version;

/// The databases of a data directory, relative to it, with their kinds.
fn databases(data: &Path) -> io::Result<Vec<(PathBuf, DatabaseKind)>> {
    let mut databases = Vec::new();
    let control = PathBuf::from("control.sqlite");
    if data.join(&control).exists() {
        databases.push((control, DatabaseKind::Control));
    }
    match std::fs::read_dir(data.join("conversations")) {
        Ok(entries) => {
            for entry in entries {
                let name = entry?.file_name();
                if Path::new(&name).extension().is_some_and(|extension| extension == "sqlite") {
                    databases.push((
                        Path::new("conversations").join(name),
                        DatabaseKind::Conversation,
                    ));
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    databases.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok(databases)
}

/// A database a release would change, relative to the data directory, and
/// how its snapshot keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrating {
    pub database: PathBuf,
    /// Whether the release converts it into a new file and leaves it as it
    /// is, so a hard link keeps it.
    pub converts: bool,
}

/// The databases of `data` that this release would migrate or convert,
/// relative to it. A database it could not open, of a schema neither its
/// own nor in its history, stops the move here, before anything is
/// interrupted, naming each such file.
pub fn migrating(data: &Path) -> io::Result<Vec<Migrating>> {
    let mut migrating = Vec::new();
    let mut other = Vec::new();
    for (database, kind) in databases(data)? {
        let path = data.join(&database);
        match schema_fit(&path, kind).map_err(io::Error::other)? {
            SchemaFit::Current => {}
            SchemaFit::Migrates => migrating.push(Migrating { database, converts: false }),
            SchemaFit::Converts => migrating.push(Migrating { database, converts: true }),
            SchemaFit::Other => other.push(path.display().to_string()),
        }
    }
    if !other.is_empty() {
        return Err(io::Error::other(format!(
            "this release cannot open {}: its schema is neither this release's nor one it migrates from, as when a development build of another unpublished schema made it",
            other.join(", ")
        )));
    }
    Ok(migrating)
}

/// The bytes the snapshot of `databases` needs free: those of the ones it
/// copies.
pub fn size(data: &Path, databases: &[Migrating]) -> io::Result<u64> {
    let mut total = 0;
    for migrating in databases.iter().filter(|migrating| !migrating.converts) {
        total += std::fs::metadata(data.join(&migrating.database))?.len();
    }
    Ok(total)
}

/// Whether the file system of `data` has `bytes` free.
pub fn room(data: &Path, bytes: u64) -> io::Result<bool> {
    let stats = rustix::fs::statvfs(data)?;
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize) >= bytes)
}

pub fn snapshot(data: &Path, version: &Version) -> PathBuf {
    data.join("snapshots").join(version.to_string())
}

fn abandoned(data: &Path, version: &Version) -> PathBuf {
    data.join("snapshots").join(format!("abandoned-{version}"))
}

/// Keeps `databases` in the snapshot of `version`, made anew.
pub fn take(
    data: &Path,
    version: &Version,
    databases: &[Migrating],
    progress: &mut dyn FnMut(u64, u64),
) -> io::Result<()> {
    let snapshot = snapshot(data, version);
    remove(&snapshot)?;
    let copied: Vec<&Migrating> = databases.iter().filter(|migrating| !migrating.converts).collect();
    let total = copied
        .iter()
        .map(|migrating| std::fs::metadata(data.join(&migrating.database)).map(|metadata| metadata.len()))
        .sum::<io::Result<u64>>()?;
    let mut done = 0;
    progress(done, total);
    for migrating in databases {
        let target = snapshot.join(&migrating.database);
        std::fs::create_dir_all(target.parent().expect("a database lies in a directory"))?;
        if migrating.converts {
            std::fs::hard_link(data.join(&migrating.database), &target)?;
        } else {
            copy(&data.join(&migrating.database), &target, &mut |read| {
                done += read;
                progress(done, total);
            })?;
            std::fs::File::open(&target)?.sync_all()?;
        }
    }
    if snapshot.exists() {
        sync_tree(&snapshot)?;
    }
    Ok(())
}

/// Puts back the databases of `version`'s snapshot, after moving the ones
/// they replace aside as abandoned by `leaving`, each by a rename.
///
/// The snapshot stays whole, so a return taken again after the next attempt
/// at the move puts the same databases back: one that converts by a hard
/// link, since no release writes it then, and one that migrates in place by
/// a copy, since the next attempt migrates it again. Taken again, it moves
/// aside only what no earlier run moved there, and puts each database back
/// anew.
pub fn restore(data: &Path, version: &Version, leaving: &Version) -> io::Result<()> {
    let snapshot = snapshot(data, version);
    if !snapshot.exists() {
        return Ok(());
    }
    let aside = abandoned(data, leaving);
    let owner = std::fs::metadata(data)?;
    let mut touched = std::collections::BTreeSet::new();
    for (database, kind) in databases(&snapshot)? {
        let kept = snapshot.join(&database);
        let target = data.join(&database);
        let moved = aside.join(&database);
        std::fs::create_dir_all(moved.parent().expect("a database lies in a directory"))?;
        // A write-ahead log beside the database belongs to it, and goes
        // with it, so none is left to stand beside the one put back.
        let first = !moved.exists();
        for suffix in ["-wal", "-shm", ""] {
            if first {
                rename_if_there(&with_suffix(&target, suffix), &with_suffix(&moved, suffix))?;
            } else {
                remove_file_if_there(&with_suffix(&target, suffix))?;
            }
        }
        if schema_fit(&kept, kind).map_err(io::Error::other)? == SchemaFit::Converts {
            std::fs::hard_link(&kept, &target)?;
        } else {
            std::fs::copy(&kept, &target)?;
            std::fs::File::open(&target)?.sync_all()?;
        }
        own_like(&target, &owner)?;
        touched.extend([target.parent(), moved.parent()].into_iter().flatten().map(Path::to_path_buf));
    }
    for directory in touched {
        crate::layout::sync_directory(&directory)?;
    }
    Ok(())
}

/// Gives each database that `version`'s snapshot shares with the data
/// directory, by a hard link to one not converted yet, a file of its own,
/// copied from the snapshot's.
///
/// A rollback hands the move to the earlier release, and one that puts a
/// snapshot back by copying it onto the database would empty a file the
/// two share, since a copy truncates its target first. Copied from the
/// snapshot's name, the copy is the database as the upgrade found it even
/// when a conversion replaces the one in the data directory meanwhile.
pub fn separate(data: &Path, version: &Version) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let snapshot = snapshot(data, version);
    if !snapshot.exists() {
        return Ok(());
    }
    let owner = std::fs::metadata(data)?;
    for (database, _) in databases(&snapshot)? {
        let kept = snapshot.join(&database);
        let Ok(live) = std::fs::metadata(data.join(&database)) else {
            continue;
        };
        let held = std::fs::metadata(&kept)?;
        if (live.dev(), live.ino()) != (held.dev(), held.ino()) {
            continue;
        }
        let staged = with_suffix(&kept, ".copy");
        std::fs::copy(&kept, &staged)?;
        std::fs::File::open(&staged)?.sync_all()?;
        own_like(&staged, &owner)?;
        std::fs::rename(&staged, &kept)?;
        crate::layout::sync_directory(kept.parent().expect("a database lies in a directory"))?;
    }
    Ok(())
}

/// Syncs `directory` and the directories below it, so the renames and links
/// in them are on disk.
fn sync_tree(directory: &Path) -> io::Result<()> {
    crate::layout::sync_directory(directory)?;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_tree(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_file_if_there(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn rename_if_there(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Removes the snapshots but those of `kept`, as a move that succeeded
/// does.
pub fn prune(data: &Path, kept: &[String]) -> io::Result<()> {
    let snapshots = data.join("snapshots");
    let entries = match std::fs::read_dir(&snapshots) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if !kept.iter().any(|name| entry.file_name() == name.as_str()) {
            remove(&entry.path())?;
        }
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Gives `path` the owner of the data directory, whose service user
/// writes it; a copy that the root `demi-server` made is root's otherwise.
fn own_like(path: &Path, owner: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    if std::fs::metadata(path)?.uid() == owner.uid() {
        return Ok(());
    }
    std::os::unix::fs::chown(path, Some(owner.uid()), Some(owner.gid()))
}

fn remove(path: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}


/// Copies `from` to `to` a range at a time, telling each range's bytes, as
/// `std::fs::copy` copies: through `copy_file_range`, which clones where the
/// file system can, or by reads and writes where the kernel refuses it.
fn copy(from: &Path, to: &Path, copied: &mut dyn FnMut(u64)) -> io::Result<()> {
    use std::io::{Read as _, Write as _};
    const RANGE: usize = 64 << 20;
    let mut source = std::fs::File::open(from)?;
    let mut target = std::fs::File::create(to)?;
    let mut ranges = true;
    let mut buffer = Vec::new();
    loop {
        let moved = if ranges {
            match copy_range(&source, &target, RANGE)? {
                Some(moved) => moved,
                None => {
                    ranges = false;
                    buffer.resize(8 << 20, 0);
                    continue;
                }
            }
        } else {
            let read = source.read(&mut buffer)?;
            target.write_all(&buffer[..read])?;
            read
        };
        if moved == 0 {
            break;
        }
        copied(moved as u64);
    }
    target.set_permissions(source.metadata()?.permissions())
}

/// One range of `source` copied onto `target` by the kernel; none where it
/// does not copy between these files.
#[cfg(target_os = "linux")]
fn copy_range(source: &std::fs::File, target: &std::fs::File, length: usize) -> io::Result<Option<usize>> {
    use rustix::io::Errno;
    match rustix::fs::copy_file_range(source, None, target, None, length) {
        Ok(moved) => Ok(Some(moved)),
        Err(Errno::XDEV | Errno::NOSYS | Errno::INVAL | Errno::OPNOTSUPP) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(target_os = "linux"))]
fn copy_range(_: &std::fs::File, _: &std::fs::File, _: usize) -> io::Result<Option<usize>> {
    Ok(None)
}
