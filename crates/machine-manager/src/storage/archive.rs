//! The check of a base archive before it is extracted: every entry is a
//! regular file, directory, symbolic link or hard link, and no path or
//! hard-link target is absolute or climbs out with `..`.

use std::{
    io,
    path::{Component, Path},
};

use tar::EntryType;

use crate::blocking::OffLoop;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Cloud archive contains an unsafe path or entry type")]
    UnsafeEntry,
    #[error("Cloud archive contains an unsafe hardlink")]
    UnsafeHardlink,
}

/// Reads every entry of the zstd-compressed tar archive at `archive`.
pub fn vet(_: &OffLoop, archive: &Path) -> Result<(), ArchiveError> {
    let decoder = zstd::stream::read::Decoder::new(fs_err::File::open(archive)?)?;
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let entry = entry?;
        let kind = entry.header().entry_type();
        match kind {
            // Metadata for the whole archive, not a member.
            EntryType::XGlobalHeader => continue,
            EntryType::Regular | EntryType::Directory | EntryType::Symlink | EntryType::Link => {}
            _ => return Err(ArchiveError::UnsafeEntry),
        }
        if !stays_inside(&entry.path()?) {
            return Err(ArchiveError::UnsafeEntry);
        }
        if kind == EntryType::Link {
            let target = entry.link_name()?.ok_or(ArchiveError::UnsafeHardlink)?;
            if !stays_inside(&target) {
                return Err(ArchiveError::UnsafeHardlink);
            }
        }
    }
    Ok(())
}

/// Whether `path` names something beneath the extraction root.
fn stays_inside(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}
