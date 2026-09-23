//! The Host's log (`runner.md` § Host log): one bounded log of diagnostics
//! per Host, kept in two files so the older can be replaced when the newer
//! fills. Every part of the runner writes `tracing` events; the log is a
//! `tracing` layer that queues each event's lines and never waits for the
//! disk. One writer thread owns the files: it writes the queued lines and
//! answers reads between them.

use std::{
    fmt::{self, Write as _},
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::layer::{Context, Layer};

use crate::connection::wire;

/// The source of the runner's own diagnostics, and of every event that names
/// no other.
pub const RUNNER: &str = "runner";

const NEWER: &str = "host.log";
const OLDER: &str = "host.log.1";
/// Each of the two files holds this much (`runner.md` § Host log).
const FILE_BYTES: u64 = 4 * 1024 * 1024;
/// A longer line of text continues on the next log line.
const LINE_BYTES: usize = 4096;
/// Lines waiting for the writer; past it a line is dropped and counted.
const QUEUE: usize = 1024;
/// A read stops here and its cursor continues, so an answer stays well within
/// the connection's message limit.
const PAGE_BYTES: usize = 2 * 1024 * 1024;

/// One line as the files keep it, a JSON object per line. `seq` is the
/// cursor: it grows by one per line across both files and across restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Line {
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub at: i64,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub text: String,
}

pub struct Query {
    /// The `next` of an earlier page; absent, the page ends at the newest line.
    pub since: Option<u64>,
    pub limit: usize,
    pub source: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Page {
    /// Oldest first.
    pub lines: Vec<Line>,
    pub next: u64,
}

impl From<Line> for wire::LogLine {
    fn from(line: Line) -> Self {
        Self {
            at: wire::Timestamp(line.at),
            source: line.source,
            conversation_id: line.conversation_id,
            text: line.text,
        }
    }
}

struct Entry {
    at: i64,
    source: String,
    conversation_id: Option<String>,
    text: String,
}

/// What the writer thread is asked, in the order asked.
enum Message {
    Entry(Entry),
    Read {
        query: Query,
        reply: oneshot::Sender<io::Result<Page>>,
    },
    Close,
}

/// Opens the log in `directory`, continuing the files a former runner left,
/// and starts its writer thread. The layer goes into the process's `tracing`
/// subscriber; the writer stays with whoever closes the log.
pub async fn open(directory: PathBuf) -> io::Result<(HostLogWriter, HostLogLayer)> {
    tokio::fs::create_dir_all(&directory).await?;
    crate::fs::chmod(&directory, 0o700).await?;
    let files = tokio::task::spawn_blocking(move || Files::open(directory))
        .await
        .map_err(io::Error::other)??;
    let dropped = Arc::new(AtomicU64::new(0));
    let (queue, queued) = mpsc::channel(QUEUE);
    let thread = std::thread::Builder::new().name("host-log".into()).spawn({
        let dropped = dropped.clone();
        move || serve(files, queued, &dropped)
    })?;
    Ok((
        HostLogWriter {
            queue: queue.clone(),
            thread,
        },
        HostLogLayer { queue, dropped },
    ))
}

/// The writer thread: it writes each queued line, first saying how many the
/// queue had no room for, and answers each read from the files as they are.
fn serve(mut files: Files, mut queued: mpsc::Receiver<Message>, dropped: &AtomicU64) {
    while let Some(message) = queued.blocking_recv() {
        match message {
            Message::Entry(entry) => {
                let missed = dropped.swap(0, Ordering::Relaxed);
                if missed > 0 {
                    files.append(Entry {
                        at: entry.at,
                        source: RUNNER.into(),
                        conversation_id: None,
                        text: format!("{missed} log lines were dropped: the log fell behind"),
                    });
                }
                files.append(entry);
            }
            Message::Read { query, reply } => {
                // The reader may have gone; the read cost nothing to keep.
                let _gone = reply.send(files.read(&query));
            }
            Message::Close => return,
        }
    }
}

/// Owns the writer thread.
pub struct HostLogWriter {
    queue: mpsc::Sender<Message>,
    thread: std::thread::JoinHandle<()>,
}

impl HostLogWriter {
    pub fn reader(&self) -> HostLogReader {
        HostLogReader {
            queue: self.queue.clone(),
        }
    }

