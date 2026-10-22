//! The log: every line `DEMI_LOG` selects of the backend's, every line of each
//! Host's runner and of each page, as a row of `log.sqlite` in the data directory, which whoever
//! diagnoses reads with `sqlite3` (`docs/delivery/diagnosis.md`).
//!
//! ```text
//!   backend `tracing` events --- layer ---+
//!   a runner's `log_lines` frames --------+--> queue --> writer thread --> log.sqlite
//!   a page's `POST /log` lines -----------+     (4096)    commits every 200 ms,
//!      (only the levels DEMI_LOG selects          trims by batches
//!       for the target `page`)
//! ```
//!
//! - A `tracing` layer turns each backend event into a row and queues it
//!   without waiting; the runner's and the page's lines enter the same queue
//!   through [`ServerLog`].
//! - One writer thread owns the database. It inserts what the queue holds in
//!   one transaction at most every 200 milliseconds, and each hour deletes,
//!   in batches of 10,000 lines between which it goes on writing, what is
//!   older than [`Limits::keep`] or over [`Limits::max_bytes`].
//! - The log is a record for diagnosis, not state: a database the writer
//!   cannot open or write, or of an older schema, is removed and created
//!   again, never migrated.
//! - The log logs itself: a queue that dropped lines, a trim, a database
//!   created again and the lines a runner no longer held are lines of it.
//! - Nothing writes a request's body or query string, a credential, a secret's
//!   value or a file's contents to the log.

mod layer;
mod store;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use demi_shared_types::{HostLogLine, LogLevel, LogLine, LogOrigin};
use tracing::Level;
use tracing_subscriber::filter::Targets;

pub use layer::ServerLogLayer;

/// The target `DEMI_LOG` names the page's lines by, as `info,page=debug` does.
pub const PAGE_TARGET: &str = "page";

/// The database's file in the data directory.
pub const DATABASE: &str = "log.sqlite";

/// Events waiting for the writer; past it an event is dropped and counted,
/// since logging never holds up the work it records.
const QUEUE: usize = 4096;

/// How long and how much the log keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long a line is kept, since the server's disk is finite.
    pub keep: Duration,
    /// The most the lines may take of the database, for the same reason.
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            keep: Duration::from_secs(14 * 24 * 60 * 60),
            max_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

/// One line as a row: the line, and its number in its Host's files for a
/// runner's.
#[derive(Debug, Clone)]
struct Row {
    line: LogLine,
    seq: Option<u64>,
}

/// What the writer thread is told, in the order told.
enum Message {
    Row(Row),
    Close,
}

/// Starts the log of the data directory `data_dir`: the writer thread, and
/// the layer that goes into the process's `tracing` subscriber.
///
/// `filter` is `DEMI_LOG`: a page's lines are stored at the levels it selects
/// for the target [`PAGE_TARGET`], as the backend's own are for theirs.
pub fn open(
    data_dir: &Path,
    limits: Limits,
    filter: Targets,
) -> io::Result<(ServerLogWriter, ServerLogLayer)> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(DATABASE);
    let dropped = Arc::new(AtomicU64::new(0));
    let (queue, queued) = mpsc::sync_channel(QUEUE);
    let thread = std::thread::Builder::new()
        .name("server-log".into())
        .spawn({
            let dropped = dropped.clone();
            let path = path.clone();
            move || store::serve(&path, limits, &queued, &dropped)
        })?;
    let sink = Sink { queue, dropped };
    Ok((
        ServerLogWriter {
            log: ServerLog {
                path,
                sink: Some(sink.clone()),
                filter: Some(Arc::new(filter)),
            },
            thread,
        },
        ServerLogLayer::new(sink),
    ))
}

/// Where lines wait for the writer.
#[derive(Clone)]
struct Sink {
    queue: mpsc::SyncSender<Message>,
    dropped: Arc<AtomicU64>,
}

