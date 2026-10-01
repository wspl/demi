//! The check of a base archive before it is extracted (`images.md` § Import
//! and publication): every entry is a regular file, directory, symbolic link
//! or hard link, and no path or hard-link target is absolute or climbs out
//! with `..`.

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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// An archive of `entries`: (type, path, hard-link target). Names are
    /// written into the header directly, so unsafe ones can be made.
    fn archive(directory: &Path, entries: &[(EntryType, &str, Option<&str>)]) -> std::path::PathBuf {
        let path = directory.join("rootfs.tar.zst");
        let file = std::fs::File::create(&path).unwrap();
        let encoder = zstd::stream::write::Encoder::new(file, 0).unwrap().auto_finish();
        let mut builder = tar::Builder::new(encoder);
        for (kind, name, target) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(0o644);
            header.set_size(0);
            let old = header.as_old_mut();
            old.name[..name.len()].copy_from_slice(name.as_bytes());
            if let Some(target) = target {
                old.linkname[..target.len()].copy_from_slice(target.as_bytes());
            }
            header.set_cksum();
            builder.append(&header, io::empty()).unwrap();
        }
        builder.into_inner().unwrap().flush().unwrap();
        path
    }

    #[test]
    fn safe_entries_pass_and_unsafe_ones_are_refused() {
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let safe = archive(
            directory.path(),
            &[
                (EntryType::Directory, "./usr/", None),
                (EntryType::Regular, "./usr/bin/tini", None),
                (EntryType::Symlink, "./bin", Some("/usr/bin")),
                (EntryType::Link, "./usr/bin/init", Some("./usr/bin/tini")),
            ],
        );
        vet(&off, &safe).unwrap();
        for (entries, error) in [
            (vec![(EntryType::Regular, "/etc/passwd", None)], "unsafe path"),
            (vec![(EntryType::Regular, "./../escape", None)], "unsafe path"),
            (vec![(EntryType::Fifo, "./run/pipe", None)], "unsafe path"),
            (vec![(EntryType::Char, "./dev/null", None)], "unsafe path"),
            (vec![(EntryType::Block, "./dev/sda", None)], "unsafe path"),
            (vec![(EntryType::Link, "./etc/shadow", Some("../../etc/shadow"))], "unsafe hardlink"),
            (vec![(EntryType::Link, "./etc/shadow", Some("/etc/shadow"))], "unsafe hardlink"),
        ] {
            let unsafe_archive = archive(directory.path(), &entries);
            let message = vet(&off, &unsafe_archive).expect_err("refused").to_string();
            assert!(message.contains(error), "{entries:?}: {message}");
        }
    }
}