    /// Writes every line queued before it and stops the writer thread. Lines
    /// written afterwards are dropped.
    pub async fn close(self) {
        // The thread is gone only if it panicked; there is nothing to flush then.
        let _gone = self.queue.send(Message::Close).await;
        let thread = self.thread;
        let joined = tokio::task::spawn_blocking(move || thread.join()).await;
        if !matches!(joined, Ok(Ok(()))) {
            eprintln!("demi-runner: the host log writer failed");
        }
    }
}

/// Reads the log for `log_read` requests.
#[derive(Clone)]
pub struct HostLogReader {
    queue: mpsc::Sender<Message>,
}

impl HostLogReader {
    /// The writer thread answers between two lines it writes, so the page
    /// holds every line queued before the read.
    pub async fn read(&self, query: Query) -> io::Result<Page> {
        let (reply, answer) = oneshot::channel();
        let closed = || io::Error::other("the host log is closed");
        self.queue
            .send(Message::Read { query, reply })
            .await
            .map_err(|_| closed())?;
        answer.await.map_err(|_| closed())?
    }
}

/// The `tracing` layer that turns events into log lines: the `source` and
/// `conversation` fields place each line, and every line of the message
/// becomes one line of the log. It never waits and never fails: a line the
/// writer has no room for is dropped and counted rather than holding up what
/// it describes.
pub struct HostLogLayer {
    queue: mpsc::Sender<Message>,
    dropped: Arc<AtomicU64>,
}

impl<S: Subscriber> Layer<S> for HostLogLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let fields = Fields::of(event);
        let at = now();
        let mut splitter = LineSplitter::default();
        let mut lines = splitter.push(fields.text.as_bytes());
        lines.extend(splitter.finish());
        for text in lines {
            let entry = Entry {
                at,
                source: fields.source.clone().unwrap_or_else(|| RUNNER.into()),
                conversation_id: fields.conversation.clone(),
                text,
            };
            if self.queue.try_send(Message::Entry(entry)).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The console layer: each event it is given goes to standard error, which a
/// guest's console shows. Only what was tried and why it failed belongs there:
/// never a token, a pairing code or content (`runner.md` § Host log).
pub struct Console;

impl<S: Subscriber> Layer<S> for Console {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        eprintln!("demi-runner: {}", Fields::of(event).text);
    }
}

/// An event's text, its message followed by any other fields, and its place
/// in the log.
#[derive(Default)]
struct Fields {
    text: String,
    source: Option<String>,
    conversation: Option<String>,
}

impl Fields {
    fn of(event: &Event<'_>) -> Self {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        Self {
            text: visitor.message + &visitor.others,
            source: visitor.source,
            conversation: visitor.conversation,
        }
    }
}

#[derive(Default)]
struct Visitor {
    message: String,
    others: String,
    source: Option<String>,
    conversation: Option<String>,
}

impl Visit for Visitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "source" => self.source = Some(value.into()),
            "conversation" => self.conversation = Some(value.into()),
            _ => self.record_debug(field, &value),
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // Writing to a String does not fail.
        let _ = match field.name() {
            "message" => write!(self.message, "{value:?}"),
            name => write!(self.others, " {name}={value:?}"),
        };
    }
}

struct Files {
    directory: PathBuf,
    next_seq: u64,
    /// The newer file, opened for append, and its length.
    newer: Option<(File, u64)>,
    /// The last write failed.
    failing: bool,
}

impl Files {
    fn open(directory: PathBuf) -> io::Result<Self> {
        let mut last = None;
        for name in [OLDER, NEWER] {
            if let Some(line) = parse(&directory.join(name))?.pop() {
                last = Some(line.seq);
            }
        }
        // A new log starts at the clock instead of at one, so a cursor from a
        // log that a reset removed is older than every line of its successor.
        let next_seq = match last {
            Some(seq) => seq + 1,
            None => now().max(1) as u64,
        };
        Ok(Self {
            directory,
            next_seq,
            newer: None,
            failing: false,
        })
    }

