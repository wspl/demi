//! The paths of Git Bash that Demi's shell and its utilities read as Git's
//! utilities do on Windows, so a path one of them takes the other takes too:
//! `/tmp` is the user's temporary folder (`%TMP%`, else `%TEMP%`), and a drive
//! written `/c/…` is `C:\…`. Any other path is the system's own, `/usr` and
//! `/etc` among them, which Git reads under its own folder.
//!
//! This is the one place the mapping is made: the shell asks it through
//! `ExecutionHost::map_path`, for its words and its globs, and the utilities
//! through `uucore::context::Control::map_path`.

use std::path::{Path, PathBuf};

/// What `path` stands for, or none where it is the system's own.
#[cfg(windows)]
pub(crate) fn map(path: &Path, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let text = path.to_str()?;
    if let Some(rest) = text.strip_prefix("/tmp")
        && (rest.is_empty() || rest.starts_with('/'))
    {
        let temp = env("TMP")
            .or_else(|| env("TEMP"))
            .map_or_else(std::env::temp_dir, PathBuf::from);
        return Some(temp.join(rest.trim_start_matches('/')));
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && (bytes.len() == 2 || bytes[2] == b'/')
    {
        let drive = PathBuf::from(format!("{}:\\", char::from(bytes[1].to_ascii_uppercase())));
        return Some(drive.join(text[2..].trim_start_matches('/')));
    }
    None
}

#[cfg(not(windows))]
pub(crate) fn map(_path: &Path, _env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    None
}
