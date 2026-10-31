//! Paths an invocation names, resolved against its explicit working
//! directory, never the process's.

use std::{
    io,
    path::{Component, Path, PathBuf},
};

/// `path` joined to `cwd`; a path that is empty or holds a NUL byte names
/// nothing and is refused.
pub fn resolve(cwd: impl AsRef<Path>, path: &str) -> io::Result<PathBuf> {
    if path.is_empty() || path.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be nonempty and contain no NUL byte",
        ));
    }
    Ok(cwd.as_ref().join(path))
}

/// The directory entry `path` names, whatever way it reaches it: `a.txt`
/// and `sub/../a.txt` are one entry, and a symbolic link is an entry of its
/// own, apart from the file it points to, as removing it shows. Its
/// directories are resolved where they exist, and the rest, as a change
/// makes them, by their names, so `new/../a.txt` is `a.txt` before `new`
/// exists. A path that ends with a separator names a directory and is no
/// other path's entry.
pub fn entry(path: &Path) -> PathBuf {
    if names_directory(path) {
        return path.to_path_buf();
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => directory(parent).join(name),
        _ => path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
    }
}

/// The file whose content `path` reaches: its entry, or what it points to
/// when it is a symbolic link, so that two changes that reach one file
/// through a link and its target are known as one.
pub fn file(path: &Path) -> PathBuf {
    if names_directory(path) {
        return path.to_path_buf();
    }
    match path.canonicalize() {
        Ok(resolved) => resolved,
        Err(_) => entry(path),
    }
}

/// Whether `path` ends with a separator, which asks for a directory: some
/// systems resolve `a.txt/` to the file, but reading it fails.
fn names_directory(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().ends_with(['/', std::path::MAIN_SEPARATOR])
}

/// `path` as a directory: its nearest existing ancestor resolved, then the
/// rest of it by name, a `..` taking off the name before it.
fn directory(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut existing = path;
    let base = loop {
        if let Ok(resolved) = existing.canonicalize() {
            break resolved;
        }
        match (existing.parent(), existing.components().next_back()) {
            (Some(parent), Some(last)) => {
                missing.push(last);
                existing = parent;
            }
            _ => break existing.to_path_buf(),
        }
    };
    let mut resolved = base;
    for component in missing.into_iter().rev() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    resolved
}
