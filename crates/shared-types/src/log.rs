//! The one line of the log, whatever wrote it: the backend, a Host's runner,
//! the Cloud's machine manager or a page.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How important a line is, lowest first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// What wrote a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOrigin {
    Backend,
    Runner,
    Page,
    /// The Cloud machine manager, which sends its lines on the backend's
    /// connection to it.
    MachineManager,
}

/// One line of the log, as the backend stores it, whatever wrote it. A line
/// never holds text a person wrote or content: it names things by ID and
/// counts their sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogLine {
    /// Milliseconds since the Unix epoch, by the writer's clock.
    pub at: i64,
    pub origin: LogOrigin,
    pub level: LogLevel,
    /// What happened, as `noun.verb` in the past tense, such as
    /// `command.ended` or `follow.answered`; the `tracing` target for a
    /// backend event that names none.
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// The Host whose runner wrote it, or that a backend line is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    /// The page load that wrote it, or whose request a backend line serves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    /// The span's and the event's other fields: IDs, counts, durations,
    /// states.
    #[serde(default)]
    pub fields: Map<String, Value>,
}

/// Whether `name` is an event's name: `noun.verb`, lowercase words of letters,
/// digits and underscores joined by dots.
/// A value that is not one never becomes a name, so a payload logged under the
/// field `event` cannot stand in for it.
pub fn is_event_name(name: &str) -> bool {
    let words: Vec<&str> = name.split('.').collect();
    words.len() >= 2
        && words.iter().all(|word| {
            word.chars().next().is_some_and(|first| first.is_ascii_lowercase())
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

impl LogLine {
    /// A line of `origin` at `at` with no IDs and no fields.
    pub fn new(at: i64, origin: LogOrigin, level: LogLevel, event: impl Into<String>) -> Self {
        Self {
            at,
            origin,
            level,
            event: event.into(),
            request_id: None,
            user_id: None,
            conversation_id: None,
            turn_id: None,
            host_id: None,
            page_id: None,
            fields: Map::new(),
        }
    }
}

/// A line of a Host's log with the number it has in the Host's files, which
/// grows by one per line and across restarts: the cursor the backend asks the
/// runner to continue after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostLogLine {
    pub seq: u64,
    pub line: LogLine,
}