    /// A write that fails (a full disk, a removed directory) loses the line
    /// and nothing else: the file is reopened for the next line. The console
    /// learns of it once, not once per lost line.
    fn append(&mut self, entry: Entry) {
        match self.try_append(entry) {
            Ok(()) => self.failing = false,
            Err(error) => {
                self.newer = None;
                if !self.failing {
                    eprintln!("demi-runner: host log write failed: {error}");
                }
                self.failing = true;
            }
        }
    }

    fn try_append(&mut self, entry: Entry) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(&Line {
            seq: self.next_seq,
            at: entry.at,
            source: entry.source,
            conversation_id: entry.conversation_id,
            text: entry.text,
        })
        .map_err(io::Error::other)?;
        bytes.push(b'\n');
        let size = bytes.len() as u64;
        if self.newer()?.1 + size > FILE_BYTES {
            self.newer = None;
            std::fs::rename(self.directory.join(NEWER), self.directory.join(OLDER))?;
        }
        let (file, length) = self.newer()?;
        file.write_all(&bytes)?;
        *length += size;
        self.next_seq += 1;
        Ok(())
    }

    fn newer(&mut self) -> io::Result<&mut (File, u64)> {
        if self.newer.is_none() {
            let mut options = OpenOptions::new();
            options.create(true).read(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(self.directory.join(NEWER))?;
            let mut length = file.metadata()?.len();
            // A crash or a failed write can leave half a line; end it so the
            // next line stands alone.
            if length > 0 {
                let mut end = [0];
                file.seek(SeekFrom::End(-1))?;
                file.read_exact(&mut end)?;
                if end != *b"\n" {
                    file.write_all(b"\n")?;
                    length += 1;
                }
            }
            self.newer = Some((file, length));
        }
        Ok(self.newer.as_mut().expect("opened above"))
    }

    fn read(&self, query: &Query) -> io::Result<Page> {
        let mut lines = parse(&self.directory.join(OLDER))?;
        lines.extend(parse(&self.directory.join(NEWER))?);
        let newest = lines.last().map(|line| line.seq);
        // A cursor ahead of the newest line comes from a log that is gone
        // (a reset, a clock set back): it continues from the oldest line.
        let since = query
            .since
            .filter(|since| newest.is_some_and(|newest| *since <= newest));
        let wanted = lines.into_iter().filter(|line| {
            since.is_none_or(|since| line.seq > since)
                && query
                    .source
                    .as_ref()
                    .is_none_or(|source| line.source == *source)
        });
        let mut budget = PAGE_BYTES;
        let mut fits = |line: &Line| {
            let size = line.source.len() + line.text.len() + 128;
            let fit = budget >= size;
            budget = budget.saturating_sub(size);
            fit
        };
        if query.since.is_none() {
            let mut page: Vec<_> = wanted
                .rev()
                .take(query.limit)
                .take_while(|line| fits(line))
                .collect();
            page.reverse();
            return Ok(Page {
                lines: page,
                next: newest.unwrap_or(0),
            });
        }
        let mut wanted = wanted.peekable();
        let mut page = Vec::new();
        while page.len() < query.limit {
            match wanted.next_if(|line| fits(line)) {
                Some(line) => page.push(line),
                None => break,
            }
        }
        // With more to come, the next read continues after the last line
        // returned; otherwise after everything this one looked at.
        let next = match (wanted.peek(), page.last()) {
            (Some(_), Some(last)) => last.seq,
            _ => newest.or(query.since).unwrap_or(0),
        };
        Ok(Page { lines: page, next })
    }
}

/// The lines of one file, oldest first; a missing file has none. Text that is
/// not a line (the half line a crash left, bytes a full disk cut short) is
/// skipped: it is a diagnostic that was lost, not state to repair.
fn parse(path: &Path) -> io::Result<Vec<Line>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect())
}

