//! A connection's direct channels (`crates/runner-direct/src/lib.rs`): a page reaches the runner
//! without the backend in the middle, over WebRTC data channels the backend
//! introduces and authorizes. The runner carries out each channel's operation as it
//! carries out the backend's request for the same thing (`demi_runner_host`), with
//! the same functions, limits, atomic writes and file watches, and opens a `stream`
//! as a `service_open`, with a `user` caller in the conversation the header names.
//! It tells the backend each stream's opening (`direct_stream`), before the stream
//! asks the backend for its artifacts as a stream the backend opened does, and its
//! closing: the backend counts an open stream as the conversation's activity, so a
//! live view watched only over the direct channel keeps the conversation from its
//! idle release. Every peer closes when the connection to the backend ends.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    time::Instant,
};


use bytes::Bytes;
use demi_command_protocol::{CommandCaller, CommandContext, EDIT_FILE_BYTES, TextRefusal, text_of};
use demi_command_sdk::{InputSource, paths::resolve};
use demi_runner_direct::{
    Answer, ByteStream, FileRange, FileText, Listing, Operations, Scope, StreamActivity,
    StreamRequest, WatchStream,
};
use demi_runner_host::{
    files::{open_read, write_file},
    fs,
    subscriptions::{Report, Subscriptions},
    watches::Watches,
};
use demi_runner_jobs::commands::streams::{StreamOpener, StreamRefusal, StreamStart};
use demi_runner_protocol::{
    direct::{ChannelError, ChannelErrorCode},
    files::{
        DirectoryEntry, Effect, FileWatchMessage, FileWatchRequest, FsFailure, PageSubscriptions,
        Subject, protected_path,
    },
    wire::{self, FsResult, Inbound, Outbound, ServiceErrorCode},
};
use futures_util::{StreamExt, future::BoxFuture};
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
    /// The connection's messages to the backend, which also carry its
    /// streams' artifact requests.
    pub backend: mpsc::Sender<wire::Frame>,
    /// The connection's end, which ends every operation.
    pub closed: CancellationToken,
    /// The forwards this runner is an end of, as the backend last said.
    pub forwards: crate::forward::ForwardList,
}

/// An open direct stream as the backend knows it: reported open before it
/// is made, and closed as it is dropped.
struct ReportedStream {
    backend: mpsc::Sender<wire::Frame>,
    control: Handle,
    closed: CancellationToken,
    stream: String,
    name: String,
    conversation: String,
}

impl ReportedStream {
    /// Tells the backend the stream opened or closed, unless the
    /// connection ended first: the backend's end of it ended what it knew
    /// of the stream already.
    fn report(&self, open: bool) -> impl Future<Output = ()> + Send + 'static {
        let report = Outbound::DirectStream {
            stream: self.stream.clone(),
            name: self.name.clone(),
            conversation: self.conversation.clone(),
            open,
        };
        let backend = self.backend.clone();
        let closed = self.closed.clone();
        async move {
            let frame = match wire::encode(&report) {
                Ok(frame) => frame,
                Err(error) => {
                    tracing::warn!("a direct stream's report encoding failed: {error}");
                    return;
                }
            };
            tokio::select! {
                _ = closed.cancelled() => {}
                // A connection that ended has no backend left to tell.
                _ = backend.send(frame) => {}
            }
        }
    }
}

impl Drop for ReportedStream {
    fn drop(&mut self) {
        // Its opening went ahead of this, or never went, and then the
        // backend ignores a stream it does not know.
        self.control.spawn(self.report(false));
    }
}

