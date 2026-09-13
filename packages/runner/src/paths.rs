use std::{
    io,
    path::{Path, PathBuf},
};

/// Resolves shell paths against an explicit cwd, never process-global cwd.
pub fn resolve(path: &str, cwd: &Path) -> io::Result<PathBuf> {
    if path.is_empty() || path.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be nonempty and contain no NUL byte",
        ));
    }
    Ok(demi_native_path::resolve(path, cwd))
}
