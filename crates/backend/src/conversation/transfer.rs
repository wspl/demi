//! File transfers (`sessions-and-targets.md` § Host operations): a file's
//! bytes between the browser and the conversation's Host, for as long as
//! the browser takes. The shard admits a transfer like any operation and
//! hands the edge the pipe end and a lease; an archive, a target change or a
//! detach ends the conversation's open transfers instead of waiting for
//! them. This module also decides what a download answers from the file's
//! metadata: its version, a conditional request, and the part a `Range`
//! asks for (`web-api.md` § File text and working tree changes).

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

use axum::http::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use demi_host_remote::{PipeReader, PipeWriter};
use demi_shell::{ByteRange, FileKind, FileStat, HostError, HostFs, WriteOptions};
use demi_web_api::ids::ConversationId;
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::host_access::{Admitted, HostAccessError, Refusal, Waits};
use crate::shard::Shard;
use crate::shard::lease::Lease;

/// What a download asks for.
#[derive(Debug, Clone)]
pub(crate) struct DownloadRequest {
    pub(crate) path: String,
    /// The version the page expects the file to still have.
    pub(crate) version: Option<String>,
    /// Only the answer's head, as `HEAD` asks.
    pub(crate) head: bool,
    pub(crate) range: Option<String>,
    pub(crate) if_none_match: Option<String>,
}

/// What a download answers.
pub(crate) enum Download {
    /// Nothing at the path is a regular file.
    NotAFile,
    /// The file is no longer the version the request named.
    Changed,
    /// The request's condition names the file's version.
    NotModified { version: String },
    /// The part's head alone: for a `HEAD`, a range past the end, or an
    /// empty part.
    Head { stat: FileStat, part: RangeAnswer },
    /// The part's bytes as the Host reads them; the edge holds the lease
    /// until it sent the last one.
    Stream {
        stat: FileStat,
        part: RangeAnswer,
        body: PipeReader,
        lease: Lease,
    },
}

/// What an upload answers before its bytes move.
pub(crate) enum Upload {
    /// A directory is at the path.
    IsDirectory,
    /// A file is at the path, and the upload did not ask to replace it.
    Exists,
    Open(OpenUpload),
}

/// An upload's write: the Host takes the bytes the edge writes and puts the
/// file in place once they end, whole or not at all.
pub(crate) struct OpenUpload {
    pub(crate) writer: PipeWriter,
    /// The write's outcome, once the Host put the file in place or failed.
    pub(crate) written: oneshot::Receiver<Result<(), HostError>>,
    pub(crate) lease: Lease,
}

