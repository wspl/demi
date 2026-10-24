//! A device's log, followed into memory from the moment the device opens
//! until it closes, across the connections it takes meanwhile, and read
//! after a cursor as `demi browser logs` reads a tab's console.
//!
//! The drivers follow the platform's log and push each entry here; a read
//! filters what the buffer kept. Nothing goes to the disk: the buffer
//! keeps the last [`LOG_BYTES`] of entries and counts the ones it let go.

use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use demi_command_package_mobile_protocol::mobile::{DeviceId, LOG_BYTES, LOG_ENTRIES, LogEntry, LogsInput, LogsResult};
use demi_shared_types::buffer::{Buffer, BufferError};

use crate::error::MobileError;

/// Which entries are one app's, as its platform tells its processes apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEntries {
    /// Android's: the app's user id, which every process of it has.
    Uid(u32),
    /// iOS's: the app's executable, which the unified log names its
    /// processes by.
    Process(String),
}

impl AppEntries {
    fn holds(&self, entry: &LogEntry) -> bool {
        match self {
            Self::Uid(uid) => entry.uid == Some(*uid),
            Self::Process(process) => entry.process.as_deref() == Some(process.as_str()),
        }
    }
}

pub struct Log {
    buffer: Mutex<Buffer<LogEntry>>,
    /// When the device opened, in milliseconds since the epoch: the log is
    /// followed from then.
    since: f64,
    /// Cancelled once the device closes or is lost, which ends what follows
    /// it across its connections, as the console of an app Demi launched.
    closed: CancellationToken,
}

impl Log {
    pub fn new() -> Self {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        Self {
            // A cursor of another log, as one of the device before it was
            // opened again, names another generation.
            buffer: Mutex::new(Buffer::new(format!("{:x}", now.as_micros()), usize::MAX, LOG_BYTES)),
            since: now.as_secs_f64() * 1000.0,
            closed: CancellationToken::new(),
        }
    }

    /// Cancelled once the device closes or is lost.
    pub fn closed(&self) -> &CancellationToken {
        &self.closed
    }

    /// Adds `entry` at the log's next position, whatever its own says.
    pub fn push(&self, entry: LogEntry) {
        let mut buffer = self.buffer.lock().expect("the log is intact");
        if let Err(error) = buffer.push(entry) {
            // An entry that cannot be kept is a gap a read counts.
            let _counted = buffer.mark_gap();
            tracing::warn!(event = "mobile_log.entry_dropped", %error, "a log entry was not kept");
        }
    }

    /// The time the log is to be followed from as a follower starts again:
    /// its last entry's, or the device's open.
    pub fn resume_at(&self) -> f64 {
        let buffer = self.buffer.lock().expect("the log is intact");
        buffer.entries().next_back().map_or(self.since, |entry| entry.timestamp.max(self.since))
    }

    /// The entries `input` asks for, of the app `app` when it names one.
    pub fn read(&self, device: &DeviceId, input: &LogsInput, app: Option<&AppEntries>) -> Result<LogsResult, MobileError> {
        let buffer = self.buffer.lock().expect("the log is intact");
        let after = input.after.as_deref().map(|cursor| buffer.position(cursor)).transpose().map_err(stale)?;
        let matching: Vec<&LogEntry> = buffer
            .entries()
            .filter(|entry| after.is_none_or(|after| entry.sequence >= after))
            .filter(|entry| input.level.is_none_or(|level| entry.level >= level))
            .filter(|entry| app.is_none_or(|app| app.holds(entry)))
            .filter(|entry| {
                input.filter.as_deref().is_none_or(|filter| {
                    entry.text.contains(filter) || entry.tag.as_deref().is_some_and(|tag| tag.contains(filter))
                })
            })
            .collect();
        let limit = input.limit.unwrap_or(LOG_ENTRIES);
        // Without a cursor, the last entries; after one, the first after it.
        let start = if after.is_none() { matching.len().saturating_sub(limit) } else { 0 };
        let entries: Vec<LogEntry> = matching.iter().skip(start).take(limit).map(|entry| (*entry).clone()).collect();
        let has_more = start + entries.len() < matching.len();
        let next = match entries.last() {
            Some(last) if has_more => last.sequence + 1,
            _ => buffer.next(),
        };
        Ok(LogsResult {
            device: device.clone(),
            entries,
            cursor: buffer.cursor(next),
            has_more,
            dropped: after.map_or(0, |after| buffer.dropped_since(after)),
        })
    }
}

fn stale(error: BufferError) -> MobileError {
    match error {
        BufferError::StaleCursor => MobileError::Usage(
            "the cursor is not one of this device's log, as one from before it was opened again; read without --after for the last entries".into(),
        ),
        other => MobileError::Failed(other.to_string()),
    }
}

/// A log entry of the platform's, its position left to the log.
pub fn entry(timestamp: f64, level: demi_command_package_mobile_protocol::mobile::LogLevel, pid: u32, text: String) -> LogEntry {
    LogEntry {
        sequence: 0,
        timestamp,
        level,
        pid,
        uid: None,
        process: None,
        tag: None,
        text,
    }
}

