//! Files only the runner's user may read: the state files the runner
//! replaces atomically, and the permission bits it sets on its directories.

use std::{
    io,
    path::{Path, PathBuf},
};

/// Replaces the state file at `path` with `bytes` and a final newline,
/// readable by the owner alone and on disk before the rename.
pub async fn write_private(path: PathBuf, mut bytes: Vec<u8>) -> io::Result<()> {
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    let publication = demi_artifact::Publication {
        mode: demi_artifact::Mode::Replace,
        permissions: demi_artifact::Permissions::Private,
        durable: true,
    };
    demi_artifact::publish_bytes(&path, &bytes, publication)
        .await
        .map_err(io_error)
}

/// An artifact failure as the IO error it is, or wraps.
pub fn io_error(error: demi_artifact::Error) -> io::Error {
    match error {
        demi_artifact::Error::Io(error) => error,
        error => io::Error::other(error),
    }
}

/// Sets `path`'s permission bits to `mode`; on Windows, where they do not
/// control access, it only checks that `path` exists.
pub async fn chmod(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await
    }
    #[cfg(windows)]
    {
        let _ = mode;
        // Unix executable and ownership bits do not control Windows execution.
        tokio::fs::metadata(path).await?;
        Ok(())
    }
}
