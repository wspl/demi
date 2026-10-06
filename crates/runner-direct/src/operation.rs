//! One channel's operation (`direct-channel.md` § Operations on the
//! channel): what the runner supplies to carry it out, and how a channel
//! carries its answer, its bytes and its end.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use demi_runner_protocol::direct::{
    ChannelError, ChannelErrorCode, ChannelHeader, ChannelRefusal, Introduction, Listed,
    MESSAGE_BYTES, Opened, QUEUE_BYTES, ReadOpened, ServiceBinding, TextOpened, WATCH_HEARTBEAT,
};
use demi_runner_protocol::files::{DirectoryEntry, FileWatchMessage, FileWatchState};
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Bytes that flow one chunk at a time, as the reader asks for them.
pub type ByteStream = Pin<Box<dyn Stream<Item = io::Result<Bytes>> + Send>>;
/// What carrying out an operation answers.
pub type Answer<T> = Pin<Box<dyn Future<Output = Result<T, ChannelError>> + Send>>;
/// What a watch says, in order.
pub type WatchStream = Pin<Box<dyn Stream<Item = FileWatchMessage> + Send>>;

/// The conversation an operation acts for, and the directory its work
/// starts in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub conversation: String,
    pub cwd: String,
}

/// A file's opened range: its size and version; its bytes follow.
pub struct FileRange {
    pub size: u64,
    pub version: String,
    pub body: ByteStream,
}

/// A file's text and its version; none when the file still has the version
/// the read named.
pub struct FileText {
    pub version: String,
    pub text: Option<String>,
}

/// A directory's listing.
pub struct Listing {
    pub path: String,
    pub home: Option<String>,
    pub entries: Vec<DirectoryEntry>,
}

/// A user stream to open: the operation its name binds, with the arguments
/// the page gave and the locale of the introducing user.
pub struct StreamRequest {
    pub binding: ServiceBinding,
    pub args: Option<serde_json::Map<String, serde_json::Value>>,
    pub introduction: Arc<Introduction>,
    /// The page's bytes, the invocation's input; they end when the channel
    /// does.
    pub input: ByteStream,
}

/// What the runner supplies to carry out each operation, the same functions
/// as its answers to the backend's requests for the same thing
/// (`direct-channel.md` § Operations on the channel). Each future runs on
/// the peers' thread, and ends, dropped, when its channel closes.
pub trait Operations: Send + Sync + 'static {
    /// A file's bytes from `offset`, `length` of them or to its end.
    fn read(&self, scope: Scope, path: String, offset: u64, length: Option<u64>)
    -> Answer<FileRange>;
    /// Writes `body` to `path`, in place whole once `body` ends cleanly, and
    /// not at all when it fails.
    fn write(&self, scope: Scope, path: String, replace: bool, body: ByteStream) -> Answer<()>;
    /// A file's text, unless it still has version `held`.
    fn text(&self, scope: Scope, path: String, held: Option<String>) -> Answer<FileText>;
    /// A directory's entries; the conversation's directory without a path.
    fn list(&self, scope: Scope, path: Option<String>) -> Answer<Listing>;
    fn mkdir(&self, scope: Scope, path: String) -> Answer<()>;
    fn delete(&self, scope: Scope, path: String) -> Answer<()>;
    /// The page's file watch: the paths of each `paths` message come
    /// through `paths`, and the watch says what the relay's would.
    fn watch(&self, scope: Scope, paths: mpsc::UnboundedReceiver<Vec<String>>) -> WatchStream;
    /// Opens a user stream: its output bytes, once it opened.
    fn stream(&self, scope: Scope, request: StreamRequest) -> Answer<ByteStream>;
}

/// What the page sends on a channel after its header.
#[derive(Debug)]
pub(crate) enum Input {
    Bytes(Bytes),
    /// A `write`'s bytes are complete.
    End,
    /// A `watch`'s `paths` message.
    Paths(Vec<String>),
}

/// A message for the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Output {
    Text(String),
    Binary(Bytes),
}

impl Output {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Binary(bytes) => bytes.len(),
        }
    }
}

/// How many messages wait between an operation and its channel: the
/// channel takes one as its queue drains, so a reader paces the file.
pub(crate) const OUTPUT_MESSAGES: usize = 4;

