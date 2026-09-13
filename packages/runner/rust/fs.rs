use std::{
    io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use demi_runner_protocol::{self as wire, Inbound, Outbound, Timestamp, WireBytes};
use futures_util::future::BoxFuture;
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::paths::resolve;

/// Returns None for messages owned by other runner subsystems.
pub async fn handle(
    message: &Inbound,
    default_cwd: &Path,
    cancel: &CancellationToken,
) -> Option<Result<Outbound, wire::WireError>> {
    let id = message.fs_request_id()?;
    Some(match call(message, default_cwd, cancel).await {
        Ok(reply) => Ok(reply),
        Err(error) => wire::fs_error(
            id.to_owned(),
            error_code(&error).map(String::from),
            error.to_string(),
        ),
    })
}

async fn call(
    message: &Inbound,
    default_cwd: &Path,
    cancel: &CancellationToken,
) -> io::Result<Outbound> {
    use Inbound::*;
    check_cancelled(cancel)?;
    let path = |path: &str, cwd: &Option<String>| {
        resolve(path, cwd.as_deref().map(Path::new).unwrap_or(default_cwd))
    };
    let encoded = match message {
        FsReadFile {
            id,
            path: value,
            cwd,
        } => wire::fs_ok_read_file(id.clone(), WireBytes(fs::read(path(value, cwd)?).await?)),
        FsWriteFile {
            id,
            path: value,
            cwd,
            data,
            create_parents,
        } => {
            let target = path(value, cwd)?;
            create_parent(&target, *create_parents).await?;
            fs::write(target, &data.0).await?;
            wire::fs_ok_write_file(id.clone(), ())
        }
        FsAppendFile {
            id,
            path: value,
            cwd,
            data,
            create_parents,
        } => {
            let target = path(value, cwd)?;
            create_parent(&target, *create_parents).await?;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(target)
                .await?;
            file.write_all(&data.0).await?;
            file.flush().await?;
            wire::fs_ok_append_file(id.clone(), ())
        }
        FsExists {
            id,
            path: value,
            cwd,
        } => {
            let exists = match fs::symlink_metadata(path(value, cwd)?).await {
                Ok(_) => true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            };
            wire::fs_ok_exists(id.clone(), exists)
        }
        FsStat {
            id,
            path: value,
            cwd,
        } => wire::fs_ok_stat(id.clone(), stat(fs::metadata(path(value, cwd)?).await?)?),
        FsLstat {
            id,
            path: value,
            cwd,
        } => wire::fs_ok_lstat(
            id.clone(),
            stat(fs::symlink_metadata(path(value, cwd)?).await?)?,
        ),
        FsReaddir {
            id,
            path: value,
            cwd,
            with_file_types,
        } => {
            let mut directory = fs::read_dir(path(value, cwd)?).await?;
            let mut entries = Vec::new();
            while let Some(entry) = directory.next_entry().await? {
                check_cancelled(cancel)?;
                let kind = entry.file_type().await?;
                entries.push(wire::FsOkReaddirResultVariant1Item {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    is_file: kind.is_file(),
                    is_directory: kind.is_dir(),
                    is_symbolic_link: kind.is_symlink(),
                });
            }
            let result = if *with_file_types == Some(true) {
                wire::FsOkReaddirResult::Variant1(entries)
            } else {
                wire::FsOkReaddirResult::Variant0(
                    entries.into_iter().map(|entry| entry.name).collect(),
                )
            };
            wire::fs_ok_readdir(id.clone(), result)
        }
        FsMkdir {
            id,
            path: value,
            cwd,
            recursive,
        } => {
            let target = path(value, cwd)?;
            if *recursive == Some(true) {
                fs::create_dir_all(target).await?;
            } else {
                fs::create_dir(target).await?;
            }
            wire::fs_ok_mkdir(id.clone(), ())
        }
        FsRm {
            id,
            path: value,
            cwd,
            recursive,
            force,
        } => {
            match remove(&path(value, cwd)?, *recursive == Some(true)).await {
                Err(error) if *force == Some(true) && error.kind() == io::ErrorKind::NotFound => {}
                result => result?,
            }
            wire::fs_ok_rm(id.clone(), ())
        }
        FsCp {
            id,
            path: value,
            cwd,
            destination,
            recursive,
        } => {
            let source = path(value, cwd)?;
            let destination = path(destination, cwd)?;
            copy(&source, &destination, *recursive == Some(true), cancel).await?;
            wire::fs_ok_cp(id.clone(), ())
        }
        FsMv {
            id,
            path: value,
            cwd,
            destination,
        } => {
            let source = path(value, cwd)?;
            let destination = path(destination, cwd)?;
            match fs::rename(&source, &destination).await {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                    copy(&source, &destination, true, cancel).await?;
                    remove(&source, true).await?;
                }
                Err(error) => return Err(error),
            }
            wire::fs_ok_mv(id.clone(), ())
        }
        FsChmod {
            id,
            path: value,
            cwd,
            mode,
        } => {
            chmod(&path(value, cwd)?, *mode as u32).await?;
            wire::fs_ok_chmod(id.clone(), ())
        }
        FsSymlink {
            id,
            path: value,
            cwd,
            target,
        } => {
            symlink(Path::new(target), &path(value, cwd)?).await?;
            wire::fs_ok_symlink(id.clone(), ())
        }
        FsLink {
            id,
            path: value,
            cwd,
            existing_path,
        } => {
            fs::hard_link(path(existing_path, cwd)?, path(value, cwd)?).await?;
            wire::fs_ok_link(id.clone(), ())
        }
        FsReadlink {
            id,
            path: value,
            cwd,
        } => wire::fs_ok_readlink(
            id.clone(),
            fs::read_link(path(value, cwd)?)
                .await?
                .to_string_lossy()
                .into_owned(),
        ),
        FsRealpath {
            id,
            path: value,
            cwd,
        } => wire::fs_ok_realpath(
            id.clone(),
            fs::canonicalize(path(value, cwd)?)
                .await?
                .to_string_lossy()
                .into_owned(),
        ),
        FsUtimes {
            id,
            path: value,
            cwd,
            atime,
            mtime,
        } => {
            let target = path(value, cwd)?;
            let atime = file_time(*atime);
            let mtime = file_time(*mtime);
            tokio::task::spawn_blocking(move || filetime::set_file_times(target, atime, mtime))
                .await
                .map_err(io::Error::other)??;
            wire::fs_ok_utimes(id.clone(), ())
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a filesystem request",
            ));
        }
    };
    encoded.map_err(io::Error::other)
}

