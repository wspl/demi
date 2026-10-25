//! A slot's network namespace, kept alive by a bind of its namespace file in
//! the manager's own directory, as `ip netns` keeps one in `/run/netns`.
//!
//! The directory is a tmpfs of the manager's mount namespace, with private
//! propagation. That namespace receives the host's mount events, and the
//! host's `/run/netns` is any program's: the first `ip netns add` after a
//! boot, which `runsc do` runs too, binds that directory onto itself, and the
//! bind would cover the manager's files beneath it. A removal would then find
//! an unmounted file there and fail as busy. The tmpfs is mounted once the
//! saved namespace is pinned, since the pin's bind of the runtime directory on
//! the host reaches this namespace the same way.
//!
//! ```text
//! /run/demi-machine-manager/netns/demi-3   a bind of the slot's namespace file
//! ```

use std::{
    io,
    path::{Path, PathBuf},
};

use fs_err::os::unix::fs::OpenOptionsExt;

use crate::{
    blocking::OffLoop,
    config::RUNTIME_DIRECTORY,
    linux::{
        mount,
        thread_ns::{self, Namespace},
    },
};

fn directory() -> PathBuf {
    Path::new(RUNTIME_DIRECTORY).join("netns")
}

/// The file that holds the namespace `name`.
pub fn path(name: &str) -> PathBuf {
    directory().join(name)
}

/// Mounts the directory of namespace files, private to this mount namespace.
pub fn mount_directory(off: &OffLoop) -> io::Result<()> {
    let directory = directory();
    crate::storage::durable::create_private(off, &directory)?;
    mount::tmpfs(off, &directory, "mode=0700")?;
    mount::make_private(off, &directory)
}

/// Creates the network namespace `name`; one of that name must not exist.
pub async fn create(name: &str) -> io::Result<()> {
    let file = path(name);
    thread_ns::run(Namespace::NewNetwork, move |off| {
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
