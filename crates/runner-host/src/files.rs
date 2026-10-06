//! File contents (`runner.md` § File contents): a file's bytes travel through
//! a pipe, never in a message. A read replies once the file is open and
//! positioned, before its first byte; a write replies once the file is in
//! place. A pipe that fails, because its reader went away or the connection
//! closed, ends the transfer and closes the file.

use std::{
    io::{self, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::Bytes;
use demi_shared_artifacts::{Mode, Permissions, Publication, Staged};
use futures_util::StreamExt;
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{Semaphore, mpsc},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use demi_command_sdk::paths::resolve;

use demi_runner_process::{
    pipes::{PipeClient, report_pipe},
    private_files::{chmod, io_error},
};
use demi_runner_protocol::wire::{self, Inbound};

use crate::{fs::error_code, git::GitService};

/// What one read of the file hands to the upload.
const CHUNK_BYTES: usize = 64 * 1024;

pub struct FileTransfers {
    output: mpsc::Sender<wire::Frame>,
    pipes: PipeClient,
    transfers: TaskTracker,
    /// Ends every transfer: cancelled by `close` and by the host connection's
    /// shutdown.
    cancel: CancellationToken,
}

impl FileTransfers {
    pub fn new(
        output: mpsc::Sender<wire::Frame>,
        pipes: PipeClient,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            output,
            pipes,
            transfers: TaskTracker::new(),
            cancel,
        }
    }

    /// `fs_readFile`: the byte range into the output pipe, after the reply;
    /// nothing when the file still has the version the request holds.
    pub fn read(&self, message: Inbound, default_cwd: &Path) -> io::Result<()> {
        let Inbound::FsReadFile {
            id,
            path,
            cwd,
            offset,
            length,
            version: held,
            output,
        } = message
        else {
            return Err(io::Error::other("not a file read"));
        };
        self.admit()?;
        let target = resolve(cwd.as_deref().map(Path::new).unwrap_or(default_cwd), &path);
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let (file, stat) = match open_range(target, offset.unwrap_or(0), length, &transfer).await {
                Ok(opened) => opened,
                Err(error) => {
                    // Nothing moves; the pipe end is still reported, as every
                    // end named to the runner is.
                    send(&reply, fs_error(id, &error), &shutdown).await;
                    report_pipe(&reply, output.id, Err(error), &shutdown).await;
                    return;
                }
            };
            let version = wire::file_version(stat.size, stat.mtime.0);
            let unchanged = held.as_deref() == Some(version.as_str());
            let opened = wire::encode(&wire::Outbound::FsOk(wire::FsOk {
                id,
                result: wire::FsResult::ReadFile(wire::OpenedFile {
                    stat,
                    version,
                    unchanged,
                }),
            }));
            if !send(&reply, opened, &shutdown).await {
                return;
            }
            if unchanged {
                // Nothing moves; the pipe end is still reported.
                let result = Err(io::Error::other("the file is unchanged"));
                report_pipe(&reply, output.id, result, &shutdown).await;
                return;
            }
            let result = pipes.put(&output.url, chunks(file), &transfer).await;
            report_pipe(&reply, output.id, result, &shutdown).await;
        });
        Ok(())
    }

    /// `fs_readFiles`: every file opened, answered at once, then their
    /// contents into the output pipe, each after its length.
    pub fn read_files(&self, message: Inbound, default_cwd: &Path) -> io::Result<()> {
        let Inbound::FsReadFiles {
            id,
            paths,
            cwd,
            limit,
            output,
        } = message
        else {
            return Err(io::Error::other("not a read of several files"));
        };
        self.admit()?;
        let base = cwd.map_or_else(|| default_cwd.to_owned(), PathBuf::from);
        let targets = paths.iter().map(|path| resolve(&base, path)).collect();
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let opened = open_files(targets, Some(limit), &transfer).await;
            let answer = wire::encode(&wire::Outbound::FsOk(wire::FsOk {
                id,
                result: wire::FsResult::ReadFiles(file_reads(&opened)),
            }));
            if !send(&reply, answer, &shutdown).await {
                return;
            }
            let result = pipes.put(&output.url, framed(opened), &transfer).await;
            report_pipe(&reply, output.id, result, &shutdown).await;
        });
        Ok(())
    }

    /// `fs_writeDirectory`: the input pipe's files into a temporary
    /// directory beside the destination, renamed into place once every one
    /// is written.
    pub fn write_directory(&self, message: Inbound, default_cwd: &Path) -> io::Result<()> {
        let Inbound::FsWriteDirectory {
            id,
            path,
            files,
            directory_mode,
            input,
        } = message
        else {
            return Err(io::Error::other("not a directory write"));
        };
        self.admit()?;
        let target = resolve(default_cwd, &path);
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let result = match target {
                Ok(target) => {
                    let written = DirectoryWrite {
                        target: &target,
                        files: &files,
                        directory_mode,
                    };
                    written.from_pipe(&pipes, &input.url, &transfer).await
                }
                Err(error) => Err(error),
            };
            let reported = result
                .as_ref()
                .map(|_| ())
                .map_err(|error| io::Error::new(error.kind(), error.to_string()));
            report_pipe(&reply, input.id, reported, &shutdown).await;
            let message = match result {
                Ok(()) => wire::encode(&wire::Outbound::FsOk(wire::FsOk {
                    id,
                    result: wire::FsResult::WriteDirectory,
                })),
                Err(error) => fs_error(id, &error),
            };
            send(&reply, message, &shutdown).await;
        });
        Ok(())
    }

    /// `fs_look`: what each path is, answered at once, then the files' first
    /// bytes into the output pipe, one file after another.
    pub fn look(&self, message: Inbound, default_cwd: &Path) -> io::Result<()> {
        let Inbound::FsLook {
            id,
            paths,
            cwd,
            output,
        } = message
        else {
            return Err(io::Error::other("not a look"));
        };
        self.admit()?;
        let base = cwd.map_or_else(|| default_cwd.to_owned(), PathBuf::from);
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let mut bytes = Vec::new();
            let mut looked = Vec::with_capacity(paths.len());
            for wanted in &paths {
                // Without a pipe, no file's bytes are read.
                let limit = if output.is_some() { wanted.limit } else { 0 };
                looked.push(look_at(&base, &wanted.path, limit, &mut bytes, &transfer).await);
            }
            let answer = wire::encode(&wire::Outbound::FsOk(wire::FsOk {
                id: id.clone(),
                result: wire::FsResult::Look(looked),
            }))
            .and_then(|answer| {
                wire::within_limit(answer, |reason| {
                    wire::encode(&wire::Outbound::FsError {
                        id,
                        code: Some("too_large".into()),
                        message: reason,
                    })
                })
            });
            if !send(&reply, answer, &shutdown).await {
                return;
            }
            if let Some(output) = output {
                let body = futures_util::stream::iter([Ok::<_, io::Error>(Bytes::from(bytes))]);
                let result = pipes.put(&output.url, body, &transfer).await;
                report_pipe(&reply, output.id, result, &shutdown).await;
            }
        });
        Ok(())
    }

    /// `fs_writeFile`: the input pipe into a temporary file beside the
    /// destination, renamed into place only when the pipe ended cleanly.
    pub fn write(&self, message: Inbound, default_cwd: &Path) -> io::Result<()> {
        let Inbound::FsWriteFile {
            id,
            path,
            cwd,
            exists,
            input,
        } = message
        else {
            return Err(io::Error::other("not a file write"));
        };
        self.admit()?;
        let target = resolve(cwd.as_deref().map(Path::new).unwrap_or(default_cwd), &path);
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let result = match target {
                Ok(target) => write_from_pipe(&pipes, &input.url, &target, exists, &transfer).await,
                Err(error) => Err(error),
            };
            let reported = result
                .as_ref()
                .map(|_| ())
                .map_err(|error| io::Error::new(error.kind(), error.to_string()));
            report_pipe(&reply, input.id, reported, &shutdown).await;
            let message = match result {
                Ok(name) => wire::encode(&wire::Outbound::FsOk(wire::FsOk {
                    id,
                    result: wire::FsResult::WriteFile(name),
                })),
                Err(error) => fs_error(id, &error),
            };
            send(&reply, message, &shutdown).await;
        });
        Ok(())
    }

    /// `git_show`: the file as the last commit has it, decoded whole under
    /// the working-tree admission `permit`, then streamed into the output
    /// pipe after the reply.
    pub fn show(
        &self,
        message: Inbound,
        default_cwd: &Path,
        git: GitService,
        capacity: Arc<Semaphore>,
    ) -> io::Result<()> {
        let Inbound::GitShow {
            id,
            root,
            path,
            output,
        } = message
        else {
            return Err(io::Error::other("not a git show"));
        };
        self.admit()?;
        let root = resolve(default_cwd, &root);
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            // Past the working-tree capacity a request waits for a slot
            // (`runner.md` § Load).
            let permit = tokio::select! {
                permit = capacity.acquire_owned() => permit.expect("working-tree capacity is never closed"),
                _ = transfer.cancelled() => return,
            };
            let decoded = match root {
                // Out of open files, the request waits for one (`runner.md` § Load).
                Ok(root) => {
                    demi_command_sdk::descriptors::retry(&transfer, || git.show(&root, &path, &transfer)).await
                }
                Err(error) => Err(crate::git::GitError::Io(error)),
            };
            // The decode is the admitted work; the upload is paced by the pipe.
            drop(permit);
            let bytes = match decoded {
                Ok(bytes) => bytes,
                Err(error) => {
                    let message = wire::encode(&wire::Outbound::GitError {
                        id,
                        code: error.code().to_owned(),
                        message: error.message(),
                    });
                    send(&reply, message, &shutdown).await;
                    let unused = io::Error::other(error.message());
                    report_pipe(&reply, output.id, Err(unused), &shutdown).await;
                    return;
                }
            };
            let found = wire::encode(&wire::Outbound::GitOk(wire::GitOk {
                id,
                result: wire::GitResult::Show,
            }));
            if !send(&reply, found, &shutdown).await {
                return;
            }
            let body = futures_util::stream::iter([Ok::<_, io::Error>(Bytes::from(bytes))]);
            let result = pipes.put(&output.url, body, &transfer).await;
            report_pipe(&reply, output.id, result, &shutdown).await;
        });
        Ok(())
    }

    pub async fn close(&self) {
        self.cancel.cancel();
        self.transfers.close();
        self.transfers.wait().await;
    }

    fn admit(&self) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        Ok(())
    }
}