impl Operations for HostOperations {
    fn read(&self, scope: Scope, path: String, offset: u64, length: Option<u64>) -> Answer<FileRange> {
        let cancel = self.closed.clone();
        Box::pin(async move {
            let target = resolve(&scope.cwd, &path);
            // Taken before the read, as every read of what a subscription covers.
            let read_at = demi_runner_host::numbers::read_at();
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
                read_at,
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
            // Before anything is read (`crates/runner-host/src/subscriptions.rs`).
            let read_at = demi_runner_host::numbers::read_at();
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
                    read_at,
                });
            };
            let mut bytes = Vec::new();
            while let Some(chunk) = body.next().await {
                bytes.extend_from_slice(&chunk.map_err(failed)?);
            }
            let text = text_of(bytes).map_err(|refusal| {
                let error = match refusal {
                    TextRefusal::TooLarge => {
                        ChannelError::new(ChannelErrorCode::FileTooLarge, 413, refusal.to_string())
                    }
                    TextRefusal::NotText => {
                        ChannelError::new(ChannelErrorCode::NotText, 415, refusal.to_string())
                    }
                };
                // Such a file is an answer like any other.
                error.of_file(version.clone(), read_at.clone())
            })?;
            Ok(FileText {
                version,
                text: Some(text),
                read_at,
            })
        })
    }

    fn list(&self, scope: Scope, path: Option<String>) -> Answer<Listing> {
        let cancel = self.closed.clone();
        let home = self.home.clone();
        Box::pin(async move {
            // Before anything is read (`crates/runner-host/src/subscriptions.rs`).
            let read_at = demi_runner_host::numbers::read_at();
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
                read_at,
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

    fn watch(&self, scope: Scope, requests: mpsc::UnboundedReceiver<FileWatchRequest>) -> WatchStream {
        let (messages, said) = mpsc::unbounded_channel();
        let watch = PageWatchTask {
            watches: self.watches.clone(),
            cwd: PathBuf::from(&scope.cwd),
            cancel: self.closed.clone(),
            messages,
        };
        // The watch ends when the page's stream of it is dropped.
        let task = AbortOnDropHandle::new(tokio::spawn(watch.run(requests)));
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
            // Its artifacts are asked for under the id the backend knows it by.
            let start = StreamStart {
                stream_id: request.stream,
                context: CommandContext {
                    conversation: scope.conversation,
                    caller: CommandCaller::User {},
                    locale: request.introduction.locale.clone(),
                    color_scheme: request.introduction.color_scheme,
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

    fn stream_activity(&self, conversation: &str, name: &str) -> BoxFuture<'static, StreamActivity> {
        let stream = ReportedStream {
            backend: self.backend.clone(),
            control: self.control.clone(),
            closed: self.closed.clone(),
            stream: uuid::Uuid::new_v4().simple().to_string(),
            name: name.to_owned(),
            conversation: conversation.to_owned(),
        };
        Box::pin(async move {
            // In the backend's queue before anything the stream asks it.
            stream.report(true).await;
            StreamActivity {
                stream: stream.stream.clone(),
                held: Box::new(stream),
            }
        })
    }

    fn forward(&self, forward: String, from: String) -> Answer<demi_runner_direct::Forwarded> {
        let target = crate::forward::connect_target_of(&self.forwards, &forward, &from);
        Box::pin(async move {
            let (address, ended) = target.ok_or_else(|| {
                ChannelError::new(
                    ChannelErrorCode::HostOperationFailed,
                    404,
                    "This Host connects no such forward",
                )
            })?;
            let tcp = demi_runner_direct::connect_target(&address, &ended)
                .await
                .map_err(|error| ChannelError::new(ChannelErrorCode::HostOperationFailed, 502, error.to_string()))?;
            Ok(demi_runner_direct::Forwarded { tcp, ended })
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

/// A page's file watch on the runner: the page's subscriptions as the relay's
/// watch keeps them, held as subscriptions of the runner's own through the
/// connection's watches, which the backend's subscriptions share, with the same
/// numbers.
struct PageWatchTask {
    watches: Watches,
    cwd: PathBuf,
    cancel: CancellationToken,
    messages: mpsc::UnboundedSender<FileWatchMessage>,
}

impl PageWatchTask {
    async fn run(self, mut requests: mpsc::UnboundedReceiver<FileWatchRequest>) {
        let (subscriptions, mut reports) = Subscriptions::new(self.watches.clone(), self.cwd.clone());
        let mut page = PageSubscriptions::new(Some(self.cwd.to_string_lossy().into_owned()));
        let mut held = Held::default();
        loop {
            let expiry = page.next_expiry();
            let effects = tokio::select! {
                () = self.cancel.cancelled() => return,
                request = requests.recv() => {
                    let Some(request) = request else {
                        return;
                    };
                    // On the runtime's clock, which a test may hold still.
                    let now = tokio::time::Instant::now().into_std();
                    match request {
                        FileWatchRequest::Subscribe { subscriptions } => page.subscribe(subscriptions, now),
                        FileWatchRequest::Unsubscribe { keys } => page.unsubscribe(&keys),
                        FileWatchRequest::Renew { keys } => page.renew(&keys, now),
                    }
                }
                report = reports.recv() => {
                    let Some(report) = report else {
                        return;
                    };
                    match report {
                        Report::Live { id, since } => match held.subject(&id) {
                            Some(subject) => page.live(&subject, since, demi_runner_host::numbers::epoch()),
                            None => Vec::new(),
                        },
                        Report::Failed { id, reason } => match held.forget(&id) {
                            Some(subject) => page.failed(&subject, &reason),
                            None => Vec::new(),
                        },
                        Report::Changed(changes) => {
                            let changes: Vec<(Subject, u64)> = changes
                                .into_iter()
                                .filter_map(|(id, at)| held.subject(&id).map(|subject| (subject, at)))
                                .collect();
                            page.changed(&changes)
                        }
                    }
                }
                () = sleep_until(expiry), if expiry.is_some() => {
                    page.expire(tokio::time::Instant::now().into_std())
                }
            };
            for effect in effects {
                match effect {
                    Effect::Page(message) => {
                        if self.messages.send(message).is_err() {
                            return;
                        }
                    }
                    Effect::Hold(subject) => {
                        let id = held.hold(subject.clone());
                        subscriptions.subscribe(id, subject.kind, subject.path);
                    }
                    Effect::Release(subject) => {
                        if let Some(id) = held.release(&subject) {
                            subscriptions.unsubscribe(id);
                        }
                    }
                    Effect::Renew(subjects) => {
                        subscriptions.renew(subjects.iter().filter_map(|subject| held.id(subject)).collect());
                    }
                }
            }
        }
    }
}

/// The runner's subscriptions a page's watch holds, by the subject each
/// is of and the id it was given.
#[derive(Default)]
struct Held {
    ids: HashMap<Subject, String>,
    subjects: HashMap<String, Subject>,
    next: u64,
}

impl Held {
    fn hold(&mut self, subject: Subject) -> String {
        self.next += 1;
        let id = self.next.to_string();
        self.ids.insert(subject.clone(), id.clone());
        self.subjects.insert(id.clone(), subject);
        id
    }

    fn release(&mut self, subject: &Subject) -> Option<String> {
        let id = self.ids.remove(subject)?;
        self.subjects.remove(&id);
        Some(id)
    }

    /// The subscription is over on the runner's side.
    fn forget(&mut self, id: &str) -> Option<Subject> {
        let subject = self.subjects.remove(id)?;
        self.ids.remove(&subject);
        Some(subject)
    }

    fn subject(&self, id: &str) -> Option<Subject> {
        self.subjects.get(id).cloned()
    }

    fn id(&self, subject: &Subject) -> Option<String> {
        self.ids.get(subject).cloned()
    }
}

/// Waits until `deadline`, or forever without one.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
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
