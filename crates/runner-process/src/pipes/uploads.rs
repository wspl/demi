//! The runner's uploads: a content pipe's bytes sent to the backend in turn,
//! in chunks, and again from where a broken connection left them
//! (`crates/runner-protocol/src/wire/pipe_frames.rs`).
//!
//! - The runner sends at most [`UPLOADS_AT_ONCE`] uploads at once, whoever
//!   asked for them: the link they share is the device's. An upload a page
//!   waits to show ([`wire::PipeRef::shown`]) goes before the others; within
//!   each, the first to ask goes first.
//! - An upload that waits says so with `pipe_queued`, so its pipe does not
//!   run out of the time its ends have to arrive.
//! - An upload goes as chunks of [`wire::UPLOAD_CHUNK_BYTES`]. A chunk whose
//!   connection breaks goes again on a new one from the offset the backend
//!   took, for as long as [`RECONNECT_FOR`] passes without a chunk taken.
//! - A failure says its cause with every error beneath it ([`causes`]).
//!
//! The body is what the runner can send again from any offset: bytes it
//! holds, and ranges of files it holds open ([`UploadBody`]).

use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex, PoisonError},
};

use bytes::Bytes;
use demi_runner_protocol::wire;
use futures_util::{Stream, StreamExt};
use tokio::{sync::oneshot, time::Instant};
use tokio_util::sync::CancellationToken;

use super::{Attempt, PipeClient, RECONNECT_FOR, RECONNECT_PAUSE, cancelled, expect_ok, lost_connection};

/// The most uploads a runner sends at once, Docker's default for all of a
/// daemon's pushes: more at once on a slow link through a proxy stalled
/// them all, and the runner's own connection with them.
pub const UPLOADS_AT_ONCE: usize = 5;

/// How much of a file one read of an upload takes.
const READ_BYTES: usize = 64 * 1024;

/// What an upload sends, which it can send again from any offset: bytes, and
/// ranges of open files, in order.
#[derive(Default)]
pub struct UploadBody {
    parts: Vec<Part>,
    len: u64,
}

enum Part {
    Bytes(Bytes),
    /// `len` bytes of `file` from `start`, which it must still have.
    File { file: Arc<std::fs::File>, start: u64, len: u64 },
}

impl Part {
    fn len(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::File { len, .. } => *len,
        }
    }
}

impl UploadBody {
    pub fn new() -> Self {
        Self::default()
    }

    /// `bytes` next.
    pub fn bytes(mut self, bytes: Bytes) -> Self {
        self.len += bytes.len() as u64;
        self.parts.push(Part::Bytes(bytes));
        self
    }

    /// `len` bytes of `file` from `start` next. A file that holds fewer by
    /// the time they are read fails the upload.
    pub fn file(mut self, file: std::fs::File, start: u64, len: u64) -> Self {
        self.len += len;
        self.parts.push(Part::File { file: Arc::new(file), start, len });
        self
    }

    /// Its bytes in all.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Its bytes from `from` to `to`.
    fn range(&self, from: u64, to: u64) -> impl Stream<Item = Result<Bytes, SourceFailed>> + Send + 'static {
        let mut pieces = Vec::new();
        let mut at = 0u64;
        for part in &self.parts {
            let (start, end) = (at, at + part.len());
            at = end;
            if end <= from || start >= to {
                continue;
            }
            let (within, until) = (from.saturating_sub(start), to.min(end) - start);
            pieces.push(match part {
                Part::Bytes(bytes) => Piece::Bytes(bytes.slice(within as usize..until as usize)),
                Part::File { file, start, .. } => Piece::File {
                    file: file.clone(),
                    from: start + within,
                    len: until - within,
                },
            });
        }
        futures_util::stream::iter(pieces).flat_map(Piece::into_stream)
    }
}

/// A part's bytes one range of an upload sends.
enum Piece {
    Bytes(Bytes),
    File { file: Arc<std::fs::File>, from: u64, len: u64 },
}

impl Piece {
    fn into_stream(self) -> futures_util::stream::BoxStream<'static, Result<Bytes, SourceFailed>> {
        match self {
            Self::Bytes(bytes) => futures_util::stream::once(async move { Ok(bytes) }).boxed(),
            Self::File { file, from, len } => read_range(file, from, len).boxed(),
        }
    }
}