/// `fs_hashFile`: the size and SHA-256 of the regular file at `target`,
/// read through and sent nowhere.
pub async fn hash(
    target: io::Result<PathBuf>,
    cancel: &CancellationToken,
) -> io::Result<wire::FileHash> {
    use sha2::{Digest, Sha256};
    let (mut file, _) = open_range(target, 0, None, cancel).await?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut chunk = vec![0; CHUNK_BYTES];
    loop {
        crate::fs::check_cancelled(cancel)?;
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
        size += read as u64;
    }
    wire::FileHash::new(size, format!("{:x}", hasher.finalize())).map_err(io::Error::other)
}

/// The regular file at `target`, positioned at `offset` and limited to
/// `length` bytes, or to the end it had when it was opened, with its
/// metadata then, which the read answers so that no `stat` precedes it
/// (`runner.md` § Host operations).
async fn open_range(
    target: io::Result<PathBuf>,
    offset: u64,
    length: Option<u64>,
    cancel: &CancellationToken,
) -> io::Result<(impl AsyncRead + Unpin + Send + 'static, wire::FileStat)> {
    let target = target?;
    // Out of open files, the transfer waits for one (`runner.md` § Load).
    let mut file = demi_command_sdk::descriptors::retry(cancel, || fs::File::open(&target)).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "not a regular file",
        ));
    }
    let rest = metadata.len().saturating_sub(offset);
    let stat = crate::fs::stat(metadata)?;
    file.seek(SeekFrom::Start(offset)).await?;
    Ok((file.take(length.map_or(rest, |length| length.min(rest))), stat))
}

