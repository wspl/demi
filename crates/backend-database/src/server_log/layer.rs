//! The `tracing` layer that turns each event into a row of the server log:
//! its spans' fields, outermost first, then its own, the later naming a
//! field winning; the request, user, conversation and turn IDs go to columns
//! of their own. It never waits and never fails: an event the writer has no
//! room for is dropped and counted rather than holding up what it describes.

use std::fmt::{self, Write as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;

use demi_web_api_protocol::diagnostics::LogLevel;
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use super::{Message, Row};

/// The fields a span recorded, kept in its extensions.
struct SpanFields(Map<String, Value>);

pub struct ServerLogLayer {
    queue: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
}

impl ServerLogLayer {
    pub(super) fn new(queue: SyncSender<Message>, dropped: Arc<AtomicU64>) -> Self {
        Self { queue, dropped }
    }
}

impl<S> Layer<S> for ServerLogLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        let Some(span) = context.span(id) else {
            return;
        };
        let mut fields = Fields::default();
        attributes.record(&mut fields);
        span.extensions_mut().insert(SpanFields(fields.values));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, context: Context<'_, S>) {
        let Some(span) = context.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if let Some(SpanFields(kept)) = extensions.get_mut::<SpanFields>() {
            let mut fields = Fields::default();
            values.record(&mut fields);
            kept.extend(fields.values);
        }
    }

    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut values = Map::new();
        if let Some(scope) = context.event_scope(event) {
            for span in scope.from_root() {
                if let Some(SpanFields(kept)) = span.extensions().get::<SpanFields>() {
                    values.extend(kept.clone());
                }
            }
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        values.extend(fields.values);
        let metadata = event.metadata();
        let mut take = |names: &[&str]| {
            let mut taken = None;
            for name in names {
                if let Some(value) = values.remove(*name) {
                    taken = Some(match value {
                        Value::String(text) => text,
                        other => other.to_string(),
                    });
                }
            }
            taken
        };
        let row = Row {
            at: super::store::now_ms(),
            level: level_of(*metadata.level()),
            target: metadata.target().to_owned(),
            message: fields.message,
            request_id: take(&["request_id"]),
            user_id: take(&["user", "user_id"]),
            conversation_id: take(&["conversation", "conversation_id"]),
            turn_id: take(&["turn", "turn_id"]),
            fields: values,
        };
        if self.queue.try_send(Message::Row(row)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
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

/// The message and the other fields of an event or a span, each as JSON:
/// numbers and booleans as themselves, an error as its message with its
/// causes, anything else as it prints.
#[derive(Default)]
struct Fields {
    message: String,
    values: Map<String, Value>,
}

impl Visit for Fields {
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