fn now() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_millis() as i64,
        // A clock before 1970 has no meaningful time to record.
        Err(_) => 0,
    }
}

/// Cuts a byte stream that arrives in arbitrary chunks into log lines: at
/// each newline, and at `LINE_BYTES` when no newline comes.
#[derive(Default)]
pub struct LineSplitter {
    pending: Vec<u8>,
}

impl LineSplitter {
    /// The lines `chunk` completes; an unfinished line waits for the next chunk.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for byte in chunk {
            if *byte == b'\n' {
                lines.extend(self.take());
                continue;
            }
            self.pending.push(*byte);
            if self.pending.len() == LINE_BYTES {
                lines.extend(self.take());
            }
        }
        lines
    }

    /// What the stream left without a final newline.
    pub fn finish(mut self) -> Option<String> {
        self.take()
    }

    fn take(&mut self) -> Option<String> {
        let bytes = std::mem::take(&mut self.pending);
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim_end_matches('\r');
        if text.is_empty() {
            return None;
        }
        Some(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, text: &str) -> Entry {
        Entry {
            at: 1_700_000_000_000,
            source: source.into(),
            conversation_id: None,
            text: text.into(),
        }
    }

    fn query(since: Option<u64>, limit: usize, source: Option<&str>) -> Query {
        Query {
            since,
            limit,
            source: source.map(str::to_owned),
        }
    }

    fn texts(page: &Page) -> Vec<&str> {
        page.lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn reads_the_newest_lines_then_continues_from_the_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = Files::open(directory.path().into()).unwrap();
        for index in 0..5 {
            files.append(entry(RUNNER, &format!("line {index}")));
        }
        let tail = files.read(&query(None, 2, None)).unwrap();
        assert_eq!(texts(&tail), ["line 3", "line 4"]);
        assert_eq!(
            files.read(&query(Some(tail.next), 10, None)).unwrap().lines,
            []
        );
        files.append(entry(RUNNER, "line 5"));
        files.append(entry(RUNNER, "line 6"));
        let first = files.read(&query(Some(tail.next), 1, None)).unwrap();
        assert_eq!(texts(&first), ["line 5"]);
        let second = files.read(&query(Some(first.next), 1, None)).unwrap();
        assert_eq!(texts(&second), ["line 6"]);
        let all = files.read(&query(Some(0), 1000, None)).unwrap();
        assert_eq!(all.lines.len(), 7);
        assert_eq!(all.next, second.next);
    }

    #[test]
    fn an_empty_log_answers_no_lines_and_keeps_the_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let files = Files::open(directory.path().into()).unwrap();
        assert_eq!(
            files.read(&query(None, 10, None)).unwrap(),
            Page {
                lines: vec![],
                next: 0
            }
        );
        assert_eq!(files.read(&query(Some(7), 10, None)).unwrap().next, 7);
    }

    #[test]
    fn keeps_one_source_and_moves_the_cursor_past_the_others() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = Files::open(directory.path().into()).unwrap();
        files.append(entry("service:demi.builtin", "tabs failed"));
        files.append(entry(RUNNER, "online"));
        files.append(Entry {
            conversation_id: Some("conversation".into()),
            ..entry("stream:browser.live", "no tabs")
        });
        files.append(entry(RUNNER, "stream ended"));
        let page = files
            .read(&query(Some(0), 10, Some("stream:browser.live")))
            .unwrap();
        assert_eq!(texts(&page), ["no tabs"]);
        assert_eq!(
            page.lines[0].conversation_id.as_deref(),
            Some("conversation")
        );
        let newest = files.read(&query(None, 1, None)).unwrap().next;
        assert_eq!(page.next, newest);
        let tail = files.read(&query(None, 1, Some(RUNNER))).unwrap();
        assert_eq!(texts(&tail), ["stream ended"]);
    }

    #[test]
    fn replaces_the_older_file_and_a_cursor_survives_rotation_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = Files::open(directory.path().into()).unwrap();
        let text = "x".repeat(LINE_BYTES);
        files.append(entry(RUNNER, "first"));
        let cursor = files.read(&query(None, 1, None)).unwrap().next;
        // Enough to fill the newer file once but not twice.
        let count = FILE_BYTES as usize / LINE_BYTES + 10;
        for _ in 0..count {
            files.append(entry(RUNNER, &text));
        }
        let older = std::fs::metadata(directory.path().join(OLDER))
            .unwrap()
            .len();
        let newer = std::fs::metadata(directory.path().join(NEWER))
            .unwrap()
            .len();
        assert!(older <= FILE_BYTES && older > FILE_BYTES - 2 * LINE_BYTES as u64);
        assert!(newer <= FILE_BYTES);
        let mut seen = 0;
        let mut since = cursor;
        loop {
            let page = files.read(&query(Some(since), 1000, None)).unwrap();
            if page.lines.is_empty() {
                break;
            }
            assert_eq!(page.lines[0].seq, since + 1);
            seen += page.lines.len();
            since = page.next;
        }
        assert_eq!(seen, count);

        // A restart continues the numbering.
        drop(files);
        let mut files = Files::open(directory.path().into()).unwrap();
        files.append(entry(RUNNER, "after restart"));
        let page = files.read(&query(Some(since), 10, None)).unwrap();
        assert_eq!(texts(&page), ["after restart"]);
        assert_eq!(page.lines[0].seq, since + 1);

        // Fill it again: the first lines are gone, and their cursor continues
        // from the oldest line still kept.
        for _ in 0..2 * count {
            files.append(entry(RUNNER, &text));
        }
        assert!(
            std::fs::metadata(directory.path().join(OLDER))
                .unwrap()
                .len()
                <= FILE_BYTES
        );
        let oldest = files.read(&query(Some(0), 1, None)).unwrap().lines[0].seq;
        assert!(oldest > cursor + 1);
        let expired = files.read(&query(Some(cursor), 1, None)).unwrap();
        assert_eq!(expired.lines[0].seq, oldest);
    }

    #[test]
    fn a_cursor_from_a_log_that_is_gone_starts_at_the_oldest_line() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = Files::open(directory.path().into()).unwrap();
        files.append(entry(RUNNER, "old"));
        let cursor = files.read(&query(None, 1, None)).unwrap().next;
        drop(files);
        std::fs::remove_file(directory.path().join(NEWER)).unwrap();
        // The new log starts at the clock, which must have moved on.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut files = Files::open(directory.path().into()).unwrap();
        files.append(entry(RUNNER, "new"));
        let page = files.read(&query(Some(cursor), 10, None)).unwrap();
        assert_eq!(texts(&page), ["new"]);
        let ahead = files.read(&query(Some(u64::MAX), 10, None)).unwrap();
        assert_eq!(texts(&ahead), ["new"]);
    }

    #[test]
    fn skips_half_a_line_and_ends_it_before_the_next() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = Files::open(directory.path().into()).unwrap();
        files.append(entry(RUNNER, "whole"));
        drop(files);
        let mut file = OpenOptions::new()
            .append(true)
            .open(directory.path().join(NEWER))
            .unwrap();
        file.write_all(b"{\"seq\":").unwrap();
        drop(file);
        let mut files = Files::open(directory.path().into()).unwrap();
        files.append(entry(RUNNER, "next"));
        let page = files.read(&query(Some(0), 10, None)).unwrap();
        assert_eq!(texts(&page), ["whole", "next"]);
        assert_eq!(page.lines[1].seq, page.lines[0].seq + 1);
    }

    #[test]
    fn a_page_stops_at_its_byte_budget_and_the_cursor_continues() {
        let directory = tempfile::tempdir().unwrap();
        let mut files = Files::open(directory.path().into()).unwrap();
        let text = "x".repeat(LINE_BYTES);
        let count = 1000;
        for _ in 0..count {
            files.append(entry(RUNNER, &text));
        }
        let first = files.read(&query(Some(0), 1000, None)).unwrap();
        assert!(first.lines.len() < count);
        assert!(first.lines.len() * LINE_BYTES <= PAGE_BYTES);
        let second = files.read(&query(Some(first.next), 1000, None)).unwrap();
        assert_eq!(second.lines[0].seq, first.lines.last().unwrap().seq + 1);
        let tail = files.read(&query(None, 1000, None)).unwrap();
        assert!(tail.lines.len() < count);
        assert_eq!(tail.lines.last().unwrap().seq, tail.next);
    }

    #[test]
    fn splits_chunks_that_do_not_end_at_a_newline() {
        let mut splitter = LineSplitter::default();
        assert_eq!(splitter.push(b"first li"), Vec::<String>::new());
        assert_eq!(
            splitter.push(b"ne\r\nsecond\n\nthi"),
            ["first line", "second"]
        );
        assert_eq!(splitter.push(b"rd"), Vec::<String>::new());
        assert_eq!(splitter.finish().as_deref(), Some("third"));

        let mut splitter = LineSplitter::default();
        let long = vec![b'y'; LINE_BYTES + 3];
        let lines = splitter.push(&long);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), LINE_BYTES);
        assert_eq!(splitter.finish().as_deref(), Some("yyy"));
        assert_eq!(LineSplitter::default().finish(), None);
    }

    /// Emits `events` into a subscriber made of `layer` alone.
    fn emit(layer: HostLogLayer, events: impl FnOnce()) {
        use tracing_subscriber::layer::SubscriberExt as _;
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), events);
    }

    #[tokio::test]
    async fn events_become_lines_in_order_and_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let (writer, layer) = open(directory.path().join("log")).await.unwrap();
        emit(layer, || {
            tracing::info!(source = "service:demi.builtin", "could not list tabs");
            tracing::warn!(source = "stream:browser.live", conversation = "c1", "view failed\nretrying");
            tracing::info!(pid = 7, "service started");
        });
        let page = writer.reader().read(query(None, 10, None)).await.unwrap();
        assert_eq!(
            texts(&page),
            ["could not list tabs", "view failed", "retrying", "service started pid=7"]
        );
        assert_eq!(page.lines[0].source, "service:demi.builtin");
        assert_eq!(page.lines[1].conversation_id.as_deref(), Some("c1"));
        assert_eq!(page.lines[3].source, RUNNER);
        writer.close().await;

        let (reopened, layer) = open(directory.path().join("log")).await.unwrap();
        emit(layer, || tracing::info!("restarted"));
        let reader = reopened.reader();
        let page = reader.read(query(Some(page.next), 10, None)).await.unwrap();
        assert_eq!(texts(&page), ["restarted"]);
        reopened.close().await;
        assert!(reader.read(query(None, 10, None)).await.is_err());
    }

    #[tokio::test]
    async fn a_full_queue_drops_lines_and_says_how_many() {
        let directory = tempfile::tempdir().unwrap();
        let files = Files::open(directory.path().into()).unwrap();
        let dropped = Arc::new(AtomicU64::new(0));
        let (queue, queued) = mpsc::channel(QUEUE);
        let layer = HostLogLayer {
            queue: queue.clone(),
            dropped: dropped.clone(),
        };
        // Nothing drains the queue yet: the lines past it are counted.
        emit(layer, || {
            for index in 0..QUEUE + 3 {
                tracing::info!("line {index}");
            }
        });
        assert_eq!(dropped.load(Ordering::Relaxed), 3);
        let thread = std::thread::spawn(move || serve(files, queued, &dropped));
        queue.send(Message::Close).await.unwrap();
        thread.join().unwrap();
        let files = Files::open(directory.path().into()).unwrap();
        let page = files.read(&query(None, 3, None)).unwrap();
        assert_eq!(texts(&page)[0], format!("line {}", QUEUE - 3));
        let all = files.read(&query(Some(0), QUEUE + 10, None)).unwrap();
        assert_eq!(all.lines[0].text, "3 log lines were dropped: the log fell behind");
    }
}
