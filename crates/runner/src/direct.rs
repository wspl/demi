//! A connection's direct channels (`direct-channel.md`): the runner carries
//! out each channel's operation as it carries out the backend's request for
//! the same thing (`runner.md` § Host operations), with the same functions,
//! limits, atomic writes and file watches, and opens a `stream` as a
//! `service_open`, with a `user` caller in the conversation the header
//! names.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use demi_command_protocol::{CommandCaller, CommandContext, EDIT_FILE_BYTES, TextRefusal, text_of};
use demi_command_sdk::{InputSource, paths::resolve};
use demi_runner_direct::{
    Answer, ByteStream, FileRange, FileText, Listing, Operations, Scope, StreamRequest,
    WatchStream,
};
use demi_runner_host::{
    files::{open_read, write_file},
    fs,
    watches::{Watched, Watches},
};
use demi_runner_jobs::commands::streams::{StreamOpener, StreamRefusal, StreamStart};
use demi_runner_protocol::{
    direct::{ChannelError, ChannelErrorCode},
    files::{DirectoryEntry, FileWatchMessage, FsFailure, PageWatch, WatchReport, protected_path},
    wire::{self, FsResult, Inbound, ServiceErrorCode},
};
use futures_util::StreamExt;
use tokio::{runtime::Handle, sync::mpsc};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

/// What a stream's invocation may send ahead of the page taking it.
const STREAM_OUTPUT: usize = 4;

/// The Host operations and service streams of one backend connection, which
/// its peers' channels carry out. `Send` and `Sync`: the peers run on a
/// thread of their own.
pub struct HostOperations {
    pub home: String,
    pub watches: Watches,
    pub streams: StreamOpener,
    /// The connection's runtime, where a stream's invocation runs as the
    /// backend's do.
    pub control: Handle,
    /// The connection's end, which ends every operation.
    pub closed: CancellationToken,
}

impl Operations for HostOperations {
    fn read(&self, scope: Scope, path: String, offset: u64, length: Option<u64>) -> Answer<FileRange> {
        let cancel = self.closed.clone();
        Box::pin(async move {
            let target = resolve(&scope.cwd, &path);
            let read = open_read(target, offset, length, None, &cancel)
                .await
                .map_err(|error| match error.kind() {
                    // A path that is not a regular file is no file to read.
                    io::ErrorKind::IsADirectory => {
                        ChannelError::new(ChannelErrorCode::FsError, 404, "Not a regular file")
                    }
                    _ => failed(error),
                })?;
            Ok(FileRange {
                size: read.opened.stat.size,
                version: read.opened.version,
                modified: read.opened.stat.mtime,
                body: read.body.unwrap_or_else(|| Box::pin(futures_util::stream::empty())),
            })
        })
    }

    fn write(&self, scope: Scope, path: String, replace: bool, body: ByteStream) -> Answer<()> {
        let cancel = self.closed.clone();
        Box::pin(async move {
            let target = resolve(&scope.cwd, &path).map_err(failed)?;
            let exists = if replace {
                wire::WriteExists::Replace
            } else {
                wire::WriteExists::Refuse
            };
            let written = write_file(std::future::ready(Ok(body)), &target, exists, &cancel).await;
            match written {
                Ok(_) => Ok(()),
                Err(error) => Err(match fs::error_code(&error) {
                    Some("EISDIR") => ChannelError::new(
                        ChannelErrorCode::IsDirectory,
                        409,
                        "A directory is at this path",
                    ),
                    Some("EEXIST") => ChannelError::new(
                        ChannelErrorCode::FileExists,
                        409,
                        "A file is already at this path",
                    ),
                    _ => failed(error),
                }),
            }
        })
    }

    fn text(&self, scope: Scope, path: String, held: Option<String>) -> Answer<FileText> {
        let cancel = self.closed.clone();
        Box::pin(async move {
            let target = resolve(&scope.cwd, &path);
            // One byte more than the limit, so a file past it is refused
            // once that byte arrives.
            let limit = Some(EDIT_FILE_BYTES as u64 + 1);
            let read = open_read(target, 0, limit, held.as_deref(), &cancel)
                .await
                .map_err(failed)?;
            let version = read.opened.version;
            let Some(mut body) = read.body else {
                return Ok(FileText {
                    version,
                    text: None,
                });
            };
            let mut bytes = Vec::new();
            while let Some(chunk) = body.next().await {
                bytes.extend_from_slice(&chunk.map_err(failed)?);
            }
            let text = text_of(bytes).map_err(|refusal| match refusal {
                TextRefusal::TooLarge => {
                    ChannelError::new(ChannelErrorCode::FileTooLarge, 413, refusal.to_string())
                }
                TextRefusal::NotText => {
                    ChannelError::new(ChannelErrorCode::NotText, 415, refusal.to_string())
                }
            })?;
            Ok(FileText {
                version,
                text: Some(text),
            })
        })
    }