impl Shard {
    /// Admits a download of one file on the conversation's main Host, and
    /// answers what the file's metadata decides: the part's bytes as a
    /// stream, or an answer without them.
    pub(crate) async fn open_download(
        &self,
        id: &ConversationId,
        request: DownloadRequest,
        cancel: &CancellationToken,
    ) -> Result<Download, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        let slot = self.conversations().slot(&record.id);
        // From the check to the registration nothing awaits, so a transition
        // that closes the conversation's transfers never misses this one.
        let open = slot.transfers.open().map_err(|_| Refusal::Busy)?;
        let waits = Waits {
            cancel,
            ended: Some(&open.ended),
        };
        let admitted = self.admit_host(&record.id, None, waits).await?;
        let host = &admitted.host.host;
        let stat = waits.wait(HostFs::stat(host, &request.path)).await??;
        if stat.kind != FileKind::File {
            return Ok(Download::NotAFile);
        }
        let version = file_version(&stat);
        if request.version.as_ref().is_some_and(|expected| *expected != version) {
            return Ok(Download::Changed);
        }
        if not_modified(request.if_none_match.as_deref(), &version) {
            return Ok(Download::NotModified { version });
        }
        let part = RangeAnswer::of(request.range.as_deref(), stat.size);
        let range = match part.range() {
            Some(range) if !request.head && range.length != Some(0) => range,
            _ => return Ok(Download::Head { stat, part }),
        };
        let body = waits.wait(host.read_pipe(&request.path, range)).await??;
        // The bytes move at the edge; the shard only holds the admission.
        let lease = self.lease_transfer(admitted, open, std::future::pending(), || {});
        Ok(Download::Stream {
            stat,
            part,
            body,
            lease,
        })
    }

    /// Admits an upload of one file to the conversation's main Host: a
    /// directory at the path is never written over, and a file only when
    /// `replace` asks.
    pub(crate) async fn open_upload(
        &self,
        id: &ConversationId,
        path: String,
        replace: bool,
        cancel: &CancellationToken,
    ) -> Result<Upload, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        let slot = self.conversations().slot(&record.id);
        // As for a download: no await between the check and the
        // registration.
        let open = slot.transfers.open().map_err(|_| Refusal::Busy)?;
        let waits = Waits {
            cancel,
            ended: Some(&open.ended),
        };
        let admitted = self.admit_host(&record.id, None, waits).await?;
        let host = admitted.host.host.clone();
        let taken = match waits.wait(HostFs::stat(&host, &path)).await? {
            Ok(stat) => Some(stat),
            Err(error) if error.code() == Some("ENOENT") => None,
            Err(error) => return Err(error.into()),
        };
        match taken {
            Some(stat) if stat.kind == FileKind::Directory => return Ok(Upload::IsDirectory),
            Some(_) if !replace => return Ok(Upload::Exists),
            _ => {}
        }
        let pipe = host.write_pipe()?;
        let writer = pipe.writer().expect("a pipe just made has its source free");
        let (done, written) = oneshot::channel();
        let input = pipe.clone();
        let write = async move {
            let outcome = host.write_from(&path, &input, WriteOptions::default()).await;
            // An edge that went away reads no outcome.
            let _ = done.send(outcome);
        };
        let stop = move || pipe.fail("the upload ended before its last byte");
        let lease = self.lease_transfer(admitted, open, write, stop);
        Ok(Upload::Open(OpenUpload { writer, written, lease }))
    }

    /// Hands the edge a lease on an admitted transfer. The shard's owner
    /// task keeps the admission and the transfer's registration until
    /// `work` ends, the edge drops the lease, or a transition ends the
    /// transfer; in the last two cases `stop` runs first. The admission goes
    /// before the registration, so a transition that waits for the transfer
    /// finds the file gate free.
    fn lease_transfer(
        &self,
        admitted: Admitted,
        open: OpenTransfer,
        work: impl Future<Output = ()> + 'static,
        stop: impl FnOnce() + 'static,
    ) -> Lease {
        let (lease, released) = Lease::new(open.ended.clone());
        let ended = open.ended.clone();
        self.tasks().spawn_local(async move {
            tokio::select! {
                () = work => {}
                () = released => stop(),
                () = ended.cancelled() => stop(),
            }
            drop(admitted);
            drop(open);
        });
        lease
    }
}

/// The conversation's open file transfers and user streams, and whether a
/// transition is closing them.
pub(crate) struct TransferSet {
    /// Cancelled when a transition closes the transfers, then replaced, so
    /// transfers admitted afterwards run under a fresh one.
    generation: RefCell<CancellationToken>,
    open: watch::Sender<usize>,
    /// How many transitions hold the transfers closed; while any does, a
    /// new transfer is refused.
    closings: Cell<u32>,
}

/// A transition is ending the conversation's transfers: a new one waits for
/// nothing and is refused (`conversation_busy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("The conversation is changing; its file transfers are closed")]
pub(crate) struct TransfersBusy;

/// A registered transfer or user stream. `ended` is cancelled when a
/// transition closes the conversation's transfers; dropping the value
/// unregisters it, which the transition waits for.
pub(crate) struct OpenTransfer {
    set: Rc<TransferSet>,
    pub(crate) ended: CancellationToken,
}

/// A transition's hold that keeps the conversation's transfers closed;
/// dropping it lets new ones in again once no other transition holds them.
pub(crate) struct TransfersClosed(Rc<TransferSet>);

impl TransferSet {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            generation: RefCell::new(CancellationToken::new()),
            open: watch::Sender::new(0),
            closings: Cell::new(0),
        })
    }

    /// Registers a transfer, unless a transition is closing the
    /// conversation's transfers. The check and the registration are one
    /// step, with no await between them, so a transition that closes the
    /// transfers never misses one admitted beside it.
    pub(crate) fn open(self: &Rc<Self>) -> Result<OpenTransfer, TransfersBusy> {
        if self.closings.get() > 0 {
            return Err(TransfersBusy);
        }
        self.open.send_modify(|open| *open += 1);
        Ok(OpenTransfer {
            set: self.clone(),
            ended: self.generation.borrow().child_token(),
        })
    }

    /// Ends every open transfer, refuses new ones until the answer is
    /// dropped, and resolves once every ended transfer let go of its
    /// admission. Transitions that close at once share the drain; the last
    /// one to let go opens the transfers again.
    pub(crate) async fn close(self: &Rc<Self>) -> TransfersClosed {
        self.closings.set(self.closings.get() + 1);
        let closed = TransfersClosed(self.clone());
        let ended = self.generation.replace(CancellationToken::new());
        ended.cancel();
        let mut open = self.open.subscribe();
        // The set holds the sender while anyone watches it.
        let _ = open.wait_for(|open| *open == 0).await;
        closed
    }
}