/// A file of a read of several, open, and the size it had then.
pub struct OpenFile {
    file: fs::File,
    size: u64,
}

/// Opens each of `targets` for a read of several files: a regular file of
/// at most `limit` bytes, when there is a limit.
pub async fn open_files(
    targets: Vec<io::Result<PathBuf>>,
    limit: Option<u64>,
    cancel: &CancellationToken,
) -> Vec<io::Result<OpenFile>> {
    let mut opened = Vec::with_capacity(targets.len());
    for target in targets {
        opened.push(open_file(target, limit, cancel).await);
    }
    opened
}

async fn open_file(
    target: io::Result<PathBuf>,
    limit: Option<u64>,
    cancel: &CancellationToken,
) -> io::Result<OpenFile> {
    let target = target?;
    // Out of open files, the read waits for one (`runner.md` § Load).
    let file = demi_command_sdk::descriptors::retry(cancel, || fs::File::open(&target)).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "not a regular file",
        ));
    }
    if limit.is_some_and(|limit| metadata.len() > limit) {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            format!("the file is over {} bytes", limit.unwrap_or_default()),
        ));
    }
    Ok(OpenFile {
        file,
        size: metadata.len(),
    })
}

/// The answer of a read of several files: which were opened, and why each
/// other was not.
pub fn file_reads(opened: &[io::Result<OpenFile>]) -> Vec<wire::FileRead> {
    opened
        .iter()
        .map(|file| match file {
            Ok(_) => wire::FileRead::Read,
            Err(error) => wire::FileRead::Failed {
                code: match error.kind() {
                    io::ErrorKind::FileTooLarge => Some("too_large".into()),
                    _ => error_code(error).map(String::from),
                },
                message: error.to_string(),
            },
        })
        .collect()
}

