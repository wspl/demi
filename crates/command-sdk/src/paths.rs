//! Paths an invocation names, resolved against its explicit working
//! directory, never the process's.

use std::{
    io,
    path::{Path, PathBuf},
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
