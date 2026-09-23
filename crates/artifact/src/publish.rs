//! Atomic publication: bytes go to a temporary file beside their path and
//! are renamed into place, so a reader sees the old file or the whole new
//! one, never part of it.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::Error;

/// Whether a publication may replace what is at its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Fails with `AlreadyExists` when the path exists.
    CreateNew,
    Replace,
}

/// The published file's permissions, on Unix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permissions {
    /// Read and write for everyone, less the process's umask: a new file's
    /// usual permissions.
    Default,
    /// Read and write for the owner only.
    Private,
    /// Read, write and run for the owner, read and run for everyone.
    Executable,
    /// The permissions of the file being replaced.
    Keep,
}

/// How a file is published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Publication {
    pub mode: Mode,
    pub permissions: Permissions,
    /// Whether the bytes are on disk before the rename, so a crash leaves the
    /// old file or the whole new one.
    pub durable: bool,
}

/// Publishes what `input` yields at `path`.
pub async fn publish(
    path: &Path,
    input: &mut (impl AsyncRead + Unpin),
    publication: Publication,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let (file, temporary) = stage(path, publication).await?;
    let mut file = tokio::fs::File::from_std(file);
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            count = input.read(&mut buffer) => count?,
        };
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count]).await?;
    }
    finish(file, temporary, path, publication).await
}

/// Publishes `bytes` at `path`.
pub async fn publish_bytes(path: &Path, bytes: &[u8], publication: Publication) -> Result<(), Error> {
    let (file, temporary) = stage(path, publication).await?;
    let mut file = tokio::fs::File::from_std(file);
    file.write_all(bytes).await?;
    finish(file, temporary, path, publication).await
}

/// Moves the directory `staged` to `destination`, replacing a directory
/// there. Both are on one volume, as a staging directory beside the
/// destination is.
pub async fn publish_directory(staged: &Path, destination: &Path) -> Result<(), Error> {
    match tokio::fs::remove_dir_all(destination).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    tokio::fs::rename(staged, destination).await?;
    Ok(())
}

/// A temporary file beside `path` with the publication's permissions.
async fn stage(
    path: &Path,
    publication: Publication,
) -> Result<(std::fs::File, tempfile::TempPath), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a file needs a parent directory"))?
        .to_owned();
    let replaced = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // The mode applies when the file is made, less the umask.
            let mode = match publication.permissions {
                Permissions::Default => 0o666,
                Permissions::Private => 0o600,
                Permissions::Executable => 0o755,
                Permissions::Keep => 0o600,
            };
            builder.permissions(std::fs::Permissions::from_mode(mode));
        }
        let temporary = builder.tempfile_in(&parent)?;
        if publication.permissions == Permissions::Keep {
            let permissions = std::fs::metadata(&replaced)?.permissions();
            temporary.as_file().set_permissions(permissions)?;
        }
        #[cfg(unix)]
        if publication.permissions == Permissions::Executable {
            use std::os::unix::fs::PermissionsExt;
            // An executable is runnable whatever the umask.
            temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(temporary.into_parts())
    })
    .await
    .map_err(std::io::Error::other)?
}

async fn finish(
    mut file: tokio::fs::File,
    temporary: tempfile::TempPath,
    path: &Path,
    publication: Publication,
) -> Result<(), Error> {
    file.flush().await?;
    if publication.durable {
        file.sync_all().await?;
    }
    drop(file);
    let path: PathBuf = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let persisted = match publication.mode {
            Mode::CreateNew => temporary.persist_noclobber(&path),
            Mode::Replace => temporary.persist(&path),
        };
        persisted.map_err(|error| Error::Io(error.error))
    })
    .await
    .map_err(std::io::Error::other)?
}
