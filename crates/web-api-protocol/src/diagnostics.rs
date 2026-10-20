//! The page's own error reports and the server log
//! (`crates/backend-http/src/request_id.rs`).

use demi_shared_types::{MAX_SAFE_INTEGER, RequestId, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `POST /page-errors`: a failure the page found itself, a value it received
/// that does not parse or a handler that threw. It says what failed and
/// where, never a value the page held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PageErrorReport {
    /// The error's title as the page shows it.
    #[garde(length(chars, min = 1, max = 200))]
    pub title: String,
    #[garde(length(chars, max = 2000))]
    pub message: String,
    /// Where it failed: a schema's paths with their expected and received
    /// types, or a stack.
    #[garde(length(chars, max = 16000))]
    pub detail: String,
}

/// The level of a server log line, lowest first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// One line of the server log: one event with the fields of its spans and
/// its own, the request, user, conversation and turn IDs among them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerLogLine {
    pub at: Timestamp,
    pub level: LogLevel,
    /// The module or name the event was logged under.
    pub target: String,
    pub message: String,
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// `GET /server/log`: lines oldest first, and the cursor the next read
/// continues from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerLog {
    pub lines: Vec<ServerLogLine>,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub next: u64,
}

/// `?request=&since=&limit=&level=` of the server log. Queries are the
/// backend's alone and are not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ServerLogQuery {
    /// Keeps the lines of one request.
    #[serde(default)]
    pub request: Option<RequestId>,
    /// The `next` of an earlier answer; without it the answer ends at the
    /// newest line.
    #[serde(default)]
    pub since: Option<u64>,
    #[serde(default)]
    pub limit: ServerLogLimit,
    /// Keeps the lines at this level and above.
    #[serde(default)]
    pub level: Option<LogLevel>,
}

/// How many lines a server log read returns: 1 to 1000, 200 when omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "u64")]
pub struct ServerLogLimit(u64);

impl ServerLogLimit {
    pub const DEFAULT: ServerLogLimit = ServerLogLimit(200);
    pub const MOST: u64 = 1000;

    pub fn get(self) -> u64 {
        self.0
    }
}

impl Default for ServerLogLimit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u64> for ServerLogLimit {
    type Error = String;

    fn try_from(limit: u64) -> Result<Self, String> {
        if (1..=Self::MOST).contains(&limit) {
            Ok(Self(limit))
        } else {
            Err(format!("limit must be 1 to {}", Self::MOST))
        }
    }
}
