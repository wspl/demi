//! Copying a directory tree: a new home from the base's `/etc/skel`, a
//! Simple Isolation device's system from its base (`crate::runtime::simple`),
//! and the difference between a system and its base, which a move to gVisor
//! makes its system filesystem (`crate::runtime::moves`).
//!
//! Directories, regular files and symbolic links are copied with their
//! modes, setuid and setgid bits included, and files hard linked to each
//! other stay linked, so a copy takes no more room than its source. A copy
//! made for an owner gives every entry to that owner; one made for none
//! keeps the copying process's own. Device nodes, sockets and FIFOs are not
//! copied: a Cloud's devices are the host's, bound at boot.
//!
//! A copy given a stop token ends at the next entry once it is cancelled,
//! with an `Interrupted` error, leaving what it copied for its caller to
//! remove: a move, which can take minutes, ends that way when the manager
//! stops.

use std::{
    collections::{HashMap, HashSet},
    io::{self, Read as _},
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
};

use tokio_util::sync::CancellationToken;

use crate::blocking::OffLoop;

/// Fails with `Interrupted` once `stop` is cancelled.
fn check(stop: Option<&CancellationToken>) -> io::Result<()> {
    match stop.is_some_and(CancellationToken::is_cancelled) {
        true => Err(io::Error::new(io::ErrorKind::Interrupted, "the Cloud manager is stopping")),
        false => Ok(()),
    }
}

