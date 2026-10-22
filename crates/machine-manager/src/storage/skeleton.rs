//! A new home's first contents: the base's `/etc/skel`, copied with
//! symbolic links as they are and every entry owned by the sandbox's
//! user.

use std::{io, path::Path};

use crate::{blocking::OffLoop, sandbox::files::USER_ID};

/// Copies the directory `skeleton` to the new directory `destination`.
pub fn copy(_: &OffLoop, skeleton: &Path, destination: &Path) -> io::Result<()> {
    for entry in walkdir::WalkDir::new(skeleton)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry?;
        let relative = entry
            .path()
            .strip_prefix(skeleton)
            .expect("a walked entry lies under its root");
        let target = destination.join(relative);
        let kind = entry.file_type();
        if kind.is_dir() {
            fs_err::create_dir(&target)?;
            fs_err::set_permissions(&target, entry.metadata()?.permissions())?;
        } else if kind.is_symlink() {
            fs_err::os::unix::fs::symlink(fs_err::read_link(entry.path())?, &target)?;
        } else if kind.is_file() {
            fs_err::copy(entry.path(), &target)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is neither a file, a directory nor a link",
                    entry.path().display()
                ),
            ));
        }
        fs_err::os::unix::fs::lchown(&target, Some(USER_ID), Some(USER_ID))?;
    }
    Ok(())
}