    fn list(&self, scope: Scope, path: Option<String>) -> Answer<Listing> {
        let cancel = self.closed.clone();
        let home = self.home.clone();
        Box::pin(async move {
            let path = path.unwrap_or_else(|| scope.cwd.clone());
            let request = Inbound::FsReaddir {
                id: String::new(),
                path: path.clone(),
                cwd: None,
            };
            let FsResult::Readdir(entries) = fs::call(&request, Path::new(&scope.cwd), &cancel)
                .await
                .map_err(failed)?
            else {
                unreachable!("a listing answers its entries");
            };
            Ok(Listing {
                path,
                home: Some(home),
                entries: entries.into_iter().filter_map(DirectoryEntry::from_wire).collect(),
            })
        })
    }

    fn mkdir(&self, scope: Scope, path: String) -> Answer<()> {
        let cancel = self.closed.clone();
        Box::pin(async move {
            let request = Inbound::FsMkdir {
                id: String::new(),
                path,
                cwd: None,
                recursive: Some(true),
            };
            fs::call(&request, Path::new(&scope.cwd), &cancel)
                .await
                .map_err(failed)?;
            Ok(())
        })
    }

    fn delete(&self, scope: Scope, path: String) -> Answer<()> {
        let cancel = self.closed.clone();
        let home = self.home.clone();
        Box::pin(async move {
            if protected_path(&path, [home.as_str(), scope.cwd.as_str()]) {
                return Err(ChannelError::new(
                    ChannelErrorCode::ProtectedPath,
                    409,
                    "The root, the home directory and the execution directory stay, with every directory holding them",
                ));
            }
            let request = Inbound::FsRm {
                id: String::new(),
                path,
                cwd: None,
                recursive: Some(true),
                force: Some(true),
            };
            fs::call(&request, Path::new(&scope.cwd), &cancel)
                .await
                .map_err(failed)?;
            Ok(())
        })
    }

    fn watch(&self, scope: Scope, paths: mpsc::UnboundedReceiver<Vec<String>>) -> WatchStream {
        let (messages, said) = mpsc::unbounded_channel();
        let watch = PageWatchTask {
            watches: self.watches.clone(),
            cwd: PathBuf::from(&scope.cwd),
            cancel: self.closed.clone(),
            messages,
        };
        // The watch ends when the page's stream of it is dropped.
        let task = AbortOnDropHandle::new(tokio::spawn(watch.run(paths)));
        Box::pin(futures_util::stream::unfold(
            (said, task),
            |(mut said, task)| async move { said.recv().await.map(|message| (message, (said, task))) },
        ))
    }

    fn stream(&self, scope: Scope, request: StreamRequest) -> Answer<ByteStream> {
        let opener = self.streams.clone();
        let control = self.control.clone();
        let stream = self.closed.child_token();
        Box::pin(async move {
            let start = StreamStart {
                stream_id: uuid::Uuid::new_v4().simple().to_string(),
                context: CommandContext {
                    conversation: scope.conversation,
                    caller: CommandCaller::User {},
                    locale: request.introduction.locale.clone(),
                },
                package: request.binding.package,
                operation: request.binding.operation,
                args: request.args,
                json: None,
                cwd: scope.cwd,
            };
            // The channel's end, or this answer dropped before the stream
            // opened, ends the invocation.
            let ending = stream.clone().drop_guard();
            let (opened, opening) = tokio::sync::oneshot::channel();
            let (uploads, uploaded) = mpsc::channel::<io::Result<Bytes>>(STREAM_OUTPUT);
            let input = request.input;
            let running = stream.clone();
            // The invocation runs on the connection's runtime, as the
            // backend's streams do.
            control.spawn(async move {
                let invocation = match opener.open(start, &running).await {
                    Ok(invocation) => invocation,
                    Err(refusal) => {
                        // The channel that asked may have gone.
                        let _ = opened.send(Err(refusal));
                        return;
                    }
                };
                if opened.send(Ok(())).is_err() {
                    return;
                }
                let source = PageSource {
                    input,
                    pending: Bytes::new(),
                };
                invocation.run(source, uploads, &running).await;
            });
            match opening.await {
                Ok(Ok(())) => {}
                Ok(Err(StreamRefusal::Refused { code, message })) => {
                    let code = match code {
                        ServiceErrorCode::UnknownOperation => ChannelErrorCode::UnknownStream,
                        ServiceErrorCode::ServiceFailed | ServiceErrorCode::Refused => {
                            ChannelErrorCode::StreamFailed
                        }
                    };
                    let status = if code == ChannelErrorCode::UnknownStream { 404 } else { 502 };
                    return Err(ChannelError::new(code, status, message));
                }
                Ok(Err(StreamRefusal::Cancelled)) | Err(_) => {
                    return Err(ChannelError::new(
                        ChannelErrorCode::StreamFailed,
                        502,
                        "The stream ended before it opened",
                    ));
                }
            }
            let output = futures_util::stream::unfold(
                (uploaded, ending),
                |(mut uploaded, ending)| async move {
                    uploaded.recv().await.map(|item| (item, (uploaded, ending)))
                },
            );
            Ok(Box::pin(output) as ByteStream)
        })
    }
}

