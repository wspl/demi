//! The `tracing` layer that writes each event as a [`LogLine`], for a process
//! that is neither the backend nor a runner: a command package's service and
//! the machine manager.
//!
//! A line holds the event's spans' fields, outermost first, then its own, the
//! later naming a field winning. The request, user, conversation, turn, Host
//! and page IDs go to the line's own fields. An event names what happened with
//! its `event` field, `noun.verb` in the past tense; one that names nothing is
//! written under its target, its message among the fields.
//!
//! The layer hands each line to a function of the process, which decides where
//! it goes (a service's standard error, the manager's connection). It never
//! waits for more than that function does.

use std::fmt::{self, Write as _};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use crate::{LogLevel, LogLine, LogOrigin, is_event_name};

/// The fields a span recorded, kept in its extensions.
struct SpanFields(Map<String, Value>);

/// The layer: each event becomes a line of `origin`, which `write` receives.
pub struct LineLayer<W> {
    origin: LogOrigin,
    write: W,
}

impl<W: Fn(LogLine) + Send + Sync + 'static> LineLayer<W> {
    pub fn new(origin: LogOrigin, write: W) -> Self {
        Self { origin, write }
    }
}

impl<S, W> Layer<S> for LineLayer<W>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    W: Fn(LogLine) + Send + Sync + 'static,
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
        // Only a string that is a name is taken for the event's: whatever
        // else a call logged under `event`, a payload as much as a typo, is
        // left out of the line, since it is not a name and may hold a message.
        let named = match values.remove("event") {
            Some(Value::String(name)) if is_event_name(&name) => Some(name),
            Some(_) => {
                values.insert("event_refused".into(), true.into());
                None
            }
            None => None,
        };
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
        let mut line = LogLine::new(
            now_ms(),
            self.origin,
            level_of(*metadata.level()),
            named.unwrap_or_else(|| metadata.target().to_owned()),
        );
        line.request_id = take(&["request_id"]);
        line.user_id = take(&["user", "user_id"]);
        line.conversation_id = take(&["conversation", "conversation_id"]);
        line.turn_id = take(&["turn", "turn_id"]);
        line.host_id = take(&["host_id"]);
        line.page_id = take(&["page_id"]);
        if !fields.message.is_empty() {
            values.insert("message".into(), fields.message.into());
        }
        line.fields = values;
        (self.write)(line);
    }
}

fn now_ms() -> i64 {
    // A clock before 1970 has no meaningful time to record.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
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
