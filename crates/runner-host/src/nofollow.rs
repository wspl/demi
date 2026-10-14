//! A look that follows no link below a directory (`runner.md` § Host
//! operations, `subagents.md` § Workflows of a project): each component
//! from that directory down is looked at without following it and opened
//! only when it is no link, so a link swapped in between a listing and a
//! read is still not followed. On Unix each step opens the next component
//! relative to the one before with `O_NOFOLLOW`; elsewhere each component's
//! own metadata is read before the path is opened, which a swap at that
//! moment could still pass.

use std::{
    io::{self, Read},
    path::{Component, Path},
};

use demi_runner_protocol::wire;

/// What `path`, below `within`, is, without following a link from
/// `within` down: a file's first `limit` bytes go into `bytes`.
pub fn look_within(within: &Path, path: &Path, limit: u64, bytes: &mut Vec<u8>) -> io::Result<wire::Looked> {
    let rest = path.strip_prefix(within).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not under {}", path.display(), within.display()),
        )
    })?;
    let mut names = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(name) => names.push(name.to_owned()),
            Component::CurDir => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} names a component a look within a directory takes no part of", path.display()),
                ));
            }
        }
    }
    platform::look(within, &names, limit, bytes)
}

/// An entry of a listing, from its own metadata.
#[cfg(not(unix))]
fn entry(name: String, metadata: &std::fs::Metadata) -> io::Result<wire::DirEntry> {
    let kind = metadata.file_type();
    Ok(wire::DirEntry {
        name,
        is_file: kind.is_file(),
        is_directory: kind.is_dir(),
        is_symbolic_link: kind.is_symlink(),
        size: metadata.len(),
        mtime: crate::fs::modified(metadata)?,
    })
}

/// A file's first `limit` bytes into `bytes`, and its size.
fn read_file(mut file: std::fs::File, limit: u64, bytes: &mut Vec<u8>) -> io::Result<wire::Looked> {
    let size = file.metadata()?.len();
    let start = bytes.len();
    let read = match (&mut file).take(limit).read_to_end(bytes) {
        Ok(read) => read,
        Err(error) => {
            bytes.truncate(start);
            return Err(error);
        }
    };
    Ok(wire::Looked::File { size, read: read as u64 })
}

#[cfg(unix)]
mod platform {
    use std::{ffi::OsString, fs::File, io, os::fd::OwnedFd, path::Path};

    use demi_runner_protocol::wire;
    use rustix::fs::{AtFlags, FileType, Mode, OFlags};
    use rustix::io::Errno;

    pub(super) fn look(within: &Path, names: &[OsString], limit: u64, bytes: &mut Vec<u8>) -> io::Result<wire::Looked> {
        let directory = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let mut current: OwnedFd = rustix::fs::open(within, directory, Mode::empty())?;
        let Some((last, between)) = names.split_last() else {
            return list(current);
        };
        for name in between {
            match rustix::fs::openat(&current, name.as_os_str(), directory | OFlags::NOFOLLOW, Mode::empty()) {
                Ok(next) => current = next,
                // A link there, or a file where a directory should be.
                Err(Errno::LOOP) => return Ok(wire::Looked::Link),
                Err(Errno::NOTDIR) => return Ok(link_or(&current, name, wire::Looked::Missing)),
                Err(Errno::NOENT) => return Ok(wire::Looked::Missing),
                Err(error) => return Err(error.into()),
            }
        }
        let stat = match rustix::fs::statat(&current, last.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT | Errno::NOTDIR) => return Ok(wire::Looked::Missing),
            Err(error) => return Err(error.into()),
        };
        let flags = match FileType::from_raw_mode(stat.st_mode) {
            FileType::Symlink => return Ok(wire::Looked::Link),
            FileType::Directory => directory | OFlags::NOFOLLOW,
            FileType::RegularFile => OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            _ => return Ok(wire::Looked::Other),
        };
        let opened = match rustix::fs::openat(&current, last.as_os_str(), flags, Mode::empty()) {
            Ok(opened) => opened,
            // Swapped for a link since its metadata was read.
            Err(Errno::LOOP) => return Ok(wire::Looked::Link),
            Err(Errno::NOTDIR) => return Ok(link_or(&current, last, wire::Looked::Other)),
            Err(Errno::NOENT) => return Ok(wire::Looked::Missing),
            Err(error) => return Err(error.into()),
        };
        // What was opened is what was looked at, or the look says what it is now.
        match FileType::from_raw_mode(rustix::fs::fstat(&opened)?.st_mode) {
            FileType::Directory => list(opened),
            FileType::RegularFile => super::read_file(File::from(opened), limit, bytes),
            _ => Ok(wire::Looked::Other),
        }
    }

    /// `O_NOFOLLOW` with `O_DIRECTORY` answers `ENOTDIR` for a link on some
    /// systems: what `name` is then decides.
    fn link_or(directory: &OwnedFd, name: &OsString, otherwise: wire::Looked) -> wire::Looked {
        match rustix::fs::statat(directory, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink => wire::Looked::Link,
            _ => otherwise,
        }
    }

    /// The entries of the open directory, each looked at without following it.
    fn list(directory: OwnedFd) -> io::Result<wire::Looked> {
        let mut entries = Vec::new();
        let mut dir = rustix::fs::Dir::read_from(&directory)?;
        while let Some(entry) = dir.read() {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "." || name == ".." {
                continue;
            }
            match rustix::fs::statat(&directory, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => {
                    let kind = FileType::from_raw_mode(stat.st_mode);
                    // The nanoseconds field is signed on some targets and
                    // unsigned on others (Linux's raw stat); it is always
                    // below one second, so it fits either way.
                    let nanoseconds = i64::try_from(stat.st_mtime_nsec).unwrap_or(0);
                    let milliseconds = i64::from(stat.st_mtime) * 1000 + nanoseconds / 1_000_000;
                    entries.push(wire::DirEntry {
                        name,
                        is_file: kind == FileType::RegularFile,
                        is_directory: kind == FileType::Directory,
                        is_symbolic_link: kind == FileType::Symlink,
                        size: u64::try_from(stat.st_size).unwrap_or(0),
                        mtime: demi_runner_protocol::wire::Timestamp(milliseconds),
                    });
                }
                // Removed since the directory named it: no longer listed.
                Err(Errno::NOENT) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(wire::Looked::Directory { entries })
    }
}

#[cfg(not(unix))]
mod platform {
    use std::{ffi::OsString, io, path::Path};

    use demi_runner_protocol::wire;

    pub(super) fn look(within: &Path, names: &[OsString], limit: u64, bytes: &mut Vec<u8>) -> io::Result<wire::Looked> {
        let mut path = within.to_owned();
        let mut metadata = std::fs::metadata(within)?;
        for name in names {
            path.push(name);
            metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(wire::Looked::Missing),
                Err(error) => return Err(error),
            };
            // A symbolic link and a junction are both links here.
            if metadata.file_type().is_symlink() {
                return Ok(wire::Looked::Link);
            }
        }
        if metadata.is_dir() {
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                match std::fs::symlink_metadata(entry.path()) {
                    Ok(metadata) => entries.push(super::entry(entry.file_name().to_string_lossy().into_owned(), &metadata)?),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            return Ok(wire::Looked::Directory { entries });
        }
        if !metadata.is_file() {
            return Ok(wire::Looked::Other);
        }
        super::read_file(std::fs::File::open(&path)?, limit, bytes)
    }
}
