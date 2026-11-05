//! Copying a directory tree: a new home from the base's `/etc/skel`, and a
//! Simple Isolation device's system from its base (`crate::runtime::simple`).
//!
//! Directories, regular files and symbolic links are copied with their
//! modes, setuid and setgid bits included, and files hard linked to each
//! other stay linked, so a copy takes no more room than its source. A copy
//! made for an owner gives every entry to that owner; one made for none
//! keeps the copying process's own. Device nodes, sockets and FIFOs are not
//! copied: a Cloud's devices are the host's, bound at boot.

use std::{collections::HashMap, io, os::unix::fs::MetadataExt as _, path::Path};

use crate::blocking::OffLoop;

/// Copies the directory `from` to the new directory `to`, every entry owned
/// by `owner` when there is one.
pub fn copy(_: &OffLoop, from: &Path, to: &Path, owner: Option<u32>) -> io::Result<()> {
    // The first copy of each file with more than one link, by its source's
    // device and inode.
    let mut linked: HashMap<(u64, u64), std::path::PathBuf> = HashMap::new();
    for entry in walkdir::WalkDir::new(from).follow_links(false).sort_by_file_name() {
        let entry = entry?;
        let relative = entry.path().strip_prefix(from).expect("a walked entry lies under its root");
        let target = to.join(relative);
        let kind = entry.file_type();
        let metadata = entry.metadata()?;
        if kind.is_dir() {
            fs_err::create_dir(&target)?;
        } else if kind.is_symlink() {
            fs_err::os::unix::fs::symlink(fs_err::read_link(entry.path())?, &target)?;
        } else if kind.is_file() {
            if metadata.nlink() > 1 {
                let key = (metadata.dev(), metadata.ino());
                if let Some(first) = linked.get(&key) {
                    fs_err::hard_link(first, &target)?;
                    continue;
                }
                linked.insert(key, target.clone());
            }
            fs_err::copy(entry.path(), &target)?;
        } else {
            continue;
        }
        if let Some(owner) = owner {
            fs_err::os::unix::fs::lchown(&target, Some(owner), Some(owner))?;
        }
        // After the owner, since a change of owner clears the setuid and
        // setgid bits; a link has no mode of its own.
        if !kind.is_symlink() {
            fs_err::set_permissions(&target, metadata.permissions())?;
        }
    }
    Ok(())
}
