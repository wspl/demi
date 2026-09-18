//! File contents (`runner.md` § File contents): a file's bytes travel through
//! a pipe, never in a message. A read replies once the file is open and
//! positioned, before its first byte; a write replies once the file is in
//! place. A pipe that fails, because its reader went away or the connection
//! closed, ends the transfer and closes the file.

use std::{
    io::{self, SeekFrom},
    path::{Path, PathBuf},
};

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, mpsc},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    connection::wire::{self, Inbound},
    fs::error_code,
    git::GitService,
    paths::resolve,
    pipes::PipeClient,
    tasks::report_pipe,
};

/// What one read of the file hands to the upload.
const CHUNK_BYTES: usize = 64 * 1024;

pub struct FileTransfers {
    output: mpsc::Sender<wire::Outbound>,
    pipes: PipeClient,
    transfers: TaskTracker,
    /// Ends every transfer: cancelled by `close` and by the host connection's
    /// shutdown.
    cancel: CancellationToken,
}

impl FileTransfers {
    pub fn new(
        output: mpsc::Sender<wire::Outbound>,
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

    /// `fs_readFile`: the byte range into the output pipe, after the reply.
    pub fn read(&self, message: Inbound, default_cwd: &Path) -> io::Result<()> {
        let Inbound::FsReadFile {
            id,
            path,
            cwd,
            offset,
            length,
            output,
        } = message
        else {
            return Err(io::Error::other("not a file read"));
        };
        self.admit()?;
        let target = resolve(&path, cwd.as_deref().map(Path::new).unwrap_or(default_cwd));
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let file = match open_range(target, offset.unwrap_or(0), length).await {
                Ok(file) => file,
                Err(error) => {
                    // Nothing moves; the pipe end is still reported, as every
                    // end named to the runner is.
                    send(&reply, fs_error(id, &error), &shutdown).await;
                    report_pipe(&reply, output.id, Err(error), &shutdown).await;
                    return;
                }
            };
            if !send(&reply, wire::fs_ok_read_file(id, ()), &shutdown).await {
                return;
            }
            let result = pipes.put(&output.url, chunks(file), &transfer).await;
            report_pipe(&reply, output.id, result, &shutdown).await;
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
            create_parents,
            input,
        } = message
        else {
            return Err(io::Error::other("not a file write"));
        };
        self.admit()?;
        let target = resolve(&path, cwd.as_deref().map(Path::new).unwrap_or(default_cwd));
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let result = match target {
                Ok(target) => {
                    write_from_pipe(
                        &pipes,
                        &input.url,
                        &target,
                        create_parents == Some(true),
                        &transfer,
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            let reported = result
                .as_ref()
                .map(|_| ())
                .map_err(|error| io::Error::new(error.kind(), error.to_string()));
            report_pipe(&reply, input.id, reported, &shutdown).await;
            let message = match result {
                Ok(()) => wire::fs_ok_write_file(id, ()),
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
        permit: OwnedSemaphorePermit,
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
        let root = resolve(&root, default_cwd);
        let reply = self.output.clone();
        let pipes = self.pipes.clone();
        let transfer = self.cancel.child_token();
        let shutdown = self.cancel.clone();
        self.transfers.spawn(async move {
            let decoded = match root {
                Ok(root) => git.show(&root, &path, &transfer).await,
                Err(error) => Err(crate::git::GitError::Io(error)),
            };
            // The decode is the admitted work; the upload is paced by the pipe.
            drop(permit);
            let bytes = match decoded {
                Ok(bytes) => bytes,
                Err(error) => {
                    let message = wire::git_error(id, error.code().to_owned(), error.message());
                    send(&reply, message, &shutdown).await;
                    let unused = io::Error::other(error.message());
                    report_pipe(&reply, output.id, Err(unused), &shutdown).await;
                    return;
                }
            };
            if !send(&reply, wire::git_ok_show(id, ()), &shutdown).await {
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

/// The regular file at `target`, positioned at `offset` and limited to
/// `length` bytes when given.
async fn open_range(
    target: io::Result<PathBuf>,
    offset: u64,
    length: Option<u64>,
) -> io::Result<impl AsyncRead + Unpin + Send + 'static> {
    let mut file = fs::File::open(target?).await?;
    if !file.metadata().await?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "not a regular file",
        ));
    }
    file.seek(SeekFrom::Start(offset)).await?;
    Ok(file.take(length.unwrap_or(u64::MAX)))
}

/// The upload body: one chunk per read, taken only when the upload asks for
/// it, so a slow reader of the pipe paces the file reads.
fn chunks(
    file: impl AsyncRead + Unpin + Send + 'static,
) -> impl futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static {
    futures_util::stream::unfold(Some(file), |state| async move {
        let mut file = state?;
        let mut buffer = vec![0u8; CHUNK_BYTES];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(count) => {
                buffer.truncate(count);
                Some((Ok(Bytes::from(buffer)), Some(file)))
            }
            // A read error fails the upload instead of ending it like EOF.
            Err(error) => Some((Err(error), None)),
        }
    })
}

async fn write_from_pipe(
    pipes: &PipeClient,
    url: &str,
    target: &Path,
    create_parents: bool,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a file needs a parent directory",
        )
    })?;
    if create_parents {
        fs::create_dir_all(parent).await?;
    }
    // A fresh name beside the destination, so the rename stays on one volume
    // and the destination changes only once, whole.
    let temporary = parent.join(format!(".demi-write-{}", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await?;
    let written = async {
        let mut body = pipes.get(url, cancel.clone()).await?;
        while let Some(chunk) = body.next().await {
            file.write_all(&chunk?).await?;
        }
        file.flush().await?;
        drop(file);
        fs::rename(&temporary, target).await
    }
    .await;
    if written.is_err() {
        // The destination is untouched; the partial copy goes with the failure.
        let _ = fs::remove_file(&temporary).await;
    }
    written
}

fn fs_error(id: String, error: &io::Error) -> Result<wire::Outbound, wire::WireError> {
    wire::fs_error(id, error_code(error).map(String::from), error.to_string())
}

/// Sends one reply unless the connection is shutting down; false when it
/// could not be sent.
async fn send(
    output: &mpsc::Sender<wire::Outbound>,
    message: Result<wire::Outbound, wire::WireError>,
    shutdown: &CancellationToken,
) -> bool {
    let message = match message {
        Ok(message) => message,
        Err(error) => {
            eprintln!("demi-runner: file transfer reply encoding failed: {error}");
            return false;
        }
    };
    tokio::select! {
        _ = shutdown.cancelled() => false,
        result = output.send(message) => result.is_ok(),
    }
}