/// The pipe body of a read of several files: each opened file's length,
/// eight bytes in big-endian order, then its bytes (`wire::FileRead`). A
/// file that got shorter since it was opened fails the body.
pub fn framed(
    opened: Vec<io::Result<OpenFile>>,
) -> impl futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static {
    futures_util::stream::iter(opened.into_iter().flatten()).flat_map(|OpenFile { file, size }| {
        let length = futures_util::stream::once(async move {
            Ok(Bytes::copy_from_slice(&size.to_be_bytes()))
        });
        length.chain(exactly(file, size))
    })
}

/// The first `size` bytes of `file`, which must have them.
fn exactly(
    file: fs::File,
    size: u64,
) -> impl futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static {
    let chunks = chunks(file.take(size));
    futures_util::stream::unfold(Some((chunks, 0)), move |state| async move {
        let (mut chunks, read) = state?;
        match chunks.next().await {
            Some(Ok(chunk)) => {
                let read = read + chunk.len() as u64;
                Some((Ok(chunk), Some((chunks, read))))
            }
            Some(Err(error)) => Some((Err(error), None)),
            None if read < size => Some((
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the file got shorter while it was read",
                )),
                None,
            )),
            None => None,
        }
    })
}

/// What is at `path`, a symbolic link followed. A file's first `limit`
/// bytes are added to `bytes`; a file it could not read adds none.
async fn look_at(
    base: &Path,
    path: &str,
    limit: u64,
    bytes: &mut Vec<u8>,
    cancel: &CancellationToken,
) -> wire::Looked {
    let unreadable = |error: io::Error| wire::Looked::Unreadable {
        code: error_code(&error).map(String::from),
        message: error.to_string(),
    };
    let target = match resolve(base, path) {
        Ok(target) => target,
        Err(error) => return unreadable(error),
    };
    let metadata = match fs::metadata(&target).await {
        Ok(metadata) => metadata,
        Err(error) if matches!(error_code(&error), Some("ENOENT" | "ENOTDIR")) => {
            return wire::Looked::Missing;
        }
        Err(error) => return unreadable(error),
    };
    if metadata.is_dir() {
        return match crate::fs::list(&target, cancel).await {
            Ok(entries) => wire::Looked::Directory { entries },
            Err(error) => unreadable(error),
        };
    }
    if !metadata.is_file() {
        return wire::Looked::Other;
    }
    let start = bytes.len();
    let read = async {
        // Out of open files, the look waits for one (`runner.md` § Load).
        let file = demi_command_sdk::descriptors::retry(cancel, || fs::File::open(&target)).await?;
        file.take(limit).read_to_end(bytes).await
    };
    match read.await {
        Ok(read) => wire::Looked::File {
            size: metadata.len(),
            read: read as u64,
        },
        Err(error) => {
            bytes.truncate(start);
            unreadable(error)
        }
    }
}