/// The `len` bytes of `file` from `from`, which it must have, each read at
/// its own position. A seek and a read would share the file's position with
/// a read of an earlier chunk that still runs after its connection broke,
/// which would move it.
fn read_range(
    file: Arc<std::fs::File>,
    from: u64,
    len: u64,
) -> impl Stream<Item = Result<Bytes, SourceFailed>> + Send + 'static {
    futures_util::stream::unfold(Some((from, len)), move |state| {
        let file = file.clone();
        async move {
            let (at, left) = state?;
            if left == 0 {
                return None;
            }
            let want = left.min(READ_BYTES as u64) as usize;
            let read = tokio::task::spawn_blocking(move || {
                let mut buffer = vec![0; want];
                let read = read_at(&file, &mut buffer, at)?;
                buffer.truncate(read);
                Ok(buffer)
            })
            .await
            .unwrap_or_else(|error| Err(io::Error::other(error)));
            match read {
                Ok(buffer) if buffer.is_empty() => Some((
                    Err(SourceFailed(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the file got shorter while it was read",
                    ))),
                    None,
                )),
                Ok(buffer) => {
                    let read = buffer.len() as u64;
                    Some((Ok(Bytes::from(buffer)), Some((at + read, left - read))))
                }
                Err(error) => Some((Err(SourceFailed(error)), None)),
            }
        }
    })
}

/// Reads into `buffer` from `file` at `at`, without its position.
fn read_at(file: &std::fs::File, buffer: &mut [u8], at: u64) -> io::Result<usize> {
    loop {
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(file, buffer, at);
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(file, buffer, at);
        match read {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            read => return read,
        }
    }
}

/// A failure to read an upload's own bytes, which no other connection
/// mends. It is the body's error itself, which the request's error holds
/// beneath it: wrapped in an `io::Error`, the chain would skip it.
#[derive(Debug)]
struct SourceFailed(io::Error);

impl std::fmt::Display for SourceFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SourceFailed {}

/// The upload's own bytes failed beneath `error`, when they did.
fn source_failure(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(failed) = error.downcast_ref::<SourceFailed>() {
            return Some(failed.to_string());
        }
        next = error.source();
    }
    None
}

/// `error` with every error beneath it, each joined by `: `, the words of
/// one that the error above repeats left out: `error sending request for url
/// (…): client error (SendRequest): connection closed before message
/// completed` rather than the first part alone.
pub fn causes(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        let words = cause.to_string();
        if !words.is_empty() && !text.contains(&words) {
            text.push_str(": ");
            text.push_str(&words);
        }
        next = cause.source();
    }
    text
}

/// The runner's uploads that send or wait to.
#[derive(Default)]
pub(super) struct Uploads(Mutex<Queue>);

#[derive(Default)]
struct Queue {
    sending: usize,
    shown: VecDeque<oneshot::Sender<Turn>>,
    other: VecDeque<oneshot::Sender<Turn>>,
}

/// An upload's turn to send; dropping it gives the turn to the next upload
/// that waits.
pub(super) struct Turn(Option<Arc<Uploads>>);

impl Uploads {
    /// A turn at once, or the wait for one.
    fn turn(self: &Arc<Self>, shown: bool) -> Result<Turn, oneshot::Receiver<Turn>> {
        let mut queue = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if queue.sending < UPLOADS_AT_ONCE {
            queue.sending += 1;
            return Ok(Turn(Some(self.clone())));
        }
        let (sender, receiver) = oneshot::channel();
        if shown {
            queue.shown.push_back(sender);
        } else {
            queue.other.push_back(sender);
        }
        Err(receiver)
    }

    /// Hands a turn that ended to the next upload that still waits.
    fn pass(self: &Arc<Self>) {
        let mut queue = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some(next) = queue.shown.pop_front().or_else(|| queue.other.pop_front()) {
            match next.send(Turn(Some(self.clone()))) {
                Ok(()) => return,
                // It stopped waiting: its turn is nobody's.
                Err(mut turn) => turn.0 = None,
            }
        }
        queue.sending -= 1;
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if let Some(uploads) = self.0.take() {
            uploads.pass();
        }
    }
}

impl PipeClient {
    /// Uploads `body` into the content pipe `pipe` in its turn, in chunks
    /// that go again from where a broken connection left them, and answers
    /// the pipe's outcome. `queued` runs when the upload has to wait.
    pub async fn upload(
        &self,
        pipe: &wire::PipeRef,
        body: UploadBody,
        queued: impl FnOnce(),
        cancel: &CancellationToken,
    ) -> io::Result<()> {
        let began = Instant::now();
        let bytes = body.len();
        let turn = match self.uploads.turn(pipe.shown) {
            Ok(turn) => turn,
            Err(waiting) => {
                queued();
                tracing::info!(event = "upload.queued", pipe = %pipe.id, shown = pipe.shown, bytes, "an upload waits for its turn");
                tokio::select! {
                    () = cancel.cancelled() => return Err(cancelled()),
                    turn = waiting => turn.map_err(|_| io::Error::other("the runner's uploads ended"))?,
                }
            }
        };
        let waited_ms = began.elapsed().as_millis() as u64;
        tracing::info!(event = "upload.started", pipe = %pipe.id, shown = pipe.shown, bytes, waited_ms, "an upload started");
        let started = Instant::now();
        let sent = tokio::select! {
            () = cancel.cancelled() => Err(cancelled()),
            sent = self.send_chunks(pipe, &body) => sent,
        };
        drop(turn);
        let ms = started.elapsed().as_millis() as u64;
        match sent {
            Ok(resumed) => {
                tracing::info!(event = "upload.ended", pipe = %pipe.id, bytes, waited_ms, ms, resumed, "an upload ended");
                Ok(())
            }
            Err(error) => {
                let cause = causes(&error);
                tracing::warn!(event = "upload.failed", pipe = %pipe.id, bytes, waited_ms, ms, %cause, "an upload failed");
                Err(error)
            }
        }
    }