/// The runner's refusal of an operation, as its channel's only message.
pub(crate) fn refusal(error: ChannelError) -> Output {
    let refusal = ChannelRefusal { error };
    Output::Text(serde_json::to_string(&refusal).expect("a refusal serializes"))
}

/// Carries out the operation `header` names, from the page's `input`, into
/// `output`; the channel closes when it returns and its messages are out.
pub(crate) async fn run(
    operations: Arc<dyn Operations>,
    introduction: Arc<Introduction>,
    header: ChannelHeader,
    input: mpsc::UnboundedReceiver<Input>,
    output: mpsc::Sender<Output>,
    cancel: CancellationToken,
) {
    let out = Out(output);
    tokio::select! {
        () = cancel.cancelled() => {}
        () = carry_out(operations, introduction, header, input, &out) => {}
    }
}

/// Where an operation's messages go; each send waits for room.
struct Out(mpsc::Sender<Output>);

impl Out {
    /// False once the channel is gone.
    async fn send(&self, message: Output) -> bool {
        self.0.send(message).await.is_ok()
    }

    /// The answer, as one text message that fits the channel's queue; an
    /// answer too large for it refuses with `too_large`, which sends the
    /// page's operation to the relay.
    async fn answer(&self, answer: &impl serde::Serialize) -> bool {
        let text = serde_json::to_string(answer).expect("an answer serializes");
        if text.len() > QUEUE_BYTES {
            let error = ChannelError::new(
                ChannelErrorCode::TooLarge,
                413,
                "The answer is too large for a direct channel",
            );
            return self.send(refusal(error)).await;
        }
        self.send(Output::Text(text)).await
    }

    async fn refuse(&self, error: ChannelError) -> bool {
        self.send(refusal(error)).await
    }

    /// `body`'s bytes in messages of at most [`MESSAGE_BYTES`]. A body that
    /// fails ends the channel without its remaining bytes, so the page sees
    /// the operation cut short rather than complete.
    async fn bytes(&self, mut body: ByteStream) -> bool {
        while let Some(chunk) = body.next().await {
            let Ok(mut chunk) = chunk else {
                return false;
            };
            while !chunk.is_empty() {
                let part = chunk.split_to(chunk.len().min(MESSAGE_BYTES));
                if !self.send(Output::Binary(part)).await {
                    return false;
                }
            }
        }
        true
    }
}

async fn carry_out(
    operations: Arc<dyn Operations>,
    introduction: Arc<Introduction>,
    header: ChannelHeader,
    input: mpsc::UnboundedReceiver<Input>,
    out: &Out,
) {
    let scope = Scope {
        conversation: header.scope().conversation.to_owned(),
        cwd: header.scope().cwd.to_owned(),
    };
    match header {
        ChannelHeader::Read {
            path,
            offset,
            length,
            version,
            ..
        } => {
            let range = match operations.read(scope, path, offset.unwrap_or(0), length).await {
                Ok(range) => range,
                Err(error) => {
                    out.refuse(error).await;
                    return;
                }
            };
            // The bytes of a version the page names can only be that
            // version's, never two versions spliced together.
            if version.as_ref().is_some_and(|version| *version != range.version) {
                let error = ChannelError::new(
                    ChannelErrorCode::FileChanged,
                    412,
                    "The file is no longer the version asked for",
                );
                out.refuse(error).await;
                return;
            }
            let opened = ReadOpened {
                ok: true,
                size: range.size,
                version: range.version,
            };
            if out.answer(&opened).await {
                out.bytes(range.body).await;
            }
        }
        ChannelHeader::Write { path, replace, .. } => {
            let body = write_body(input);
            match operations.write(scope, path, replace, body).await {
                Ok(()) => out.answer(&Opened::new()).await,
                Err(error) => out.refuse(error).await,
            };
        }
        ChannelHeader::Text { path, version, .. } => {
            let read = match operations.text(scope, path, version).await {
                Ok(read) => read,
                Err(error) => {
                    out.refuse(error).await;
                    return;
                }
            };
            let opened = TextOpened {
                ok: true,
                version: read.version,
                unchanged: read.text.is_none(),
            };
            if out.answer(&opened).await
                && let Some(text) = read.text
            {
                let body = futures_util::stream::iter([Ok(Bytes::from(text))]);
                out.bytes(Box::pin(body)).await;
            }
        }
        ChannelHeader::List { path, .. } => match operations.list(scope, path).await {
            Ok(listing) => {
                let listed = Listed {
                    ok: true,
                    path: listing.path,
                    home: listing.home,
                    entries: listing.entries,
                };
                out.answer(&listed).await;
            }
            Err(error) => {
                out.refuse(error).await;
            }
        },
        ChannelHeader::Mkdir { path, .. } => {
            match operations.mkdir(scope, path).await {
                Ok(()) => out.answer(&Opened::new()).await,
                Err(error) => out.refuse(error).await,
            };
        }
        ChannelHeader::Delete { path, .. } => {
            match operations.delete(scope, path).await {
                Ok(()) => out.answer(&Opened::new()).await,
                Err(error) => out.refuse(error).await,
            };
        }
        ChannelHeader::Watch { .. } => {
            if !out.answer(&Opened::new()).await {
                return;
            }
            let (paths, requested) = mpsc::unbounded_channel();
            let watch = operations.watch(scope, requested);
            watch_messages(watch, input, paths, out).await;
        }
        ChannelHeader::Stream { stream, args, .. } => {
            let Some(binding) = introduction.streams.get(&stream).cloned() else {
                let error =
                    ChannelError::new(ChannelErrorCode::UnknownStream, 404, "No such stream");
                out.refuse(error).await;
                return;
            };
            let request = StreamRequest {
                binding,
                args,
                introduction: introduction.clone(),
                input: stream_input(input),
            };
            match operations.stream(scope, request).await {
                Ok(output) => {
                    if out.answer(&Opened::new()).await {
                        out.bytes(output).await;
                    }
                }
                Err(error) => {
                    out.refuse(error).await;
                }
            }
        }
    }
}

