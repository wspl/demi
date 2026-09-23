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

/// A file being published: what the caller writes to it stays out of sight
/// at its path until `publish`, and dropping it unpublished removes it.
pub struct Staged {
    file: tokio::fs::File,
    temporary: tempfile::TempPath,
    path: PathBuf,
    publication: Publication,
}

impl Staged {
    /// A temporary file beside `path` with the publication's permissions.
    pub async fn new(path: &Path, publication: Publication) -> Result<Self, Error> {
        let (file, temporary) = stage(path, publication).await?;
        Ok(Self {
            file: tokio::fs::File::from_std(file),
            temporary,
            path: path.to_owned(),
            publication,
        })
    }

    /// The staged file, for the caller to fill.
    pub fn file(&mut self) -> &mut tokio::fs::File {
        &mut self.file
    }

    /// Makes the staged bytes the file at the path.
    pub async fn publish(self) -> Result<(), Error> {
        finish(self.file, self.temporary, &self.path, self.publication).await
    }
}

/// Publishes what `input` yields at `path`. Once `cancel` fires, nothing is
/// published: the rename happens only if it had not fired by then.
pub async fn publish(
    path: &Path,
    input: &mut (impl AsyncRead + Unpin),
    publication: Publication,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let mut staged = Staged::new(path, publication).await?;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            count = input.read(&mut buffer) => count?,
        };
        if count == 0 {
            break;
        }
        staged.file().write_all(&buffer[..count]).await?;
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    staged.publish().await
}

/// Publishes `bytes` at `path`.
pub async fn publish_bytes(path: &Path, bytes: &[u8], publication: Publication) -> Result<(), Error> {
    let path = path.to_owned();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || publish_bytes_blocking(&path, &bytes, publication))
        .await
        .map_err(std::io::Error::other)?
}

/// Publishes `bytes` at `path` on this thread, for work that already runs on
/// a blocking thread, such as a file mutation that must stay in one piece.
pub fn publish_bytes_blocking(path: &Path, bytes: &[u8], publication: Publication) -> Result<(), Error> {
    use std::io::Write as _;
    let (mut file, temporary) = stage_blocking(path, publication)?;
    file.write_all(bytes)?;
    if publication.durable {
        file.sync_all()?;
    }
    drop(file);
    persist(temporary, path, publication.mode)
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
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || stage_blocking(&path, publication))
        .await
        .map_err(std::io::Error::other)?
}

fn stage_blocking(
    path: &Path,
    publication: Publication,
) -> Result<(std::fs::File, tempfile::TempPath), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a file needs a parent directory"))?;
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
    let temporary = builder.tempfile_in(parent)?;
    if publication.permissions == Permissions::Keep {
        let permissions = std::fs::metadata(path)?.permissions();
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
    tokio::task::spawn_blocking(move || persist(temporary, &path, publication.mode))
        .await
        .map_err(std::io::Error::other)?
}

/// Renames the staged file to `path`.
fn persist(temporary: tempfile::TempPath, path: &Path, mode: Mode) -> Result<(), Error> {
    let persisted = match mode {
        Mode::CreateNew => temporary.persist_noclobber(path),
        Mode::Replace => temporary.persist(path),
    };
    persisted.map_err(|error| Error::Io(error.error))
}
