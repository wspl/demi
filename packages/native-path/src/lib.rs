//! Shell path resolution shared by native commands and utility adapters.

use std::path::{Path, PathBuf};

/// Resolve against the invocation directory without changing process state.
pub fn resolve(path: impl AsRef<Path>, cwd: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    #[cfg(windows)]
    let mapped = path.to_str().and_then(windows_drive_path);
    #[cfg(windows)]
    let path = mapped.as_deref().map(Path::new).unwrap_or(path);
    if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.as_ref().join(path)
    }
}

#[cfg(any(windows, test))]
fn windows_drive_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && (bytes.len() == 2 || bytes[2] == b'/')
    {
        Some(format!(
            "{}:/{}",
            bytes[1] as char,
            path.get(3..).unwrap_or("")
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_mapping_preserves_native_and_relative_paths() {
        assert_eq!(windows_drive_path("/c/users/a"), Some("c:/users/a".into()));
        assert_eq!(windows_drive_path("/D"), Some("D:/".into()));
        for path in ["relative", "C:\\users", "//server/share", "/tmp", "/"] {
            assert_eq!(windows_drive_path(path), None);
        }
    }
}
