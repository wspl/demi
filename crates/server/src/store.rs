//! The machine manager's store on a filesystem that shares extents.
//!
//! The manager clones a Cloud's disk images, a wake's working copy and a
//! checkpoint's save, by sharing their extents where the filesystem can
//! (`crates/machine-manager/src/storage/clone.rs`), so a copy takes room only
//! as either side is written. A host whose own filesystem cannot, such as
//! ext4, gets the store in an XFS filesystem with `reflink=1` in a sparse
//! file beside it, `<store>.xfs`:
//!
//! - The file is as large as the host's filesystem and takes only what XFS
//!   holds; the manager's large writes check the host's room
//!   (`crates/machine-manager/src/storage/room.rs`).
//! - Its line in `/etc/fstab` mounts it from a loop device with `discard`, so
//!   what XFS frees goes back to the host, and `nofail`, so a failure never
//!   holds the machine's boot. The line also orders the manager's service
//!   after the mount and makes it require it, so no manager of any release
//!   starts on the directory beneath; and the line belongs to no release, so
//!   a rollback keeps it.
//! - A store that is a filesystem of its own, such as a disk mounted there,
//!   stays as it is.
//!
//! Setup makes it before the manager first runs ([`make`]). An update moves an
//! existing store in after its switch, while the manager is stopped and every
//! Cloud saved, so no working copy exists ([`move_in`]):
//!
//! ```text
//! <store>.xfs.formatting  made, formatted, then renamed:
//! <store>.xfs.partial     mounted at <store>.moving
//!   each base, file and directory:    copied, synced, original removed
//!   each Cloud's latest save:         copied, sparse
//!   each Cloud's previous save:       cloned from the latest, then the 4 KiB
//!                                     blocks in which the two differ written
//! fstab line, then <store>.xfs.partial -> <store>.xfs, mounted at <store>
//! ```
//!
//! While the partial file exists, the store's directory holds only what the
//! move has not taken, and the manager refuses to start; the update's next
//! run goes on with the move from where it stopped, a machine's restart
//! included.

use std::{
    fs,
    io::{self, Read as _},
    os::unix::fs::{FileExt as _, MetadataExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    process::Command,
};

use demi_machine_manager_protocol::{
    ROOM_RESERVE,
    store::{appended, mounted_from, partial_file, store_file},
};

use crate::{layout::MANAGER, moving::run};

/// How a store came to share extents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// It shares extents already, is in its XFS file already, or is a
    /// filesystem of its own, which stays.
    Already,
    /// It moved into its XFS file, this many bytes.
    Moved { bytes: u64 },
}

/// Makes the new store `store` share extents: nothing when its filesystem
/// can or it is a filesystem of its own, else its XFS file, mounted. A store
/// that holds anything already, as a setup that continues may find, stays
/// as it is, for an update to move in while no manager runs on it.
pub fn make(store: &Path) -> io::Result<()> {
    fs::create_dir_all(store)?;
    let store = fs::canonicalize(store)?;
    let file = store_file(&store);
    if !file.exists() {
        let holds_anything = fs::read_dir(&store)?.next().is_some();
        if holds_anything || own_filesystem(&store)? || shares_extents(&store)? {
            return Ok(());
        }
        format(&file, host_size(&store)?)?;
    }
    add_to_fstab(&file, &store)?;
    mount(&store, &file)
}

/// Requires what a move of the store `store` runs, before anything stops:
/// nothing for a store that needs no move.
pub fn require_tools(store: &Path) -> io::Result<()> {
    if !store.exists() || store_file(store).exists() || partial_file(store).exists() {
        return Ok(());
    }
    if own_filesystem(store)? || shares_extents(store)? {
        return Ok(());
    }
    for tool in ["mkfs.xfs", "cp", "mount", "umount"] {
        if which::which(tool).is_err() {
            return Err(io::Error::other(format!(
                "this release moves the Cloud store onto XFS, and the machine lacks {tool}: install xfsprogs"
            )));
        }
    }
    Ok(())
}