/// The page's bytes as the invocation asks for them, each chunk within the
/// protocol's record limit.
struct PageSource {
    input: ByteStream,
    pending: Bytes,
}

impl InputSource for PageSource {
    type Error = io::Error;

    async fn next(&mut self) -> io::Result<Option<Bytes>> {
        while self.pending.is_empty() {
            match self.input.next().await {
                Some(chunk) => self.pending = chunk?,
                None => return Ok(None),
            }
        }
        let limit = self.pending.len().min(demi_command_protocol::MAX_RECORD_BYTES);
        Ok(Some(self.pending.split_to(limit)))
    }
}

/// A page's file watch on the runner (`web-api.md` § File watch): the
/// working tree's watch and one of each path the page names, followed
/// through the connection's watches the backend's watches share, with the
/// order of states the relay's watch keeps.
struct PageWatchTask {
    watches: Watches,
    cwd: PathBuf,
    cancel: CancellationToken,
    messages: mpsc::UnboundedSender<FileWatchMessage>,
}

/// Which watch a report comes from: the working tree's, or a path's.
type Source = Option<String>;

impl PageWatchTask {
    async fn run(self, mut requested: mpsc::UnboundedReceiver<Vec<String>>) {
        let (reports, mut reported) = mpsc::unbounded_channel::<(Source, WatchReport)>();
        let mut watch = PageWatch::new();
        let _tree = self.follow(None, self.cwd.clone(), true, &reports);
        let mut followed: HashMap<String, AbortOnDropHandle<()>> = HashMap::new();
        loop {
            let said = tokio::select! {
                () = self.cancel.cancelled() => return,
                paths = requested.recv() => {
                    let Some(paths) = paths else {
                        return;
                    };
                    let changed = watch.paths(paths);
                    for path in changed.stop {
                        followed.remove(&path);
                    }
                    for path in changed.start {
                        let task = self.follow(Some(path.clone()), PathBuf::from(&path), false, &reports);
                        followed.insert(path, task);
                    }
                    Vec::new()
                }
                Some((source, report)) = reported.recv() => watch.report(source.as_deref(), report),
            };
            for message in said.into_iter().chain(watch.settle()) {
                if self.messages.send(message).is_err() {
                    return;
                }
            }
        }
    }

    /// Follows the watch of `path`, below it when `recursive`, handing its
    /// reports on until the task is dropped.
    fn follow(
        &self,
        source: Source,
        path: PathBuf,
        recursive: bool,
        reports: &mpsc::UnboundedSender<(Source, WatchReport)>,
    ) -> AbortOnDropHandle<()> {
        let watches = self.watches.clone();
        let cancel = self.cancel.clone();
        let reports = reports.clone();
        let requested = resolve(&self.cwd, &path.to_string_lossy());
        AbortOnDropHandle::new(tokio::spawn(async move {
            let watched = Watched {
                watches: &watches,
                requested,
                recursive,
                cancel: &cancel,
            };
            watched
                .follow(|report| {
                    let sent = reports.send((source.clone(), report)).is_ok();
                    std::future::ready(sent)
                })
                .await;
        }))
    }
}

/// A file operation's failure as the relay's route answers it.
fn failed(error: io::Error) -> ChannelError {
    match FsFailure::of(fs::error_code(&error)) {
        FsFailure::NotFound => ChannelError::new(ChannelErrorCode::FsError, 404, error.to_string()),
        FsFailure::Forbidden => ChannelError::new(ChannelErrorCode::FsError, 403, error.to_string()),
        FsFailure::Failed => {
            ChannelError::new(ChannelErrorCode::HostOperationFailed, 500, error.to_string())
        }
    }
}
