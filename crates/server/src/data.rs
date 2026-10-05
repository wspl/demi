//! The backend's databases across a move (`upgrades.md` § Switch,
//! § Rollback): the copy an upgrade takes of each database its release will
//! migrate, into `<data>/snapshots/<version>/`, and a rollback's return of
//! that copy, which first moves the databases it replaces to
//! `snapshots/abandoned-<version>/`. Both services are stopped, so the files
//! are consistent. Copies use the file system's copy, which clones where it
//! can.

use std::{
    io,
    path::{Path, PathBuf},
};

use demi_backend_database::{DatabaseKind, schema_differs};
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

/// The databases of `data` that this release would migrate, relative to it.
pub fn migrating(data: &Path) -> io::Result<Vec<PathBuf>> {
    let mut migrating = Vec::new();
    for (database, kind) in databases(data)? {
        if schema_differs(&data.join(&database), kind).map_err(io::Error::other)? {
            migrating.push(database);
        }
    }
    Ok(migrating)
}

/// The bytes `databases` take, which their copy needs free.
pub fn size(data: &Path, databases: &[PathBuf]) -> io::Result<u64> {
    let mut total = 0;
    for database in databases {
        total += std::fs::metadata(data.join(database))?.len();
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

/// Copies `databases` into the snapshot of `version`, made anew.
pub fn take(data: &Path, version: &Version, databases: &[PathBuf]) -> io::Result<()> {
    let snapshot = snapshot(data, version);
    remove(&snapshot)?;
    copy_all(data, &snapshot, databases)
}

/// Puts back the databases of `version`'s snapshot, after moving the ones
/// they replace aside as abandoned by `leaving`. Taken again after an
/// interruption, it puts back the same copy.
pub fn restore(data: &Path, version: &Version, leaving: &Version) -> io::Result<()> {
    let snapshot = snapshot(data, version);
    let saved = relative_files(&snapshot)?;
    let aside = abandoned(data, leaving);
    if !aside.exists() {
        let present: Vec<PathBuf> = saved
            .iter()
            .filter(|database| data.join(database).exists())
            .cloned()
            .collect();
        let staged = aside.with_file_name(format!(".{}", aside.file_name().unwrap().to_string_lossy()));
        remove(&staged)?;
        copy_all(data, &staged, &present)?;
        std::fs::rename(&staged, &aside)?;
    }
    let owner = std::fs::metadata(data)?;
    for database in &saved {
        let target = data.join(database);
        // A write-ahead log beside the database belongs to the one replaced.
        for suffix in ["-wal", "-shm"] {
            remove_file(&with_suffix(&target, suffix))?;
        }
        std::fs::copy(snapshot.join(database), &target)?;
        own_like(&target, &owner)?;
        std::fs::File::open(&target)?.sync_all()?;
    }
    Ok(())
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

fn copy_all(data: &Path, into: &Path, databases: &[PathBuf]) -> io::Result<()> {
    for database in databases {
        let target = into.join(database);
        std::fs::create_dir_all(target.parent().expect("a database lies in a directory"))?;
        std::fs::copy(data.join(database), &target)?;
        std::fs::File::open(&target)?.sync_all()?;
    }
    if into.exists() {
        crate::layout::sync_directory(into)?;
    }
    Ok(())
}

/// The files under `directory`, relative to it.
fn relative_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in std::fs::read_dir(directory.join(&relative))? {
            let entry = entry?;
            let path = relative.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
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

fn remove_file(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}