/// The upload body: one chunk per read, taken only when the upload asks for
/// it, so a slow reader of the pipe paces the file reads. A read error fails
/// the upload instead of ending it like EOF.
fn chunks(
    file: impl AsyncRead + Unpin + Send + 'static,
) -> impl futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static {
    tokio_util::io::ReaderStream::with_capacity(file, CHUNK_BYTES)
}

/// Streams the pipe into a file staged beside `target`, making the
/// directories above it that are missing, and publishes it there when the
/// pipe ends cleanly, as `exists` says when the path is taken; any failure
/// removes the staged file and leaves `target` as it was. Answers the name
/// of the file written.
async fn write_from_pipe(
    pipes: &PipeClient,
    url: &str,
    target: &Path,
    exists: wire::WriteExists,
    cancel: &CancellationToken,
) -> io::Result<String> {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a file needs a parent directory and a name",
        ));
    };
    let name = name.to_string_lossy().into_owned();
    fs::create_dir_all(parent).await?;
    // What is there now refuses the write before its bytes move; the
    // publication checks again.
    match fs::symlink_metadata(target).await {
        Ok(taken) if taken.is_dir() && exists != wire::WriteExists::Rename => {
            return Err(is_directory(target));
        }
        Ok(_) if exists == wire::WriteExists::Refuse => return Err(file_exists(target)),
        _ => {}
    }
    let publication = Publication {
        mode: match exists {
            wire::WriteExists::Replace => Mode::Replace,
            wire::WriteExists::Refuse | wire::WriteExists::Rename => Mode::CreateNew,
        },
        permissions: Permissions::Default,
        durable: false,
    };
    // Out of open files, the write waits for one (`runner.md` § Load).
    let mut staged =
        demi_command_sdk::descriptors::retry(cancel, || Staged::new(target, publication))
            .await
            .map_err(io_error)?;
    let mut body = pipes.get(url, cancel.clone()).await?;
    while let Some(chunk) = body.next().await {
        staged.file().write_all(&chunk?).await?;
    }
    match exists {
        wire::WriteExists::Replace => {
            staged.publish().await.map_err(io_error)?;
            Ok(name)
        }
        wire::WriteExists::Refuse => match staged.publish().await.map_err(io_error) {
            Ok(()) => Ok(name),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let directory = fs::symlink_metadata(target)
                    .await
                    .is_ok_and(|taken| taken.is_dir());
                Err(if directory {
                    is_directory(target)
                } else {
                    file_exists(target)
                })
            }
            Err(error) => Err(error),
        },
        wire::WriteExists::Rename => {
            let directory = parent.to_owned();
            let candidates = std::iter::once(name.clone())
                .chain((2..).map(move |number| numbered(&name, number)))
                .map(move |name| directory.join(name));
            let written = staged
                .publish_first_free(candidates)
                .await
                .map_err(io_error)?;
            Ok(written
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default())
        }
    }
}

/// A directory an `fs_writeDirectory` writes.
struct DirectoryWrite<'a> {
    target: &'a Path,
    files: &'a [wire::DirectoryFile],
    directory_mode: u32,
}

