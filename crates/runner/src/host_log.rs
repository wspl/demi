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

use demi_runner_process::{lines::LineSplitter, private_files::chmod};
use demi_runner_protocol::wire;

/// The source of the runner's own diagnostics, and of every event that names
/// no other.
pub const RUNNER: &str = "runner";

const NEWER: &str = "host.log";
const OLDER: &str = "host.log.1";
/// Lines waiting for the writer; past it a line is dropped and counted.
const QUEUE: usize = 1024;

/// How much the files and a page of the log hold.
#[derive(Clone, Copy)]
struct Limits {
    /// Each of the two files holds this much.
    file_bytes: u64,
    /// A read stops here and its cursor continues.
    page_bytes: usize,
}

/// Each file holds 4 MiB (`runner.md` § Host log), and a page 2 MiB, so an
/// answer stays well within the connection's message limit.
const LIMITS: Limits = Limits {
    file_bytes: 4 * 1024 * 1024,
    page_bytes: 2 * 1024 * 1024,
};

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
    chmod(&directory, 0o700).await?;
    let files = tokio::task::spawn_blocking(move || Files::open(directory, LIMITS))
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
    limits: Limits,
    next_seq: u64,
    /// The newer file, opened for append, and its length.
    newer: Option<(File, u64)>,
    /// The last write failed.
    failing: bool,
}

impl Files {
    fn open(directory: PathBuf, limits: Limits) -> io::Result<Self> {
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
            limits,
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
        if self.newer()?.1 + size > self.limits.file_bytes {
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
        let mut budget = self.limits.page_bytes;
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
