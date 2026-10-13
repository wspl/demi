//! The server log (`backend.md` § Request IDs and the server log): every
//! event `DEMI_LOG` selects, as a row of `log.sqlite` in the data directory,
//! which an administrator searches by request. A `tracing` layer turns each
//! event into a row and queues it without waiting; one writer thread owns the
//! database, inserts what the queue holds in one transaction at most every
//! 200 milliseconds, and each hour deletes what is older than seven days or
//! over 512 MiB. The log is a record for diagnosis, not state: a database
//! the writer cannot open or write is removed and created again, never
//! migrated (`storage.md` § Schemas and migrations).

mod layer;
mod store;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc;

pub use layer::ServerLogLayer;
pub use store::{LogQuery, LogRead};

/// The database's file in the data directory.
pub const DATABASE: &str = "log.sqlite";

/// Events waiting for the writer; past it an event is dropped and counted.
const QUEUE: usize = 4096;

/// One event as a row.
#[derive(Debug, Clone)]
struct Row {
    /// Milliseconds since the Unix epoch.
    at: i64,
    level: demi_web_api_protocol::diagnostics::LogLevel,
    target: String,
    message: String,
    request_id: Option<String>,
    user_id: Option<String>,
    conversation_id: Option<String>,
    turn_id: Option<String>,
    /// The span's and the event's other fields.
    fields: serde_json::Map<String, serde_json::Value>,
}

/// What the writer thread is told, in the order told.
enum Message {
    Row(Row),
    Close,
}

/// Starts the log of the data directory `data_dir`: the writer thread, and
/// the layer that goes into the process's `tracing` subscriber.
pub fn open(data_dir: &Path) -> io::Result<(ServerLogWriter, ServerLogLayer)> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(DATABASE);
    let dropped = Arc::new(AtomicU64::new(0));
    let (queue, queued) = mpsc::sync_channel(QUEUE);
    let thread = std::thread::Builder::new()
        .name("server-log".into())
        .spawn({
            let dropped = dropped.clone();
            move || store::serve(&path, &queued, &dropped)
        })?;
    Ok((
        ServerLogWriter {
            queue: queue.clone(),
            thread,
        },
        ServerLogLayer::new(queue, dropped),
    ))
}

/// Owns the writer thread.
pub struct ServerLogWriter {
    queue: mpsc::SyncSender<Message>,
    thread: std::thread::JoinHandle<()>,
}

impl ServerLogWriter {
    /// Writes every event queued before it and stops the writer thread;
    /// events logged afterwards go to standard error alone.
    pub async fn close(self) {
        let Self { queue, thread } = self;
        let closed = tokio::task::spawn_blocking(move || {
            // The thread is gone only if it panicked; nothing is left to
            // write then.
            let _gone = queue.send(Message::Close);
            thread.join()
        })
        .await;
        if !matches!(closed, Ok(Ok(()))) {
            eprintln!("demi-backend: the server log's writer failed");
        }
    }
}

/// Reads the log of a data directory.
#[derive(Debug, Clone)]
pub struct ServerLog {
    path: PathBuf,
}

impl ServerLog {
    pub fn of(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(DATABASE),
        }
    }

    /// The lines `query` asks for, oldest first. A log not yet written, or
    /// being created again, has none.
    pub async fn read(&self, query: LogQuery) -> Result<LogRead, crate::StorageError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || store::read(&path, &query)).await?
    }
}