impl DirectoryWrite<'_> {
    /// Writes the directory from the pipe into `.<name>.partial` beside it,
    /// which a write cut short leaves for the next one to remove, and
    /// renames it into place.
    async fn from_pipe(
        &self,
        pipes: &PipeClient,
        url: &str,
        cancel: &CancellationToken,
    ) -> io::Result<()> {
        let (Some(parent), Some(name)) = (self.target.parent(), self.target.file_name()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a directory needs a parent and a name",
            ));
        };
        fs::create_dir_all(parent).await?;
        let partial = parent.join(format!(".{}.partial", name.to_string_lossy()));
        crate::fs::remove_all(&partial, cancel).await?;
        fs::create_dir(&partial).await?;
        let written = self.fill(&partial, pipes, url, cancel).await;
        if let Err(error) = written {
            // The next write removes what this one leaves.
            if let Err(left) = crate::fs::remove_all(&partial, cancel).await {
                tracing::debug!("a directory write left {}: {left}", partial.display());
            }
            return Err(error);
        }
        // Renamed while it is writable, which some systems ask of a
        // directory that moves.
        match fs::rename(&partial, self.target).await {
            Ok(()) => {}
            // A directory there is complete: another write put it in place.
            Err(_) if fs::metadata(self.target).await.is_ok_and(|taken| taken.is_dir()) => {
                crate::fs::remove_all(&partial, cancel).await?;
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        chmod(self.target, self.directory_mode).await
    }

    /// Writes each file of the listing from the pipe, each after its length,
    /// and gives the directories inside `partial` their mode, deepest first.
    async fn fill(
        &self,
        partial: &Path,
        pipes: &PipeClient,
        url: &str,
        cancel: &CancellationToken,
    ) -> io::Result<()> {
        let body = pipes.get(url, cancel.clone()).await?;
        let mut contents = tokio_util::io::StreamReader::new(body);
        let mut directories = std::collections::BTreeSet::new();
        for file in self.files {
            let relative = inside(&file.path)?;
            let destination = partial.join(&relative);
            let mut above = relative.parent();
            while let Some(directory) = above.filter(|directory| !directory.as_os_str().is_empty()) {
                directories.insert(partial.join(directory));
                above = directory.parent();
            }
            if let Some(directory) = destination.parent() {
                fs::create_dir_all(directory).await?;
            }
            let mut length = [0; 8];
            contents.read_exact(&mut length).await?;
            let length = u64::from_be_bytes(length);
            let mut written = fs::File::create(&destination).await?;
            let copied = tokio::io::copy(&mut (&mut contents).take(length), &mut written).await?;
            if copied < length {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the directory's contents ended inside a file",
                ));
            }
            written.flush().await?;
            drop(written);
            chmod(&destination, file.mode).await?;
        }
        if contents.read(&mut [0]).await? > 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the directory's contents go on past its last file",
            ));
        }
        // Deepest first, so each is still writable while its entries change.
        for directory in directories.iter().rev() {
            chmod(directory, self.directory_mode).await?;
        }
        Ok(())
    }
}

/// A path inside a directory: relative, its parts plain names.
fn inside(path: &str) -> io::Result<PathBuf> {
    let relative = PathBuf::from(path);
    let plain = relative
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)));
    if path.is_empty() || !plain {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{path} is not a path inside the directory"),
        ));
    }
    Ok(relative)
}

/// `file_name` with `number` before its extension: `notes-2.txt`, and
/// `.env-2` for a name that is all extension.
fn numbered(file_name: &str, number: u32) -> String {
    let name = Path::new(file_name);
    match (name.file_stem(), name.extension()) {
        (Some(stem), Some(extension)) => format!(
            "{}-{number}.{}",
            stem.to_string_lossy(),
            extension.to_string_lossy()
        ),
        _ => format!("{file_name}-{number}"),
    }
}

fn is_directory(target: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::IsADirectory,
        format!("{} is a directory", target.display()),
    )
}

fn file_exists(target: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{} exists", target.display()),
    )
}

fn fs_error(id: String, error: &io::Error) -> Result<wire::Frame, wire::WireError> {
    wire::encode(&wire::Outbound::FsError {
        id,
        code: error_code(error).map(String::from),
        message: error.to_string(),
    })
}

/// Sends one reply unless the connection is shutting down; false when it
/// could not be sent.
async fn send(
    output: &mpsc::Sender<wire::Frame>,
    message: Result<wire::Frame, wire::WireError>,
    shutdown: &CancellationToken,
) -> bool {
    let message = match message {
        Ok(message) => message,
        Err(error) => {
            tracing::warn!("file transfer reply encoding failed: {error}");
            return false;
        }
    };
    tokio::select! {
        _ = shutdown.cancelled() => false,
        result = output.send(message) => result.is_ok(),
    }
}
