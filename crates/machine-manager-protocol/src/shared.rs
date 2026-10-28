//! An integration's shared folder: one directory every Cloud of the
//! integration mounts, where the integration's settings say
//! (`crates/backend-user-shard/src/integrations.rs`).
//!
//! A mount path is written as the person sets it, `~/shared` or `/data`, and
//! the manager mounts it at the absolute path [`mount_path`] answers. A path
//! over a directory every base has is refused: mounted there, the folder
//! would hide what a Cloud needs to run.

/// The home of a sandbox's user, `demi`, where `~` points.
pub const SANDBOX_HOME: &str = "/home/demi";

/// The top-level directories of every base, which a shared folder never
/// covers.
const BASE_DIRECTORIES: [&str; 21] = [
    "bin", "boot", "dev", "etc", "home", "lib", "lib32", "lib64", "libx32", "media", "mnt", "opt",
    "proc", "root", "run", "sbin", "srv", "sys", "tmp", "usr", "var",
];

/// The absolute path in a sandbox at which a shared folder set to `path`
/// is mounted: a path in the home, `~/...`, or one under a top-level
/// directory no base has, such as `/data`. Answers why `path` is no such
/// path.
pub fn mount_path(path: &str) -> Result<String, String> {
    let (root, rest) = if let Some(rest) = path.strip_prefix("~/") {
        (SANDBOX_HOME, rest)
    } else if let Some(rest) = path.strip_prefix('/') {
        let top = rest.split('/').next().unwrap_or_default();
        if BASE_DIRECTORIES.contains(&top) {
            return Err(format!("/{top} holds what a Cloud needs to run; choose a path in ~ or a new top-level directory such as /data"));
        }
        ("", rest)
    } else {
        return Err("a shared folder's path starts with ~/ or /".to_owned());
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return Err("a shared folder's path names a directory, not ~ or / itself".to_owned());
    }
    if rest.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err("a shared folder's path has no empty, . or .. parts".to_owned());
    }
    if rest.chars().any(|character| character.is_control()) {
        return Err("a shared folder's path has no control characters".to_owned());
    }
    Ok(format!("{root}/{rest}"))
}
