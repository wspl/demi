//! Release publication (`builds-and-releases.md` § Packaging): a directory of
//! verified files and the record that describes them, published once. Every
//! file is copied into a stage beside the directory and checked against its
//! declared size and SHA-256 as it is copied, and the stage becomes the
//! directory in one rename. A release is immutable: one already in place is
//! accepted only when its record and every file are the ones being
//! published. However publication ends, its stage is gone.

use std::path::{Component, Path, PathBuf};

use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use crate::{Digest, Error};

/// One file of a release: where its bytes are read from, its path inside
/// the release directory, the size and SHA-256 it must have, and whether it
/// is a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFile {
    pub source: PathBuf,
    pub path: PathBuf,
    pub digest: Digest,
    pub executable: bool,
}

/// The file that describes a release, such as a package descriptor: its
/// name in the release directory and its bytes.
#[derive(Debug, Clone, Copy)]
pub struct ReleaseRecord<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

/// Publishes the release of `record` and `files` at `directory`. Once
/// `cancel` fires, nothing is published.
pub async fn publish_release(
    directory: &Path,
    record: ReleaseRecord<'_>,
    files: &[ReleaseFile],
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let invalid = |reason: &str| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, reason.to_owned()));
    let mut record_name = Path::new(record.name).components();
    if !matches!((record_name.next(), record_name.next()), (Some(Component::Normal(_)), None)) {
        return Err(invalid("a release record is one file name"));
    }
    let inside = |path: &Path| {
        path.components().next().is_some()
            && path.components().all(|component| matches!(component, Component::Normal(_)))
    };
    if !files.iter().all(|file| inside(&file.path)) {
        return Err(invalid("a release file's path stays inside its release"));
    }
    let (parent, name) = match (directory.parent(), directory.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => (parent, name),
        _ => return Err(invalid("a release directory needs a parent directory")),
    };
    tokio::fs::create_dir_all(parent).await?;
    let prefix = format!(".{}-stage-", name.to_string_lossy());
    let stage = {
        let parent = parent.to_owned();
        tokio::task::spawn_blocking(move || tempfile::Builder::new().prefix(&prefix).tempdir_in(parent))
            .await
            .map_err(std::io::Error::other)??
    };
    let mut directories = vec![stage.path().to_owned()];
    for file in files {
        let destination = stage.path().join(&file.path);
        let mut ancestor = destination.parent();
        while let Some(directory) = ancestor.filter(|directory| *directory != stage.path()) {
            directories.push(directory.to_owned());
            ancestor = directory.parent();
        }
        stage_file(file, &destination, cancel).await?;
    }
    let mut written = tokio::fs::File::create(stage.path().join(record.name)).await?;
    written.write_all(record.bytes).await?;
    written.flush().await?;
    written.sync_all().await?;
    drop(written);
    // Deepest first, so each directory's entries are on disk before its
    // parent's; equal paths end up side by side for `dedup`.
    directories.sort_by(|a, b| {
        let depth = b.components().count().cmp(&a.components().count());
        depth.then_with(|| a.cmp(b))
    });
    directories.dedup();
    for directory in &directories {
        sync_directory(directory).await?;
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    match tokio::fs::rename(stage.path(), directory).await {
        Ok(()) => {
            // The stage is the release now; there is nothing left to remove.
            let _published = stage.keep();
            sync_directory(parent).await
        }
        // A directory in place fails the rename, with an error that differs
        // by platform; it is the release only when it holds the same bytes.
        Err(error) => match tokio::fs::metadata(directory).await {
            Ok(metadata) if metadata.is_dir() => in_place(directory, record, files, cancel).await,
            _ => Err(error.into()),
        },
    }
}

/// Copies `file` to `destination`, checking its bytes as they are copied.
async fn stage_file(file: &ReleaseFile, destination: &Path, cancel: &CancellationToken) -> Result<(), Error> {
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut input = tokio::fs::File::open(&file.source).await?;
    let mut output = tokio::fs::File::create(destination).await?;
    crate::copy(&mut input, &file.digest, &mut output, cancel).await?;
    output.sync_all().await?;
    #[cfg(unix)]
    if file.executable {
        use std::os::unix::fs::PermissionsExt;
        // A program is runnable whatever the umask.
        tokio::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o755)).await?;
    }
    Ok(())
}

/// Accepts the release at `directory` when its record and files are the
/// ones being published; anything else there is a conflict.
async fn in_place(
    directory: &Path,
    record: ReleaseRecord<'_>,
    files: &[ReleaseFile],
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let path = directory.join(record.name);
    match tokio::fs::read(&path).await {
        Ok(bytes) if bytes == record.bytes => {}
        Ok(_) => return Err(Error::Conflict(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(Error::Conflict(path)),
        Err(error) => return Err(error.into()),
    }
    for file in files {
        let path = directory.join(&file.path);
        match crate::digest(&path, file.digest.size, cancel).await {
            Ok(found) if found == file.digest => {}
            Ok(_) | Err(Error::TooLarge { .. }) => return Err(Error::Conflict(path)),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::Conflict(path));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Puts a directory's entries on disk.
#[cfg(unix)]
async fn sync_directory(path: &Path) -> Result<(), Error> {
    tokio::fs::File::open(path).await?.sync_all().await?;
    Ok(())
}

/// The standard library cannot open a directory on Windows, so there a
/// directory's entries reach the disk when the system writes them.
#[cfg(not(unix))]
async fn sync_directory(_: &Path) -> Result<(), Error> {
    Ok(())
}