/// Copies the directory `from` to the new directory `to`, every entry owned
/// by `owner` when there is one, until `stop` is cancelled.
pub fn copy(_: &OffLoop, from: &Path, to: &Path, owner: Option<u32>, stop: Option<&CancellationToken>) -> io::Result<()> {
    // The first copy of each file with more than one link, by its source's
    // device and inode.
    let mut linked: HashMap<(u64, u64), std::path::PathBuf> = HashMap::new();
    for entry in walkdir::WalkDir::new(from).follow_links(false).sort_by_file_name() {
        check(stop)?;
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

/// Writes into the new directory `upper` what the tree `system` changes in
/// `base`, as OverlayFS's upper directory over `base` holds it: each
/// directory, file and link that is new or differs, and a whiteout, a
/// character device 0/0, for each entry of `base` that `system` lacks. The
/// paths in `skip`, relative to the root, are left out, as are the entries
/// below them.
///
/// `system` records no owners: a Simple Isolation device's files are all its
/// user's. A changed entry gets its owner in `base`. A new one gets its
/// directory's: whoever writes in a directory only its owner may write in,
/// such as a log the Cloud's user keeps in its own directory, owns what it
/// writes there.
///
/// An entry of `system` is compared with `base` only below directories both
/// have: `base` may hold a link where `system` holds a directory, and
/// following it would read the host.
pub fn upper(
    _: &OffLoop,
    base: &Path,
    system: &Path,
    upper: &Path,
    skip: &[&Path],
    stop: &CancellationToken,
) -> io::Result<()> {
    fs_err::create_dir(upper)?;
    fs_err::set_permissions(upper, fs_err::symlink_metadata(system)?.permissions())?;
    let skipped = |relative: &Path| skip.iter().any(|skip| relative.starts_with(skip));
    // The directories both trees have, relative to the roots.
    let mut shared: HashSet<PathBuf> = HashSet::from([PathBuf::new()]);
    // The owners of the directories only `system` has.
    let mut created: HashMap<PathBuf, (u32, u32)> = HashMap::new();
    let mut walk = walkdir::WalkDir::new(system).follow_links(false).sort_by_file_name().into_iter();
    while let Some(entry) = walk.next() {
        check(Some(stop))?;
        let entry = entry?;
        let relative = entry.path().strip_prefix(system).expect("a walked entry lies under its root").to_owned();
        if relative.as_os_str().is_empty() {
            continue;
        }
        if skipped(&relative) {
            if entry.file_type().is_dir() {
                walk.skip_current_dir();
            }
            continue;
        }
        let parent = relative.parent().unwrap_or(Path::new(""));
        let lower = match shared.contains(parent) {
            true => present(&base.join(&relative))?,
            false => None,
        };
        let metadata = entry.metadata()?;
        let kind = entry.file_type();
        let target = upper.join(&relative);
        let owner = match (&lower, created.get(parent)) {
            (Some(lower), _) => (lower.uid(), lower.gid()),
            (None, Some(&owner)) => owner,
            (None, None) => {
                let directory = fs_err::symlink_metadata(base.join(parent))?;
                (directory.uid(), directory.gid())
            }
        };
        if kind.is_dir() {
            match &lower {
                Some(lower) if lower.is_dir() => {
                    shared.insert(relative.clone());
                    if lower.mode() == metadata.mode() {
                        continue;
                    }
                }
                _ => {
                    created.insert(relative.clone(), owner);
                }
            }
            ancestors(base, upper, &relative)?;
            fs_err::create_dir(&target)?;
        } else if kind.is_symlink() {
            let link = fs_err::read_link(entry.path())?;
            if lower.as_ref().is_some_and(|lower| lower.is_symlink())
                && fs_err::read_link(base.join(&relative))? == link
            {
                continue;
            }
            ancestors(base, upper, &relative)?;
            fs_err::os::unix::fs::symlink(link, &target)?;
            fs_err::os::unix::fs::lchown(&target, Some(owner.0), Some(owner.1))?;
            continue;
        } else if kind.is_file() {
            if lower.as_ref().is_some_and(|lower| lower.is_file() && lower.mode() == metadata.mode())
                && same_contents(entry.path(), &base.join(&relative))?
            {
                continue;
            }
            ancestors(base, upper, &relative)?;
            fs_err::copy(entry.path(), &target)?;
        } else {
            continue;
        }
        fs_err::os::unix::fs::lchown(&target, Some(owner.0), Some(owner.1))?;
        // After the owner, since a change of owner clears the setuid and
        // setgid bits.
        fs_err::set_permissions(&target, metadata.permissions())?;
    }
    let mut walk = walkdir::WalkDir::new(base).follow_links(false).sort_by_file_name().into_iter();
    while let Some(entry) = walk.next() {
        let entry = entry?;
        let relative = entry.path().strip_prefix(base).expect("a walked entry lies under its root").to_owned();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let parent = relative.parent().unwrap_or(Path::new(""));
        if skipped(&relative) || !shared.contains(parent) {
            if entry.file_type().is_dir() {
                walk.skip_current_dir();
            }
            continue;
        }
        if shared.contains(&relative) || present(&system.join(&relative))?.is_some() {
            continue;
        }
        if entry.file_type().is_dir() {
            walk.skip_current_dir();
        }
        ancestors(base, upper, &relative)?;
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &upper.join(&relative),
            rustix::fs::FileType::CharacterDevice,
            rustix::fs::Mode::empty(),
            rustix::fs::makedev(0, 0),
        )?;
    }
    Ok(())
}

/// What is at `path`, its last component not followed; none when nothing
/// is, or when a component above it is no directory.
fn present(path: &Path) -> io::Result<Option<std::fs::Metadata>> {
    match fs_err::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Makes each directory above `relative` in `upper` that is missing, with
/// its mode and owner in `base`, where each is a directory both trees have.
fn ancestors(base: &Path, upper: &Path, relative: &Path) -> io::Result<()> {
    let mut path = PathBuf::new();
    for component in relative.parent().into_iter().flat_map(Path::components) {
        path.push(component);
        let target = upper.join(&path);
        if present(&target)?.is_some() {
            continue;
        }
        let lower = fs_err::symlink_metadata(base.join(&path))?;
        fs_err::create_dir(&target)?;
        fs_err::os::unix::fs::lchown(&target, Some(lower.uid()), Some(lower.gid()))?;
        fs_err::set_permissions(&target, lower.permissions())?;
    }
    Ok(())
}

/// Whether the regular files `left` and `right` hold the same bytes.
fn same_contents(left: &Path, right: &Path) -> io::Result<bool> {
    let (mut left, mut right) = (fs_err::File::open(left)?, fs_err::File::open(right)?);
    if left.metadata()?.len() != right.metadata()?.len() {
        return Ok(false);
    }
    let (mut ours, mut theirs) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let read = left.read(&mut ours)?;
        if read == 0 {
            return Ok(true);
        }
        right.read_exact(&mut theirs[..read])?;
        if ours[..read] != theirs[..read] {
            return Ok(false);
        }
    }
}

/// The bytes the entries of the tree at `root` take on disk, links not
/// followed.
pub fn used_bytes(_: &OffLoop, root: &Path) -> io::Result<u64> {
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let blocks = entry?.metadata()?.blocks();
        total = total.saturating_add(blocks.saturating_mul(512));
    }
    Ok(total)
}