/// A `write`'s bytes, which end cleanly at `{ end: true }`; a channel that
/// closes before it fails them, so the write leaves the file as it was.
fn write_body(input: mpsc::UnboundedReceiver<Input>) -> ByteStream {
    Box::pin(futures_util::stream::unfold(Some(input), |input| async move {
        let mut input = input?;
        match input.recv().await {
            Some(Input::Bytes(bytes)) => Some((Ok(bytes), Some(input))),
            Some(Input::End) => None,
            Some(Input::Paths(_)) | None => {
                let cut = io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the page ended the write before its last byte",
                );
                Some((Err(cut), None))
            }
        }
    }))
}

/// A stream's input bytes, which end with the channel.
fn stream_input(input: mpsc::UnboundedReceiver<Input>) -> ByteStream {
    Box::pin(futures_util::stream::unfold(input, |mut input| async move {
        loop {
            match input.recv().await? {
                Input::Bytes(bytes) => return Some((Ok(bytes), input)),
                Input::End | Input::Paths(_) => {}
            }
        }
    }))
}

/// Carries a watch's messages to the page, and the page's `paths` to the
/// watch; after [`WATCH_HEARTBEAT`] without another message it says
/// `heartbeat`, as the relay's watch does.
async fn watch_messages(
    mut watch: WatchStream,
    mut input: mpsc::UnboundedReceiver<Input>,
    paths: mpsc::UnboundedSender<Vec<String>>,
    out: &Out,
) {
    let mut quiet = Box::pin(tokio::time::sleep(WATCH_HEARTBEAT));
    loop {
        let message = tokio::select! {
            message = watch.next() => match message {
                Some(message) => message,
                None => return,
            },
            requested = input.recv() => {
                match requested {
                    // The watch ends with its channel.
                    None => return,
                    Some(Input::Paths(requested)) => {
                        // The watch ended first; its end follows.
                        let _ = paths.send(requested);
                    }
                    Some(Input::Bytes(_) | Input::End) => {}
                }
                continue;
            }
            () = &mut quiet => FileWatchMessage::Heartbeat,
        };
        let mut text = serde_json::to_string(&message).expect("a watch message serializes");
        // Paths too many for one message are reported as lost, as a
        // runner's watch reports them to the backend.
        if text.len() > QUEUE_BYTES {
            let lost = FileWatchMessage::State {
                state: FileWatchState::Lost,
                reason: None,
            };
            text = serde_json::to_string(&lost).expect("a watch message serializes");
        }
        if !out.send(Output::Text(text)).await {
            return;
        }
        quiet
            .as_mut()
            .reset(tokio::time::Instant::now() + WATCH_HEARTBEAT);
    }
}