async fn create_parent(path: &Path, enabled: Option<bool>) -> io::Result<()> {
    if enabled == Some(true)
        && let Some(parent) = path.parent()
    {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
}

pub async fn chmod(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await
    }
    #[cfg(windows)]
    {
        let _ = mode;
        // Unix executable and ownership bits do not control Windows execution.
        fs::metadata(path).await?;
        Ok(())
    }
}

async fn remove(path: &Path, recursive: bool) -> io::Result<()> {
    if fs::symlink_metadata(path).await?.is_dir() {
        if !recursive {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "recursive removal is required for a directory",
            ));
        }
        fs::remove_dir_all(path).await
    } else {
        fs::remove_file(path).await
    }
}

pub async fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::symlink(target, link).await
    }
    #[cfg(windows)]
    {
        let resolved = if target.is_absolute() {
            target.to_owned()
        } else {
            link.parent().unwrap_or(Path::new(".")).join(target)
        };
        if fs::metadata(resolved)
            .await
            .is_ok_and(|metadata| metadata.is_dir())
        {
            fs::symlink_dir(target, link).await
        } else {
            fs::symlink_file(target, link).await
        }
    }
}

async fn copy(
    source: &Path,
    destination: &Path,
    recursive: bool,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source).await?;
    if metadata.is_dir() {
        if !recursive {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "recursive copy is required for a directory",
            ));
        }
        let canonical_source = fs::canonicalize(source).await?;
        let canonical_destination = canonical_destination(destination).await?;
        if canonical_destination.starts_with(&canonical_source) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot copy a directory into itself",
            ));
        }
    }
    copy_entry(source, destination, cancel).await
}