impl Sink {
    /// Queues `row` without waiting; one the queue has no room for is dropped
    /// and counted, and the writer says so with its next commit.
    fn push(&self, row: Row) {
        if self.queue.try_send(Message::Row(row)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Owns the writer thread.
pub struct ServerLogWriter {
    log: ServerLog,
    thread: std::thread::JoinHandle<()>,
}

impl ServerLogWriter {
    /// The handle the backend's parts store lines with.
    pub fn log(&self) -> ServerLog {
        self.log.clone()
    }

    /// Writes every event queued before it and stops the writer thread;
    /// events logged afterwards go to standard error alone.
    pub async fn close(self) {
        let Self { log, thread } = self;
        let closed = tokio::task::spawn_blocking(move || {
            // The thread is gone only if it panicked; nothing is left to
            // write then.
            if let Some(sink) = &log.sink {
                let _gone = sink.queue.send(Message::Close);
            }
            thread.join()
        })
        .await;
        if !matches!(closed, Ok(Ok(()))) {
            eprintln!("demi-backend: the server log's writer failed");
        }
    }
}

/// The log of a data directory, which the runners' and the pages' lines are
/// stored through. Cloning it is cheap.
#[derive(Clone)]
pub struct ServerLog {
    path: PathBuf,
    sink: Option<Sink>,
    /// What `DEMI_LOG` selects, which a page's lines are held to.
    filter: Option<Arc<Targets>>,
}

impl ServerLog {
    /// A log with no writer, which stores nothing, as a backend started
    /// without [`open`] has.
    pub fn detached(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(DATABASE),
            sink: None,
            filter: None,
        }
    }

    /// Stores a runner's frame of lines as the Host `host_id`'s, which `owner`
    /// owns; `missed` is the count of lines the runner no longer held.
    ///
    /// A runner is a Host's program and may be wrong or hostile, so its lines
    /// are the runner's, its Host's and its owner's, whatever they claim:
    /// - the user is the owner, and a request, a turn or a page the line names
    ///   is not taken, since nothing here can check it;
    ///   The follow resumes from the highest `seq` stored for the Host, so a
    ///   runner sends no line below it.
    pub fn store_host(&self, host_id: &str, owner: &str, lines: Vec<HostLogLine>, missed: u64) {
        if missed > 0 {
            tracing::warn!(event = "log.missed", host_id, missed, "a runner no longer held lines of its log");
        }
        let Some(sink) = &self.sink else {
            return;
        };
        for HostLogLine { seq, mut line } in lines {
            line.origin = LogOrigin::Runner;
            line.host_id = Some(host_id.to_owned());
            line.user_id = Some(owner.to_owned());
            line.request_id = None;
            line.page_id = None;
            line.turn_id = None;
            sink.push(Row { line, seq: Some(seq) });
        }
    }

    /// Stores the lines of a page's `POST /log`, which the route has
    /// checked and given their page, user and origin.
    ///
    /// A line below the level `DEMI_LOG` selects for [`PAGE_TARGET`] is not
    /// stored, as a backend event below it is not: a page writes `debug` lines
    /// for every scroll, which the log keeps only when asked.
    pub fn store_page(&self, lines: Vec<LogLine>) {
        let Some(sink) = &self.sink else {
            return;
        };
        for line in lines {
            let selected = self
                .filter
                .as_ref()
                .is_none_or(|filter| filter.would_enable(PAGE_TARGET, &tracing_level(line.level)));
            if selected {
                sink.push(Row { line, seq: None });
            }
        }
    }

    /// The highest `seq` stored for the Host's runner lines; none for a Host
    /// with no line, or a log not yet written.
    pub async fn host_since(&self, host_id: &str) -> Option<u64> {
        let path = self.path.clone();
        let host = host_id.to_owned();
        let read = tokio::task::spawn_blocking(move || store::last_seq(&path, &host)).await;
        match read {
            Ok(Ok(seq)) => seq,
            Ok(Err(error)) => {
                tracing::warn!(event = "log.since_failed", host_id, error = &error as &dyn std::error::Error, "a Host's last stored line could not be read");
                None
            }
            Err(_) => None,
        }
    }
}

fn tracing_level(level: LogLevel) -> Level {
    match level {
        LogLevel::Trace => Level::TRACE,
        LogLevel::Debug => Level::DEBUG,
        LogLevel::Info => Level::INFO,
        LogLevel::Warn => Level::WARN,
        LogLevel::Error => Level::ERROR,
    }
}
