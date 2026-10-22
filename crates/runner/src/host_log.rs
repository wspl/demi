//! The Host's log: the runner's `tracing` events as [`LogLine`]s, kept in two
//! files on the Host and sent to the backend, which stores them in its log.
//!
//! ```text
//!   tracing events --- layer --> queue --> writer thread --> host.log, host.log.1
//!                                              |  (newest lines also in memory)
//!                                              v
//!   backend <-- log_lines (lowest-priority queue) <-- follower <-- log_follow
//! ```
//!
//! Without it a resident service that fails on a Cloud and writes why to its
//! standard error leaves nothing once the Cloud is gone.
//!
//! # How lines get in
//!
//! Every part of the runner writes `tracing` events. The log is a layer that
//! writes them as lines, so no module holds a handle to the files, and a
//! resident service's standard error enters the same way, as `stderr` lines,
//! except a line the service wrote as a `LogLine` itself
//! (`demi_command_sdk::log_to_stderr`), which is kept as it is.
//! An event names what happened in its `event` field, `noun.verb` in the past
//! tense, and its IDs (`conversation`, `turn`, `request_id`) go to the line's
//! own fields; the backend adds the Host's ID.
//!
//! Writing never holds up the work it describes:
//!
//! - The layer queues each line, and one writer thread owns the files.
//! - A line the queue has no room for is dropped and counted: the log says so
//!   with a `log.dropped` line, and the next frame to the backend counts it
//!   as missed.
//! - The writer answers the follower's reads between two lines.
//!
//! # How lines reach the backend
//!
//! The files are a buffer for as long as the backend cannot be reached. After
//! the backend's `log_follow { since }` the follower sends the lines after
//! `since`, then each new line, in `log_lines` frames of at most
//! [`LOG_FRAME_BYTES`], on the connection's lowest-priority queue: a command's
//! exit or an answer leaves before a log frame. A frame's `missed` counts the
//! lines the files no longer held and those the queue dropped since the last
//! one.
//!
//! A paired device keeps the files in `log/` of its installation state. A
//! managed guest's state is temporary, so it keeps them in `/var/log/demi` on
//! the system layer, which outlives a stop.
//!
//! # What goes in
//!
//! What a source writes is IDs, counts, durations and states, and the
//! diagnostics of what it tried and why it failed. Page content, typed text,
//! cookies, tokens and file contents never go to the log.

