//! A tab's console history is bounded independently of any reader's cursor.
use super::{
    Result,
    history::Buffer,
    protocol::{
        CONSOLE_BYTES, CONSOLE_ENTRIES, DEFAULT_NODES, LogEntry, LogLevel, LogsInput, LogsResult,
    },
};
use chromiumoxide::{Page, cdp::js_protocol::runtime::EventConsoleApiCalled};
use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(super) struct Console {
    buffer: Buffer<LogEntry>,
}

impl Console {
    fn new() -> Result<Self> {
        Ok(Self {
            buffer: Buffer::new("logs", CONSOLE_ENTRIES, CONSOLE_BYTES)?,
        })
    }

    fn push(&mut self, event: &EventConsoleApiCalled) -> Result<()> {
        let text = event
            .args
            .iter()
            .map(|argument| match &argument.value {
                Some(Value::String(text)) => text.clone(),
                Some(value) => value.to_string(),
                None => argument
                    .unserializable_value
                    .as_ref()
                    .map(|value| value.as_ref().to_owned())
                    .or_else(|| argument.description.clone())
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        let level = match event.r#type.as_ref() {
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warning" => LogLevel::Warning,
            "error" | "assert" => LogLevel::Error,
            _ => LogLevel::Log,
        };
        let url = event
            .stack_trace
            .as_ref()
            .and_then(|stack| stack.call_frames.first())
            .map(|frame| frame.url.clone());
        self.buffer.push(LogEntry {
            sequence: 0,
            level,
            text,
            url,
            timestamp: *event.timestamp.inner(),
        })
    }

    pub fn read(&self, input: &LogsInput) -> Result<LogsResult> {
        let after = input
            .after
            .as_deref()
            .map(|cursor| self.buffer.position(cursor))
            .transpose()?;
        let entries: Vec<_> = self
            .buffer
            .entries()
            .filter(|entry| after.is_none_or(|after| entry.sequence >= after))
            .filter(|entry| {
                input
                    .level
                    .as_ref()
                    .is_none_or(|levels| levels.contains(&entry.level))
            })
            .filter(|entry| {
                input
                    .filter
                    .as_ref()
                    .is_none_or(|filter| entry.text.contains(filter))
            })
            .collect();
        let limit = input.limit.unwrap_or(DEFAULT_NODES);
        let start = if after.is_none() {
            entries.len().saturating_sub(limit)
        } else {
            0
        };
        let page: Vec<LogEntry> = entries
            .iter()
            .skip(start)
            .take(limit)
            .map(|entry| (*entry).clone())
            .collect();
        let has_more = start + page.len() < entries.len();
        let next = if has_more {
            page.last()
                .map_or(self.buffer.next(), |entry| entry.sequence + 1)
        } else {
            self.buffer.next()
        };
        Ok(LogsResult {
            entries: page,
            cursor: self.buffer.cursor(next),
            has_more,
            truncated: self.buffer.has_evicted(),
        })
    }
}

/// Subscribe before navigation; tab/environment cancellation owns the collector's lifetime.
pub(super) async fn observe(
    page: &Page,
    ended: CancellationToken,
    tasks: &TaskTracker,
) -> Result<std::sync::Arc<Mutex<Console>>> {
    let mut events = page.event_listener::<EventConsoleApiCalled>().await?;
    let state = std::sync::Arc::new(Mutex::new(Console::new()?));
    let collected = state.clone();
    tasks.spawn(async move {
        loop {
            let event = tokio::select! {
                biased;
                _ = ended.cancelled() => break,
                event = events.next() => event,
            };
            match event {
                Some(Ok(event)) => {
                    if collected.lock().await.push(&event).is_err() {
                        // An unrepresentable entry is a visible gap, never a collector crash.
                        if collected.lock().await.buffer.mark_gap().is_err() {
                            break;
                        }
                    }
                }
                Some(Err(_)) => {
                    if collected.lock().await.buffer.mark_gap().is_err() {
                        break;
                    }
                }
                None => break,
            }
        }
    });
    Ok(state)
}
