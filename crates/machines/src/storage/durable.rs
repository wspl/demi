//! Durable writes of the manager's records: a record replaces its file
//! atomically through `artifact`'s publication, and a directory is synced
//! after an entry in it changes, so a crash leaves the old state or the new
//! one.

use std::{io, path::Path};

use demi_artifact::{Mode, Permissions, Publication};
use serde::Serialize;

use crate::blocking::OffLoop;

/// Replaces `path` with `value`'s JSON, synced before the rename. The caller
/// syncs the directory once the rename must survive a crash.
pub fn write_json(_: &OffLoop, path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let publication = Publication {
        mode: Mode::Replace,
        permissions: Permissions::Default,
        durable: true,
    };
    demi_artifact::publish_bytes_blocking(path, &bytes, publication).map_err(|error| {
        let kind = match &error {
            demi_artifact::Error::Io(error) => error.kind(),
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, format!("failed to write {}: {error}", path.display()))
    })
}

/// Syncs a file's data or a directory's entries to disk.
pub fn sync(_: &OffLoop, path: &Path) -> io::Result<()> {
    fs_err::File::open(path)?.sync_all()
}

/// Removes the directory tree at `path`; one that is gone already is fine.
pub fn remove_tree(_: &OffLoop, path: &Path) -> io::Result<()> {
    match fs_err::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Creates the directory `path` and any missing parents, each private to
/// the owner; a directory that exists keeps its mode.
pub fn create_private(_: &OffLoop, path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| io::Error::new(error.kind(), format!("failed to create {}: {error}", path.display())))
}