    /// Sends `body` into `pipe` chunk by chunk to its end, and answers how
    /// many times it went again from the backend's offset.
    async fn send_chunks(&self, pipe: &wire::PipeRef, body: &UploadBody) -> io::Result<u32> {
        let total = body.len();
        let mut offset = 0u64;
        let mut resumed = 0u32;
        let mut lost_since: Option<Instant> = None;
        let mut backoff = demi_command_sdk::descriptors::Backoff::default();
        loop {
            let end = (offset + wire::UPLOAD_CHUNK_BYTES).min(total);
            let complete = end == total;
            let request = self
                .request(reqwest::Method::PUT, &pipe.url)?
                .header(wire::UPLOAD_OFFSET, offset)
                .header(wire::UPLOAD_COMPLETE, if complete { "?1" } else { "?0" })
                .header(reqwest::header::CONTENT_LENGTH, end - offset)
                .body(reqwest::Body::wrap_stream(body.range(offset, end)));
            // Why the chunk goes again, when it does.
            let again = match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let taken = taken_offset(&response);
                    match (status, taken) {
                        (reqwest::StatusCode::OK, _) if complete => {
                            // The confirmation, which carries no payload.
                            expect_ok(response).await?.bytes().await.map_err(io::Error::other)?;
                            return Ok(resumed);
                        }
                        (reqwest::StatusCode::NO_CONTENT, Some(taken)) if !complete && taken <= total => {
                            offset = taken;
                            lost_since = None;
                            continue;
                        }
                        // Another offset than the backend took: from there.
                        (reqwest::StatusCode::CONFLICT, Some(taken)) if taken <= total && taken != offset => {
                            if taken > offset {
                                lost_since = None;
                            }
                            offset = taken;
                            continue;
                        }
                        // A chunk before, whose connection broke, is still
                        // being taken.
                        (reqwest::StatusCode::CONFLICT, Some(_)) => {
                            "the chunk before is still being taken".to_owned()
                        }
                        _ => return Err(expect_ok(response).await.err().unwrap_or_else(|| {
                            io::Error::other(format!("the backend answered a chunk with {status}"))
                        })),
                    }
                }
                Err(error) => {
                    if let Some(failure) = source_failure(&error) {
                        return Err(io::Error::other(failure));
                    }
                    // Out of open files, the connection waits for one
                    // (`crates/command-sdk/src/descriptors.rs`).
                    if demi_command_sdk::descriptors::exhausted(&error) {
                        backoff.wait().await;
                        continue;
                    }
                    if !lost_connection(&error) {
                        return Err(io::Error::other(causes(&error)));
                    }
                    causes(&error)
                }
            };
            let since = *lost_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= RECONNECT_FOR {
                return Err(io::Error::other(again));
            }
            tokio::time::sleep(RECONNECT_PAUSE).await;
            match self.taken(&pipe.url).await {
                Ok(taken) if taken <= total => {
                    // Bytes taken are progress, whichever chunk took them.
                    if taken > offset {
                        lost_since = None;
                    }
                    resumed += 1;
                    tracing::info!(event = "upload.resumed", pipe = %pipe.id, offset = taken, cause = %again, "an upload goes again from the backend's offset");
                    offset = taken;
                }
                Ok(taken) => {
                    return Err(io::Error::other(format!(
                        "the backend took {taken} bytes of an upload of {total}"
                    )));
                }
                // The next chunk asks again, and a 409 names the offset.
                Err(Attempt::Lost(_)) => {}
                Err(Attempt::Refused(error)) => return Err(error),
            }
        }
    }

    /// The offset of `path`'s upload the backend took.
    async fn taken(&self, path: &str) -> Result<u64, Attempt> {
        let request = self.request(reqwest::Method::HEAD, path).map_err(Attempt::Refused)?;
        let response = request.send().await.map_err(Attempt::of)?;
        match taken_offset(&response) {
            Some(taken) if response.status().is_success() => Ok(taken),
            _ => Err(Attempt::Refused(io::Error::other(format!(
                "the backend answered the upload's offset with {}",
                response.status()
            )))),
        }
    }
}

/// The offset an answer says the backend took.
fn taken_offset(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(wire::UPLOAD_OFFSET)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}