/// Whether the store `store` is where [`move_in`] would leave it: in its XFS
/// file, mounted, or on a filesystem that needs none. An update keeps the
/// running Clouds only over a settled store, since a move needs every Cloud
/// saved.
pub fn settled(store: &Path) -> io::Result<bool> {
    let store = fs::canonicalize(store)?;
    let file = store_file(&store);
    if file.exists() {
        return mounted_from(&store, &file);
    }
    Ok(!partial_file(&store).exists() && (own_filesystem(&store)? || shares_extents(&store)?))
}

/// Moves the store `store`, which no manager runs on, into its XFS file
/// unless it needs no move; `progress` hears the bytes moved of all left.
pub fn move_in(store: &Path, progress: &mut dyn FnMut(u64, u64)) -> io::Result<Moved> {
    let store = fs::canonicalize(store)?;
    let file = store_file(&store);
    let partial = partial_file(&store);
    if file.exists() {
        // A move that renamed its file, and stopped before it mounted it.
        add_to_fstab(&file, &store)?;
        mount(&store, &file)?;
        return Ok(Moved::Already);
    }
    if !partial.exists() {
        if own_filesystem(&store)? || shares_extents(&store)? {
            return Ok(Moved::Already);
        }
        make_room(&store)?;
        format(&partial, host_size(&store)?)?;
    }
    let staging = appended(&store, ".moving");
    fs::create_dir_all(&staging)?;
    if !mounted_from(&staging, &partial)? {
        run(Command::new("mount").args(["-t", "xfs", "-o", "loop,discard"]).arg(&partial).arg(&staging))?;
    }
    let total = allocated(&store)?;
    let mut done = 0;
    let moved = move_tree(&store, &store, &staging, false, &mut |bytes| {
        done += bytes;
        progress(done, total);
    });
    // A move that failed stays mounted for the next run to go on with.
    moved?;
    run(Command::new("umount").arg(&staging))?;
    fs::remove_dir(&staging)?;
    // The line first: a stop after the rename finds the file and its line.
    add_to_fstab(&file, &store)?;
    fs::rename(&partial, &file)?;
    sync_directory(store.parent().unwrap_or(Path::new("/")))?;
    mount(&store, &file)?;
    Ok(Moved::Moved { bytes: total })
}

/// Whether `store` is a filesystem of its own: another device than its
/// parent's, such as a disk mounted there.
fn own_filesystem(store: &Path) -> io::Result<bool> {
    let parent = store.parent().unwrap_or(Path::new("/"));
    Ok(fs::metadata(store)?.dev() != fs::metadata(parent)?.dev())
}

/// Whether two files in `directory` can share extents: a clone of a small
/// file there, removed again.
fn shares_extents(directory: &Path) -> io::Result<bool> {
    let source = tempfile::NamedTempFile::new_in(directory)?;
    source.as_file().write_all_at(&[1; 4096], 0)?;
    source.as_file().sync_all()?;
    let clone = tempfile::NamedTempFile::new_in(directory)?;
    // A server is Linux; elsewhere nothing shares extents this way.
    #[cfg(target_os = "linux")]
    return Ok(rustix::fs::ioctl_ficlone(clone.as_file(), source.as_file()).is_ok());
    #[cfg(not(target_os = "linux"))]
    {
        drop((clone, source));
        Ok(false)
    }
}

/// The size of the host's filesystem under `store`, which the XFS file
/// takes as its own: a sparse file holds only what is written to it.
fn host_size(store: &Path) -> io::Result<u64> {
    let stats = rustix::fs::statvfs(store)?;
    Ok(stats.f_blocks.saturating_mul(stats.f_frsize))
}