impl Drop for OpenTransfer {
    fn drop(&mut self) {
        self.set.open.send_modify(|open| *open -= 1);
    }
}

impl Drop for TransfersClosed {
    fn drop(&mut self) {
        self.0.closings.set(self.0.closings.get() - 1);
    }
}

/// A file's version as an ETag: its size and modification time.
pub(crate) fn file_version(stat: &FileStat) -> String {
    format!("W/\"{:x}-{}\"", stat.size, hexadecimal(stat.modified.as_millisecond()))
}

/// `value` in hexadecimal, a negative one with a minus sign, as the
/// browser writes a number in base 16.
fn hexadecimal(value: i64) -> String {
    if value < 0 {
        format!("-{:x}", value.unsigned_abs())
    } else {
        format!("{value:x}")
    }
}

/// Whether `If-None-Match` names `etag`, compared weakly (RFC 9110 § 13.1.2):
/// the tags are equal once a `W/` prefix is set aside, and `*` names every
/// version.
pub(crate) fn not_modified(if_none_match: Option<&str>, etag: &str) -> bool {
    let strip = |tag: &str| tag.trim().trim_start_matches("W/").to_owned();
    let Some(header) = if_none_match else {
        return false;
    };
    header.trim() == "*" || header.split(',').any(|tag| strip(tag) == strip(etag))
}

/// The part of a file of `size` bytes an answer sends (RFC 9110 § 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeAnswer {
    /// No range, one that does not parse, several, or a reversed one: the
    /// whole file, 200.
    Whole { size: u64 },
    /// One satisfiable range, clamped to the end: 206.
    Part { start: u64, length: u64, size: u64 },
    /// A range that starts past the end, or an empty suffix: 416.
    Unsatisfiable { size: u64 },
}