use std::{
    collections::VecDeque,
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

use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::layer::{Context, Layer};

use demi_runner_process::private_files::chmod;
use demi_runner_protocol::wire::{
    self, HostLogLine, LOG_FRAME_BYTES, LogLevel, LogLine, LogOrigin,
};

const NEWER: &str = "host.log";
const OLDER: &str = "host.log.1";
/// Lines waiting for the writer; past it a line is dropped and counted, since
/// logging never holds up the work it records.
const QUEUE: usize = 1024;
/// Each of the two files holds this much, since the Host's disk is not the
/// log's: the older file is replaced when the newer one fills.
const FILE_BYTES: u64 = 4 * 1024 * 1024;
/// The newest lines the writer also keeps in memory, so that a follower that
/// is up to date reads them without reading the files for each new line; the
/// files hold the rest, so memory does not grow with the log.
const MEMORY_LINES: usize = 2048;
/// The most bytes of a line's text: a longer one is cut, since a line must fit
/// a frame of at most [`LOG_FRAME_BYTES`].
const TEXT_BYTES: usize = 8 * 1024;

/// What a read of the log after a cursor gives.
#[derive(Debug, PartialEq)]
pub struct Page {
    /// Oldest first.
    pub lines: Vec<HostLogLine>,
    /// The lines the files no longer held, between the cursor and these.
    pub missed: u64,
}

/// What the writer thread is asked, in the order asked.
enum Message {
    Line(LogLine),
    Read {
        since: Option<u64>,
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
    let files = tokio::task::spawn_blocking(move || Files::open(directory, FILE_BYTES))
        .await
        .map_err(io::Error::other)??;
    let dropped = Arc::new(AtomicU64::new(0));
    let lost = Arc::new(AtomicU64::new(0));
    let (latest, newest) = watch::channel(files.last.unwrap_or(0));
    let (queue, queued) = mpsc::channel(QUEUE);
    let thread = std::thread::Builder::new().name("host-log".into()).spawn({
        let dropped = dropped.clone();
        let lost = lost.clone();
        move || serve(files, queued, &dropped, &lost, &latest)
    })?;
    Ok((
        HostLogWriter {
            queue: queue.clone(),
            newest,
            lost,
            thread,
        },
        HostLogLayer { queue, dropped },
    ))
}

/// The writer thread: it writes each queued line, first saying how many the
/// queue had no room for, and answers each read from the files as they are.
fn serve(
    mut files: Files,
    mut queued: mpsc::Receiver<Message>,
    dropped: &AtomicU64,
    lost: &AtomicU64,
    latest: &watch::Sender<u64>,
) {
    while let Some(message) = queued.blocking_recv() {
        match message {
            Message::Line(line) => {
                let missed = dropped.swap(0, Ordering::Relaxed);
                if missed > 0 {
                    lost.fetch_add(missed, Ordering::Relaxed);
                    let mut dropping =
                        LogLine::new(line.at, LogOrigin::Runner, LogLevel::Warn, "log.dropped");
                    dropping.fields.insert("dropped".into(), missed.into());
                    dropping
                        .fields
                        .insert("why".into(), "the writer fell behind".into());
                    files.append(dropping);
                }
                files.append(line);
                if let Some(last) = files.last {
                    latest.send_replace(last);
                }
            }
            Message::Read { since, reply } => {
                // The reader may have gone; the read cost nothing to keep.
                let _gone = reply.send(files.read_after(since));
            }
            Message::Close => return,
        }
    }
}

/// Owns the writer thread.
pub struct HostLogWriter {
    queue: mpsc::Sender<Message>,
    newest: watch::Receiver<u64>,
    lost: Arc<AtomicU64>,
    thread: std::thread::JoinHandle<()>,
}

impl HostLogWriter {
    /// What a connection follows the log with.
    pub fn follower(&self) -> HostLogFollower {
        HostLogFollower {
            queue: self.queue.clone(),
            newest: self.newest.clone(),
            lost: self.lost.clone(),
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
            crate::console::line("demi-runner: the host log writer failed");
        }
    }
}

/// Reads the log after a cursor and waits for new lines, for a connection's
/// follow.
#[derive(Clone)]
pub struct HostLogFollower {
    queue: mpsc::Sender<Message>,
    newest: watch::Receiver<u64>,
    lost: Arc<AtomicU64>,
}

impl HostLogFollower {
    /// The lines after `since` (all lines for none), counting the lines the
    /// queue dropped since the last read as missed. The writer
    /// thread answers between two lines it writes, so the page holds every
    /// line queued before the read.
    pub async fn read_after(&self, since: Option<u64>) -> io::Result<Page> {
        let (reply, answer) = oneshot::channel();
        let closed = || io::Error::other("the host log is closed");
        self.queue
            .send(Message::Read { since, reply })
            .await
            .map_err(|_| closed())?;
        let mut page = answer.await.map_err(|_| closed())??;
        page.missed += self.lost.swap(0, Ordering::Relaxed);
        Ok(page)
    }

    /// Returns once a line after `since` may exist.
    pub async fn wait_after(&mut self, since: Option<u64>) {
        let after = since.unwrap_or(0);
        // The writer is gone only when the log closed with the process.
        let _closed = self.newest.wait_for(|newest| *newest > after).await;
    }
}

/// The layer: each event it is given becomes a line in the files.
pub struct HostLogLayer {
    queue: mpsc::Sender<Message>,
    dropped: Arc<AtomicU64>,
}

impl<S: Subscriber> Layer<S> for HostLogLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let line = line_of(event);
        if self.queue.try_send(Message::Line(line)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// An event as a line: its `event` field names it, else its target does; an
/// event of a resident service's standard error (it has a `source`) is a
/// `stderr` line with the text in its fields. A line a service wrote as a
/// [`LogLine`] arrives whole in the `service_line` field
/// (`crates/runner-command-packages/src/process.rs`) and is kept as it is.
fn line_of(event: &Event<'_>) -> LogLine {
    let mut collected = Collected::default();
    event.record(&mut collected);
    let metadata = event.metadata();
    let mut values = collected.values;
    if let Some(Value::String(written)) = values.remove("service_line")
        && let Ok(mut line) = serde_json::from_str::<LogLine>(&written)
    {
        line.origin = LogOrigin::Runner;
        return line;
    }
    let mut refused = false;
    let mut text = |name: &str| match values.remove(name) {
        Some(Value::String(text)) => Some(text),
        Some(other) => Some(other.to_string()),
        None => None,
    };
    // Only a string that is a name is the event's; any other value logged
    // under `event`, a payload as much as a typo, is left out of the line.
    let named = match text("event") {
        Some(name) if wire::is_event_name(&name) => Some(name),
        Some(_) => {
            refused = true;
            None
        }
        None => None,
    };
    let source = text("source").filter(|source| source != "runner");
    let conversation = text("conversation").or_else(|| text("conversation_id"));
    let turn = text("turn").or_else(|| text("turn_id"));
    let request = text("request_id");
    let (event_name, message_key) = match (&named, &source) {
        (Some(name), _) => (name.clone(), "message"),
        (None, Some(_)) => ("stderr".to_owned(), "text"),
        (None, None) => (metadata.target().to_owned(), "message"),
    };
    let mut line = LogLine::new(now(), LogOrigin::Runner, level_of(*metadata.level()), event_name);
    line.conversation_id = conversation;
    line.turn_id = turn;
    line.request_id = request;
    if refused {
        values.insert("event_refused".into(), true.into());
    }
    if let Some(source) = source {
        values.insert("source".into(), source.into());
    }
    if !collected.message.is_empty() {
        values.insert(message_key.into(), cut(collected.message).into());
    }
    line.fields = values;
    line
}

/// `text`, cut to [`TEXT_BYTES`] at a character.
fn cut(mut text: String) -> String {
    if text.len() > TEXT_BYTES {
        let mut end = TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

fn level_of(level: Level) -> LogLevel {
    match level {
        Level::TRACE => LogLevel::Trace,
        Level::DEBUG => LogLevel::Debug,
        Level::INFO => LogLevel::Info,
        Level::WARN => LogLevel::Warn,
        Level::ERROR => LogLevel::Error,
    }
}

/// An event's message and other fields, each as JSON.
#[derive(Default)]
struct Collected {
    message: String,
    values: Map<String, Value>,
}

impl Visit for Collected {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.values.insert(field.name().into(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.values.insert(field.name().into(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.values.insert(field.name().into(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.values.insert(field.name().into(), value.into());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.values.insert(field.name().into(), value.into());
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        let mut text = value.to_string();
        let mut source = value.source();
        while let Some(cause) = source {
            // Writing to a String does not fail.
            let _ = write!(text, ": {cause}");
            source = cause.source();
        }
        self.values.insert(field.name().into(), text.into());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let text = format!("{value:?}");
        if field.name() == "message" {
            self.message = text;
        } else {
            self.values.insert(field.name().into(), text.into());
        }
    }
}

/// The console layer: each event it is given goes to standard error, which a
/// guest's console shows. Only what was tried and why it failed belongs there:
/// never a token, a pairing code or content.
pub struct Console;

impl<S: Subscriber> Layer<S> for Console {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut visitor = ConsoleText::default();
        event.record(&mut visitor);
        crate::console::line(crate::console::runner_line(visitor.message + &visitor.others));
    }
}

/// An event's message followed by any other fields.
#[derive(Default)]
struct ConsoleText {
    message: String,
    others: String,
}

impl Visit for ConsoleText {
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
    file_bytes: u64,
    next_seq: u64,
    /// The `seq` of the newest line, none for a log with no line.
    last: Option<u64>,
    /// The newest lines, oldest first.
    memory: VecDeque<HostLogLine>,
    /// The newer file, opened for append, and its length.
    newer: Option<(File, u64)>,
    /// The last write failed.
    failing: bool,
}

impl Files {
    fn open(directory: PathBuf, file_bytes: u64) -> io::Result<Self> {
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
            file_bytes,
            next_seq,
            last,
            memory: VecDeque::new(),
            newer: None,
            failing: false,
        })
    }

    /// A write that fails (a full disk, a removed directory) loses the line
    /// and nothing else: the file is reopened for the next line. The console
    /// learns of it once, not once per lost line.
    fn append(&mut self, line: LogLine) {
        match self.try_append(line) {
            Ok(()) => self.failing = false,
            Err(error) => {
                self.newer = None;
                if !self.failing {
                    crate::console::line(format_args!("demi-runner: host log write failed: {error}"));
                }
                self.failing = true;
            }
        }
    }

    fn try_append(&mut self, line: LogLine) -> io::Result<()> {
        let line = HostLogLine {
            seq: self.next_seq,
            line,
        };
        let mut bytes = serde_json::to_vec(&line).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let size = bytes.len() as u64;
        if self.newer()?.1 + size > self.file_bytes {
            self.newer = None;
            std::fs::rename(self.directory.join(NEWER), self.directory.join(OLDER))?;
        }
        let (file, length) = self.newer()?;
        file.write_all(&bytes)?;
        *length += size;
        self.next_seq += 1;
        self.last = Some(line.seq);
        if self.memory.len() == MEMORY_LINES {
            self.memory.pop_front();
        }
        self.memory.push_back(line);
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

    /// The lines after `since`, from memory when it holds the line after
    /// `since` and from the files when not, and how many lines between
    /// `since` and them the files no longer held. A cursor ahead of the newest
    /// line comes from a log that is gone (a reset, a clock set back): it
    /// continues from the oldest line.
    fn read_after(&self, since: Option<u64>) -> io::Result<Page> {
        let since = since.filter(|since| self.last.is_some_and(|last| *since <= last));
        let covered = self
            .memory
            .front()
            .is_some_and(|first| since.is_some_and(|since| first.seq <= since + 1));
        if covered {
            let lines = self
                .memory
                .iter()
                .filter(|line| Some(line.seq) > since)
                .cloned()
                .collect();
            return Ok(Page { lines, missed: 0 });
        }
        let mut stored = parse(&self.directory.join(OLDER))?;
        stored.extend(parse(&self.directory.join(NEWER))?);
        let oldest = stored.first().map(|line| line.seq);
        let missed = match (since, oldest) {
            (Some(since), Some(oldest)) if oldest > since + 1 => oldest - (since + 1),
            _ => 0,
        };
        stored.retain(|line| since.is_none_or(|since| line.seq > since));
        Ok(Page {
            lines: stored,
            missed,
        })
    }
}

/// The lines of one file, oldest first; a missing file has none. Text that is
/// not a line (the half line a crash left, bytes a full disk cut short, a line
/// of an older runner's shape) is skipped: it is a diagnostic that was lost,
/// not state to repair.
fn parse(path: &Path) -> io::Result<Vec<HostLogLine>> {
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

/// `page` as the frames the wire carries it in: lines up to
/// [`LOG_FRAME_BYTES`] each (a line over it is alone in its frame), the missed
/// count in the first.
pub fn frames(page: Page) -> Result<Vec<(wire::Frame, u64, u64)>, wire::WireError> {
    let mut frames = Vec::new();
    let mut missed = page.missed;
    let mut lines = Vec::new();
    let mut bytes = 0;
    let mut finish = |lines: &mut Vec<HostLogLine>, missed: &mut u64| -> Result<(), wire::WireError> {
        let count = lines.len() as u64;
        let frame = wire::encode(&wire::Outbound::LogLines {
            lines: std::mem::take(lines),
            missed: *missed,
        })?;
        frames.push((frame, count, *missed));
        *missed = 0;
        Ok(())
    };
    for line in page.lines {
        let size = serde_json::to_vec(&line).map_or(0, |bytes| bytes.len());
        if !lines.is_empty() && bytes + size > LOG_FRAME_BYTES {
            finish(&mut lines, &mut missed)?;
            bytes = 0;
        }
        bytes += size;
        lines.push(line);
    }
    if !lines.is_empty() || missed > 0 {
        finish(&mut lines, &mut missed)?;
    }
    Ok(frames)
}
