//! A slot's network namespace, kept alive by a bind of its namespace file at
//! `/run/netns/<name>`, as `ip netns` keeps one.

use std::{
    io,
    path::{Path, PathBuf},
};

use fs_err::os::unix::fs::OpenOptionsExt;

use crate::{
    blocking::OffLoop,
    linux::{
        mount,
        thread_ns::{self, Namespace},
    },
};

const DIRECTORY: &str = "/run/netns";

/// The file that holds the namespace `name`.
pub fn path(name: &str) -> PathBuf {
    Path::new(DIRECTORY).join(name)
}

/// Creates the network namespace `name`; one of that name must not exist.
pub async fn create(name: &str) -> io::Result<()> {
    let file = path(name);
    thread_ns::run(Namespace::NewNetwork, move |off| {
        fs_err::create_dir_all(DIRECTORY)?;
        fs_err::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o444)
            .open(&file)?;
        let bound = mount::bind(off, Path::new("/proc/thread-self/ns/net"), &file);
        if bound.is_err() {
            // The namespace ends with this thread; the empty file is
            // removed so a later create can succeed, and a failure to remove
            // it is reported by that create.
            let _ = fs_err::remove_file(&file);
        }
        bound
    })
    .await
}

/// Removes the network namespace `name` if it exists: its file is detached
/// and deleted, and the kernel frees the namespace once nothing uses it.
pub fn remove(off: &OffLoop, name: &str) -> io::Result<()> {
    let file = path(name);
    if mount::mount_root(off, &file)? == Some(true) {
        mount::detach(off, &file)?;
    }
    match fs_err::remove_file(&file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