impl RangeAnswer {
    /// What a request for `size` bytes answers, given its `Range` header:
    /// `bytes=a-b`, `bytes=a-` or the suffix `bytes=-n`. A start too large
    /// for a number starts past the end; an end too large for one ends at
    /// the file's end.
    pub(crate) fn of(header: Option<&str>, size: u64) -> Self {
        let whole = Self::Whole { size };
        let Some((first, last)) = header.and_then(|header| header.trim().strip_prefix("bytes=")?.split_once('-')) else {
            return whole;
        };
        let digits = |text: &str| text.chars().all(|char| char.is_ascii_digit());
        if !digits(first) || !digits(last) || (first.is_empty() && last.is_empty()) {
            return whole;
        }
        let (start, end) = if first.is_empty() {
            // The digits are ASCII, so they only fail to parse by overflowing.
            let suffix = last.parse::<u64>().unwrap_or(u64::MAX);
            if suffix == 0 {
                return Self::Unsatisfiable { size };
            }
            (size.saturating_sub(suffix), size.checked_sub(1))
        } else {
            let Ok(start) = first.parse::<u64>() else {
                return Self::Unsatisfiable { size };
            };
            let end = if last.is_empty() {
                size.checked_sub(1)
            } else {
                let last = last.parse::<u64>().unwrap_or(u64::MAX);
                // A last byte before the first is no range at all.
                if last < start {
                    return whole;
                }
                Some(last.min(size.saturating_sub(1)))
            };
            (start, end)
        };
        match end {
            Some(end) if start < size => Self::Part {
                start,
                length: end - start + 1,
                size,
            },
            _ => Self::Unsatisfiable { size },
        }
    }

    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Whole { .. } => StatusCode::OK,
            Self::Part { .. } => StatusCode::PARTIAL_CONTENT,
            Self::Unsatisfiable { .. } => StatusCode::RANGE_NOT_SATISFIABLE,
        }
    }

    /// The bytes to send; none for a refusal.
    pub(crate) fn range(&self) -> Option<ByteRange> {
        match *self {
            Self::Whole { size } => Some(ByteRange {
                offset: 0,
                length: Some(size),
            }),
            Self::Part { start, length, .. } => Some(ByteRange {
                offset: start,
                length: Some(length),
            }),
            Self::Unsatisfiable { .. } => None,
        }
    }

    /// The headers that describe the answer's part. A streamed body's
    /// length is left out by the edge, which sends it chunked.
    pub(crate) fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        match *self {
            Self::Whole { size } => {
                headers.insert(CONTENT_LENGTH, HeaderValue::from(size));
            }
            Self::Part { start, length, size } => {
                headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
                let range = format!("bytes {start}-{}/{size}", start + length - 1);
                headers.insert(CONTENT_RANGE, HeaderValue::from_str(&range).expect("digits are a header value"));
            }
            Self::Unsatisfiable { size } => {
                let range = format!("bytes */{size}");
                headers.insert(CONTENT_RANGE, HeaderValue::from_str(&range).expect("digits are a header value"));
            }
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use demi_core::Timestamp;
    use demi_shell::FileKind;

    use super::*;

    #[test]
    fn a_range_asks_for_one_part_of_the_file_or_for_all_of_it() {
        let of = |header: &str, size: u64| RangeAnswer::of(Some(header), size);
        assert_eq!(RangeAnswer::of(None, 100), RangeAnswer::Whole { size: 100 });
        assert_eq!(of("bytes=10-19", 100), RangeAnswer::Part { start: 10, length: 10, size: 100 });
        assert_eq!(of("bytes=90-", 100), RangeAnswer::Part { start: 90, length: 10, size: 100 });
        assert_eq!(of("bytes=-5", 100), RangeAnswer::Part { start: 95, length: 5, size: 100 });
        assert_eq!(of("bytes=-500", 100), RangeAnswer::Part { start: 0, length: 100, size: 100 });
        assert_eq!(of("bytes=50-5000", 100), RangeAnswer::Part { start: 50, length: 50, size: 100 });
        // Past the end, or an empty suffix: nothing can be sent.
        for header in ["bytes=100-", "bytes=-0"] {
            assert_eq!(of(header, 100), RangeAnswer::Unsatisfiable { size: 100 }, "{header}");
        }
        assert_eq!(of("bytes=0-", 0), RangeAnswer::Unsatisfiable { size: 0 });
        assert_eq!(of("bytes=-5", 0), RangeAnswer::Unsatisfiable { size: 0 });
        // Several ranges, a reversed one, or another unit: the whole file.
        for header in ["bytes=0-1,5-6", "bytes=9-3", "items=0-1", "bytes=-", "bytes=a-b", "bytes=٣-"] {
            assert_eq!(of(header, 100), RangeAnswer::Whole { size: 100 }, "{header}");
        }
        // Numbers past a u64: a start starts past the end, an end ends at
        // the file's end.
        assert_eq!(of("bytes=99999999999999999999-", 100), RangeAnswer::Unsatisfiable { size: 100 });
        assert_eq!(of("bytes=10-99999999999999999999", 100), RangeAnswer::Part { start: 10, length: 90, size: 100 });
        assert_eq!(of("bytes=-99999999999999999999", 100), RangeAnswer::Part { start: 0, length: 100, size: 100 });

        let part = of("bytes=10-19", 100).headers();
        assert_eq!(part[CONTENT_RANGE], "bytes 10-19/100");
        assert_eq!(part[CONTENT_LENGTH], "10");
        assert_eq!(part[ACCEPT_RANGES], "bytes");
        assert_eq!(of("bytes=100-", 100).headers()[CONTENT_RANGE], "bytes */100");
        assert_eq!(RangeAnswer::Whole { size: 100 }.headers()[CONTENT_LENGTH], "100");
    }

    #[test]
    fn a_version_is_the_size_and_time_and_a_condition_compares_it_weakly() {
        let stat = FileStat {
            kind: FileKind::File,
            mode: 0o644,
            size: 300_000,
            modified: Timestamp::from_millisecond(1_790_000_000_123).unwrap(),
        };
        let etag = file_version(&stat);
        assert_eq!(etag, "W/\"493e0-1a0c4506c7b\"");
        assert!(not_modified(Some(&etag), &etag));
        assert!(not_modified(Some("\"493e0-1a0c4506c7b\""), &etag));
        assert!(not_modified(Some("\"other\", W/\"493e0-1a0c4506c7b\""), &etag));
        assert!(not_modified(Some(" * "), &etag));
        assert!(!not_modified(Some("W/\"493e0-0\""), &etag));
        assert!(!not_modified(None, &etag));
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn closing_ends_the_open_transfers_refuses_new_ones_and_waits_for_their_release() {
        let set = TransferSet::new();
        let first = set.open().unwrap();
        let second = set.open().unwrap();
        let closing = tokio::task::spawn_local({
            let set = set.clone();
            async move { set.close().await }
        });
        tokio::task::yield_now().await;
        assert!(first.ended.is_cancelled() && second.ended.is_cancelled());
        assert!(!closing.is_finished());
        // A transfer that arrives while the transition closes them waits for
        // nothing: it is refused.
        assert!(set.open().is_err());
        drop(first);
        tokio::task::yield_now().await;
        assert!(!closing.is_finished());
        drop(second);
        let closed = closing.await.unwrap();
        // Two transitions at once: the transfers open again after the last.
        let other = set.close().await;
        drop(closed);
        assert!(set.open().is_err());
        drop(other);
        let reopened = set.open().unwrap();
        assert!(!reopened.ended.is_cancelled());
    }
}
