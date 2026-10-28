//! Where the manager's store is when the host's filesystem shares no
//! extents: an XFS filesystem in a file beside the store's directory, which
//! `demi-server` makes and mounts there (`crates/server/src/store.rs`) and
//! which the manager requires mounted (`crates/machine-manager/src/storage/room.rs`).

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

/// The XFS file of the store `data`: `<data>.xfs` beside it.
pub fn store_file(data: &Path) -> PathBuf {
    appended(data, ".xfs")
}

/// The XFS file a move of the store `data` fills before it is the store's:
/// `<data>.xfs.partial`. While it exists, the store's directory holds only
/// what the move has not taken yet, so no manager may run on it.
pub fn partial_file(data: &Path) -> PathBuf {
    appended(data, ".xfs.partial")
}

/// `path` with `suffix` after its last component, whatever that holds.
pub fn appended(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

/// The loop device and its backing file of the filesystem that holds
/// `path`, the mount with the longest point that contains it; none for a
/// filesystem on any other device.
pub fn loop_holding(path: &Path) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let path = std::fs::canonicalize(path)?;
    let holding = mounts()?
        .into_iter()
        .filter(|(point, _)| path.starts_with(point))
        .max_by_key(|(point, _)| point.as_os_str().len());
    backing(holding.map(|(_, source)| source))
}

/// Whether the filesystem mounted at `point` itself is the one in `file`.
pub fn mounted_from(point: &Path, file: &Path) -> io::Result<bool> {
    let point = std::fs::canonicalize(point)?;
    // The last mount at a point is the one on top.
    let source = mounts()?.into_iter().filter(|(at, _)| *at == point).last().map(|(_, source)| source);
    let Some((_, behind)) = backing(source)? else {
        return Ok(false);
    };
    Ok(std::fs::canonicalize(behind).ok() == std::fs::canonicalize(file).ok())
}

/// Each mount of this process's namespace: its point and its source.
fn mounts() -> io::Result<Vec<(PathBuf, String)>> {
    let mounts = match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(mounts) => mounts,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(mounts
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            let separator = fields.iter().position(|field| *field == "-")?;
            Some((unescape(fields.get(4)?), fields.get(separator + 2)?.to_string()))
        })
        .collect())
}

/// The loop device `source` names and the file behind it; none for any
/// other source.
fn backing(source: Option<String>) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let Some(device) = source.as_deref().and_then(|source| source.strip_prefix("/dev/")) else {
        return Ok(None);
    };
    if !device.starts_with("loop") {
        return Ok(None);
    }
    match std::fs::read_to_string(format!("/sys/block/{device}/loop/backing_file")) {
        Ok(file) => Ok(Some((PathBuf::from(format!("/dev/{device}")), PathBuf::from(file.trim_end())))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// A mount point as `mountinfo` writes it, with its space, tab, newline and
/// backslash escaped in octal.
fn unescape(field: &str) -> PathBuf {
    let mut bytes = Vec::with_capacity(field.len());
    let mut rest = field.as_bytes();
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'\\' && tail.len() >= 3 {
            if let Some(value) = std::str::from_utf8(&tail[..3]).ok().and_then(|digits| u8::from_str_radix(digits, 8).ok()) {
                bytes.push(value);
                rest = &tail[3..];
                continue;
            }
        }
        bytes.push(first);
        rest = tail;
    }
    #[cfg(unix)]
    return PathBuf::from(<OsString as std::os::unix::ffi::OsStringExt>::from_vec(bytes));
    #[cfg(not(unix))]
    return PathBuf::from(String::from_utf8_lossy(&bytes).into_owned());
}