async fn canonical_destination(path: &Path) -> io::Result<PathBuf> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(ancestor).await {
            Ok(mut base) => {
                for part in suffix.into_iter().rev() {
                    base.push(part);
                }
                return Ok(base);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                suffix.push(ancestor.file_name().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid destination")
                })?);
                ancestor = ancestor.parent().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid destination")
                })?;
            }
            Err(error) => return Err(error),
        }
    }
}

fn copy_entry<'a>(
    source: &'a Path,
    destination: &'a Path,
    cancel: &'a CancellationToken,
) -> BoxFuture<'a, io::Result<()>> {
    Box::pin(async move {
        check_cancelled(cancel)?;
        let metadata = fs::symlink_metadata(source).await?;
        if metadata.is_symlink() {
            return symlink(&fs::read_link(source).await?, destination).await;
        }
        if metadata.is_file() {
            fs::copy(source, destination).await?;
            return Ok(());
        }
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot copy a special file",
            ));
        }
        fs::create_dir_all(destination).await?;
        let mut entries = fs::read_dir(source).await?;
        while let Some(entry) = entries.next_entry().await? {
            copy_entry(&entry.path(), &destination.join(entry.file_name()), cancel).await?;
        }
        fs::set_permissions(destination, metadata.permissions()).await?;
        Ok(())
    })
}

fn stat(metadata: std::fs::Metadata) -> io::Result<wire::FsOkStatResult> {
    let mtime = match metadata.modified()?.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).map_err(io::Error::other)?,
        Err(error) => -i64::try_from(error.duration().as_millis()).map_err(io::Error::other)?,
    };
    let mut result = wire::FsOkStatResult {
        is_file: metadata.is_file(),
        is_directory: metadata.is_dir(),
        is_symbolic_link: metadata.is_symlink(),
        mode: 0.,
        size: metadata.len() as f64,
        mtime: Timestamp(mtime),
        uid: None,
        gid: None,
        ino: None,
        dev: None,
        nlink: None,
        is_character_device: None,
        is_fifo: None,
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        result.mode = metadata.mode().into();
        result.uid = Some(metadata.uid().into());
        result.gid = Some(metadata.gid().into());
        result.ino = Some(metadata.ino() as f64);
        result.dev = Some(metadata.dev() as f64);
        result.nlink = Some(metadata.nlink() as f64);
        result.is_character_device = Some(metadata.file_type().is_char_device());
        result.is_fifo = Some(metadata.file_type().is_fifo());
    }
    #[cfg(windows)]
    {
        let kind = if metadata.is_dir() {
            0o040000
        } else if metadata.is_symlink() {
            0o120000
        } else {
            0o100000
        };
        let permissions = if metadata.permissions().readonly() {
            0o444
        } else {
            0o666
        };
        result.mode = (kind | permissions | if metadata.is_dir() { 0o111 } else { 0 }) as f64;
    }
    Ok(result)
}

fn file_time(timestamp: Timestamp) -> filetime::FileTime {
    filetime::FileTime::from_unix_time(
        timestamp.0.div_euclid(1000),
        (timestamp.0.rem_euclid(1000) * 1_000_000) as u32,
    )
}

fn check_cancelled(cancel: &CancellationToken) -> io::Result<()> {
    if cancel.is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "filesystem request cancelled",
        ))
    } else {
        Ok(())
    }
}

pub fn error_code(error: &io::Error) -> Option<&'static str> {
    use io::ErrorKind::*;
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::ELOOP) {
        return Some("ELOOP");
    }
    Some(match error.kind() {
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        AlreadyExists => "EEXIST",
        NotADirectory => "ENOTDIR",
        IsADirectory => "EISDIR",
        DirectoryNotEmpty => "ENOTEMPTY",
        InvalidInput | InvalidData => "EINVAL",
        CrossesDevices => "EXDEV",
        Interrupted => "EINTR",
        ReadOnlyFilesystem => "EROFS",
        StorageFull => "ENOSPC",
        TooManyLinks => "EMLINK",
        BrokenPipe => "EPIPE",
        _ => return None,
    })
}
