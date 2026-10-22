//! The `tracing` layer that turns each event into a row of the server log.
//!
//! A row holds the event's spans' fields, outermost first, then its own, the later
//! naming a field winning. The request, user, conversation, turn, Host and page
//! IDs go to columns of their own.
//!
//! An event names what happened with its `event` field, `noun.verb` in the past
//! tense; one that names nothing is logged under its target, its message
//! among the fields.
//!
//! The layer never waits and never fails: an event the writer has no room for is
//! dropped and counted rather than holding up what it describes.

use std::fmt::{self, Write as _};

use demi_shared_types::{LogLevel, LogLine, LogOrigin, is_event_name};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use super::{Row, Sink};

/// The fields a span recorded, kept in its extensions.
struct SpanFields(Map<String, Value>);

pub struct ServerLogLayer {
    sink: Sink,
}

impl ServerLogLayer {
    pub(super) fn new(sink: Sink) -> Self {
        Self { sink }
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
            super::store::now_ms(),
            LogOrigin::Backend,
            level_of(*metadata.level()),
            named.unwrap_or_else(|| metadata.target().to_owned()),
        );
        line.request_id = take(&["request_id"]);
        line.user_id = take(&["user", "user_id"]);
        line.conversation_id = take(&["conversation", "conversation_id"]);
        line.turn_id = take(&["turn", "turn_id"]);
        // The Host a line is about, by whichever name the code used for it.
        line.host_id = take(&["host_id", "device", "device_id"]);
        line.page_id = take(&["page_id"]);
        if !fields.message.is_empty() {
            values.insert("message".into(), fields.message.into());
        }
        line.fields = values;
        self.sink.push(Row { line, seq: None });
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
