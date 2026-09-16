//! A tab's console history is bounded independently of any reader's cursor.
use super::{
    Result,
    history::Buffer,
    protocol::{CONSOLE_BYTES, CONSOLE_ENTRIES, DEFAULT_NODES, LogsInput},
};
use chromiumoxide::{Page, cdp::js_protocol::runtime::EventConsoleApiCalled};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(super) struct Console {
    buffer: Buffer,
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
            "debug" => "debug",
            "info" => "info",
            "warning" => "warning",
            "error" | "assert" => "error",
            _ => "log",
        };
        let mut entry = json!({"level": level, "text": text, "timestamp": event.timestamp});
        if let Some(frame) = event
            .stack_trace
            .as_ref()
            .and_then(|stack| stack.call_frames.first())
        {
            entry["url"] = json!(frame.url);
        }
        self.buffer.push(entry)
    }

    pub fn read(&self, input: &LogsInput) -> Result<Value> {
        let after = input
            .after
            .as_deref()
            .map(|cursor| self.buffer.position(cursor))
            .transpose()?;
        let entries: Vec<_> = self
            .buffer
            .entries()
            .filter(|entry| {
                after.is_none_or(|after| {
                    entry["sequence"]
                        .as_u64()
                        .is_some_and(|sequence| sequence >= after)
                })
            })
            .filter(|entry| {
                input
                    .level
                    .as_ref()
                    .is_none_or(|levels| levels.iter().any(|level| entry["level"] == *level))
            })
            .filter(|entry| {
                input.filter.as_ref().is_none_or(|filter| {
                    entry["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(filter))
                })
            })
            .collect();
        let limit = input.limit.map_or(DEFAULT_NODES, |limit| limit as usize);
        let start = if after.is_none() {
            entries.len().saturating_sub(limit)
        } else {
            0
        };
        let page: Vec<_> = entries.iter().skip(start).take(limit).copied().collect();
        let has_more = start + page.len() < entries.len();
        let next = if has_more {
            page.last()
                .and_then(|entry| entry["sequence"].as_u64())
                .map_or(self.buffer.next(), |sequence| sequence + 1)
        } else {
            self.buffer.next()
        };
        Ok(
            json!({"entries": page, "cursor": self.buffer.cursor(next), "hasMore": has_more, "truncated": self.buffer.has_evicted()}),
        )
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