/// Makes `file`, sparse, `bytes` large, an XFS filesystem with reflink: in a
/// file of its own first, renamed to `file` once formatted, so a file of
/// that name is always a whole filesystem.
fn format(file: &Path, bytes: u64) -> io::Result<()> {
    let formatting = appended(file, ".formatting");
    match fs::remove_file(&formatting) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let made = fs::OpenOptions::new().write(true).create_new(true).open(&formatting)?;
    fs::set_permissions(&formatting, fs::Permissions::from_mode(0o600))?;
    made.set_len(bytes)?;
    run(Command::new("mkfs.xfs").args(["-q", "-m", "reflink=1", "-L", "demi-cloud"]).arg(&formatting))?;
    made.sync_all()?;
    fs::rename(&formatting, file)?;
    sync_directory(file.parent().unwrap_or(Path::new("/")))
}

/// Adds the line that mounts `file` at `store` to `/etc/fstab`, unless it
/// has one for `store`, keeping every other byte of the file as it was.
fn add_to_fstab(file: &Path, store: &Path) -> io::Result<()> {
    const FSTAB: &str = "/etc/fstab";
    let existing = match fs::read(FSTAB) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let point = escape(store);
    let has = existing.split(|byte| *byte == b'\n').any(|line| {
        let line = String::from_utf8_lossy(line);
        !line.trim_start().starts_with('#') && line.split_whitespace().nth(1) == Some(point.as_str())
    });
    if has {
        return Ok(());
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with(b"\n") {
        text.push(b'\n');
    }
    text.extend_from_slice(b"# Demi's Cloud store, which shares extents (demi-server)\n");
    text.extend_from_slice(
        format!(
            "{} {point} xfs loop,discard,nofail,x-systemd.before={MANAGER},x-systemd.required-by={MANAGER} 0 0\n",
            escape(file)
        )
        .as_bytes(),
    );
    let staged = PathBuf::from(format!("{FSTAB}.demi"));
    let mode = fs::metadata(FSTAB).map(|metadata| metadata.permissions()).unwrap_or(fs::Permissions::from_mode(0o644));
    fs::write(&staged, &text)?;
    fs::set_permissions(&staged, mode)?;
    fs::File::open(&staged)?.sync_all()?;
    fs::rename(&staged, FSTAB)?;
    sync_directory(Path::new("/etc"))?;
    run(Command::new("systemctl").arg("daemon-reload"))
}

/// A path as `/etc/fstab` writes it: a space, tab or backslash in octal.
fn escape(path: &Path) -> String {
    let mut text = String::new();
    for character in path.to_string_lossy().chars() {
        match character {
            ' ' => text.push_str("\\040"),
            '\t' => text.push_str("\\011"),
            '\\' => text.push_str("\\134"),
            other => text.push(other),
        }
    }
    text
}

/// Mounts the store from its XFS file, unless it is mounted from it.
fn mount(store: &Path, file: &Path) -> io::Result<()> {
    if mounted_from(store, file)? {
        return Ok(());
    }
    run(Command::new("mount").arg(store))
}

fn sync_directory(directory: &Path) -> io::Result<()> {
    fs::File::open(directory)?.sync_all()
}

/// Makes room for copying `source`, `bytes` large, to `target`, before it is
/// copied: whatever an earlier run copied of it goes first, then the host
/// must hold `bytes` with the reserve to spare. While it does not, the
/// largest previous save goes; answers whether `source` is still there to
/// copy, since it may have been that save.
fn room_for(store: &Path, source: &Path, target: &Path, bytes: u64) -> io::Result<bool> {
    if fs::symlink_metadata(target).is_ok() {
        remove(target)?;
    }
    loop {
        let stats = rustix::fs::statvfs(store)?;
        let free = stats.f_bavail.saturating_mul(stats.f_frsize);
        let needed = bytes.saturating_add(ROOM_RESERVE);
        if free >= needed {
            return Ok(true);
        }
        let Some(previous) = previous_saves(store)?.into_iter().max_by_key(|(_, bytes)| *bytes) else {
            return Err(io::Error::other(format!(
                "Not enough disk space to move the Cloud storage: needs {:.1} GB more",
                (needed - free) as f64 / 1e9
            )));
        };
        println!("Removing the previous save {} to make room for the move", previous.0.display());
        fs::remove_dir_all(&previous.0)?;
        if !source.exists() {
            return Ok(false);
        }
    }
}

/// Makes room for the move, which holds one item twice at a time, its
/// original and its copy, on the host's filesystem the store is on: the
/// largest item with the reserve to spare. While the host lacks it, the
/// largest previous save goes, a Cloud's fallback; when none is left, the
/// move is refused.
fn make_room(store: &Path) -> io::Result<()> {
    loop {
        let largest = largest_item(store)?;
        let stats = rustix::fs::statvfs(store)?;
        let free = stats.f_bavail.saturating_mul(stats.f_frsize);
        let needed = largest.saturating_add(ROOM_RESERVE);
        if free >= needed {
            return Ok(());
        }
        let Some(previous) = previous_saves(store)?.into_iter().max_by_key(|(_, bytes)| *bytes) else {
            return Err(io::Error::other(format!(
                "Not enough disk space to move the Cloud storage: needs {:.1} GB more",
                (needed - free) as f64 / 1e9
            )));
        };
        println!("Removing the previous save {} to make room for the move", previous.0.display());
        fs::remove_dir_all(&previous.0)?;
    }
}

/// The bytes of the largest item a move copies at once: a base, a save or
/// any other entry of the store.
fn largest_item(store: &Path) -> io::Result<u64> {
    let mut largest = 0;
    for item in items(store)? {
        largest = largest.max(allocated(&item)?);
    }
    Ok(largest)
}

/// What a move copies one at a time: each base, each save, and every other
/// entry of the store as a whole.
fn items(store: &Path) -> io::Result<Vec<PathBuf>> {
    let mut items = Vec::new();
    for entry in fs::read_dir(store)? {
        let path = entry?.path();
        if path.file_name() != Some("images".as_ref()) {
            items.push(path);
            continue;
        }
        for image in fs::read_dir(&path)? {
            let image = image?.path();
            let generations = image.join("generations");
            if image.file_name() == Some("bases".as_ref()) {
                items.extend(children(&image)?);
            } else if generations.is_dir() {
                items.extend(children(&generations)?);
            } else {
                items.push(image);
            }
        }
    }
    Ok(items)
}

fn children(directory: &Path) -> io::Result<Vec<PathBuf>> {
    fs::read_dir(directory)?.map(|entry| entry.map(|entry| entry.path())).collect()
}

/// Each Cloud's saves other than its committed one, with their bytes.
fn previous_saves(store: &Path) -> io::Result<Vec<(PathBuf, u64)>> {
    let images = store.join("images");
    let mut saves = Vec::new();
    if !images.is_dir() {
        return Ok(saves);
    }
    for device in children(&images)? {
        let generations = device.join("generations");
        if !generations.is_dir() {
            continue;
        }
        let Some(current) = committed(&device)? else {
            continue;
        };
        for save in children(&generations)? {
            if save.file_name() != Some(current.as_ref()) {
                let bytes = allocated(&save)?;
                saves.push((save, bytes));
            }
        }
    }
    Ok(saves)
}

/// The generation a device's `current.json` names; none for a directory
/// without one.
fn committed(device: &Path) -> io::Result<Option<String>> {
    let record = match fs::read(device.join("current.json")) {
        Ok(record) => record,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let value: serde_json::Value = serde_json::from_slice(&record).map_err(io::Error::other)?;
    Ok(value["generation"].as_str().map(str::to_owned))
}

/// Moves what is left of `from` into `to`, entry by entry: each copied with
/// its owners, modes, times and extended attributes, synced, then removed,
/// so a move that stops goes on with what is left. A Cloud's directory moves
/// as [`move_device`] says. The store's own directory stays, the mount point
/// of its XFS file.
fn move_tree(store: &Path, from: &Path, to: &Path, nested: bool, moved: &mut dyn FnMut(u64)) -> io::Result<()> {
    let times = times_of(from)?;
    mirror_directory(from, to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() && source.join("generations").is_dir() {
            move_device(store, &source, &target, moved)?;
        } else if kind.is_dir() && (entry.file_name() == "images" || entry.file_name() == "bases") {
            move_tree(store, &source, &target, true, moved)?;
        } else {
            let bytes = allocated(&source)?;
            room_for(store, &source, &target, bytes)?;
            copy(&source, &target)?;
            remove(&source)?;
            moved(bytes);
        }
    }
    set_times(to, &times)?;
    if nested {
        fs::remove_dir(from)?;
    }
    Ok(())
}

/// Moves one Cloud's directory: its committed save copied, unless an earlier
/// run of the move did; each other save cloned from it with the blocks in
/// which it differs; then the rest.
fn move_device(store: &Path, from: &Path, to: &Path, moved: &mut dyn FnMut(u64)) -> io::Result<()> {
    let times = times_of(from)?;
    mirror_directory(from, to)?;
    let generations = from.join("generations");
    let generations_times = times_of(&generations)?;
    let moved_generations = to.join("generations");
    mirror_directory(&generations, &moved_generations)?;
    let current = committed(from)?.or(committed(to)?);
    let mut saves = children(&generations)?;
    // The committed save first: the others are cloned from it.
    saves.sort_by_key(|save| save.file_name().and_then(|name| name.to_str()) != current.as_deref());
    let mut latest = current
        .as_ref()
        .map(|current| moved_generations.join(current))
        .filter(|moved| moved.is_dir() && !generations.join(moved.file_name().expect("a save has a name")).exists());
    for save in saves {
        let target = moved_generations.join(save.file_name().expect("a save has a name"));
        let bytes = allocated(&save)?;
        // A previous save that went to make room is no longer to move.
        if !room_for(store, &save, &target, bytes)? {
            continue;
        }
        match &latest {
            Some(latest) if *latest != target => clone_differing(latest, &save, &target)?,
            _ => copy(&save, &target)?,
        }
        if latest.is_none() {
            latest = Some(target);
        }
        remove(&save)?;
        moved(bytes);
    }
    set_times(&moved_generations, &generations_times)?;
    fs::remove_dir(&generations)?;
    for source in children(from)? {
        copy(&source, &to.join(source.file_name().expect("an entry has a name")))?;
        remove(&source)?;
    }
    set_times(to, &times)?;
    fs::remove_dir(from)
}

/// Makes the save `target` from the save `source`, given the save already
/// moved to `latest`: each image a clone of the latest's, with the 4 KiB
/// blocks in which `source`'s differ written over it, and a block that is a
/// hole or zeros in `source` punched; then the rest of `source`.
fn clone_differing(latest: &Path, source: &Path, target: &Path) -> io::Result<()> {
    const BLOCK: usize = 4096;
    mirror_directory(source, target)?;
    for path in children(source)? {
        let name = path.file_name().expect("an entry has a name");
        let destination = target.join(name);
        let shared = latest.join(name);
        if !name.to_string_lossy().ends_with(".ext4") || !shared.is_file() {
            copy(&path, &destination)?;
            continue;
        }
        let _ = fs::remove_file(&destination);
        run(Command::new("cp").args(["--reflink=always"]).arg(&shared).arg(&destination))?;
        let mut from = fs::File::open(&path)?;
        let written = fs::OpenOptions::new().read(true).write(true).open(&destination)?;
        let length = from.metadata()?.len();
        written.set_len(length)?;
        let (mut ours, mut theirs) = (vec![0; 1 << 20], vec![0; 1 << 20]);
        let mut position = 0u64;
        loop {
            let read = read_full(&mut from, &mut ours)?;
            if read == 0 {
                break;
            }
            let compared = written.read_at(&mut theirs[..read], position)?;
            for start in (0..read).step_by(BLOCK) {
                let end = (start + BLOCK).min(read);
                if end <= compared && ours[start..end] == theirs[start..end] {
                    continue;
                }
                let offset = position + start as u64;
                if ours[start..end].iter().all(|byte| *byte == 0) {
                    punch(&written, offset, (end - start) as u64)?;
                } else {
                    written.write_all_at(&ours[start..end], offset)?;
                }
            }
            position += read as u64;
        }
        written.sync_all()?;
        copy_attributes(&path, &destination)?;
    }
    set_times(target, &times_of(source)?)
}

/// Reads until `buffer` is full or the file ends.
fn read_full(file: &mut fs::File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = file.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// Makes `length` bytes of `file` at `offset` a hole, which reads as zeros
/// and takes no room.
fn punch(file: &fs::File, offset: u64, length: u64) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    return rustix::fs::fallocate(
        file,
        rustix::fs::FallocateFlags::PUNCH_HOLE | rustix::fs::FallocateFlags::KEEP_SIZE,
        offset,
        length,
    )
    .map_err(io::Error::from);
    #[cfg(not(target_os = "linux"))]
    file.write_all_at(&vec![0; length as usize], offset)
}

/// Copies `source`, a file or a directory, to `target` with its owners,
/// modes, times and extended attributes, sparse, and syncs the copy.
fn copy(source: &Path, target: &Path) -> io::Result<()> {
    if source.is_dir() && !source.is_symlink() {
        mirror_directory(source, target)?;
        let mut contents = source.as_os_str().to_owned();
        contents.push("/.");
        run(Command::new("cp").args(["-a", "--sparse=always"]).arg(contents).arg(target))?;
    } else {
        let _ = fs::remove_file(target);
        run(Command::new("cp").args(["-a", "--sparse=always"]).arg(source).arg(target))?;
    }
    run(Command::new("sync").arg("-f").arg(target))
}

/// Makes `to` a directory with `from`'s owners, mode and extended
/// attributes, unless it is one; its times follow once it is filled
/// ([`set_times`]).
fn mirror_directory(from: &Path, to: &Path) -> io::Result<()> {
    if !to.is_dir() {
        fs::create_dir(to)?;
    }
    copy_attributes(from, to)
}

/// Gives `to` the owners, mode and extended attributes, ACLs among them, of
/// `from`.
fn copy_attributes(from: &Path, to: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(from)?;
    rustix::fs::chown(to, Some(rustix::fs::Uid::from_raw(metadata.uid())), Some(rustix::fs::Gid::from_raw(metadata.gid())))?;
    fs::set_permissions(to, metadata.permissions())?;
    #[cfg(target_os = "linux")]
    {
        let mut names = vec![0u8; 64 * 1024];
        let length = rustix::fs::listxattr(from, &mut names)?;
        for name in names[..length].split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
            let name = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(name);
            let mut value = vec![0u8; 64 * 1024];
            let size = rustix::fs::getxattr(from, name, &mut value)?;
            rustix::fs::setxattr(to, name, &value[..size], rustix::fs::XattrFlags::empty())?;
        }
    }
    Ok(())
}

/// The access and modification times of `path`, read before anything in a
/// directory is removed, which changes them. A move that stopped midway read
/// them changed already, so its directories take the times of its stop.
fn times_of(path: &Path) -> io::Result<rustix::fs::Timestamps> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(rustix::fs::Timestamps {
        last_access: rustix::fs::Timespec { tv_sec: metadata.atime(), tv_nsec: metadata.atime_nsec() as _ },
        last_modification: rustix::fs::Timespec { tv_sec: metadata.mtime(), tv_nsec: metadata.mtime_nsec() as _ },
    })
}

/// Gives `path` the times `times`, once nothing is added to it any more.
fn set_times(path: &Path, times: &rustix::fs::Timestamps) -> io::Result<()> {
    rustix::fs::utimensat(rustix::fs::CWD, path, times, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)?;
    Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
}

/// The bytes `path` takes on the host, a directory's with everything in it.
fn allocated(path: &Path) -> io::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    let mut total = metadata.blocks().saturating_mul(512);
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            total = total.saturating_add(allocated(&entry?.path())?);
        }
    }
    Ok(total)
}
