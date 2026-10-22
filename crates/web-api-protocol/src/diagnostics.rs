//! The page's lines of the log (`POST /log`).

use std::collections::BTreeMap;

use demi_shared_types::{LogLevel, MAX_SAFE_INTEGER};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The header the page names its load in on every request, and the query
/// parameter on a socket's URL.
pub const PAGE_HEADER: &str = "x-demi-page";
pub const PAGE_PARAMETER: &str = "page";

/// The level of a line the page wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl From<PageLogLevel> for LogLevel {
    fn from(level: PageLogLevel) -> Self {
        match level {
            PageLogLevel::Debug => Self::Debug,
            PageLogLevel::Info => Self::Info,
            PageLogLevel::Warn => Self::Warn,
            PageLogLevel::Error => Self::Error,
        }
    }
}

/// `POST /log`: the lines a page wrote, oldest first. A line names things by
/// ID and counts their sizes; it never holds text the user typed or page
/// content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PageLog {
    /// The page load's ID: `pg_` and 16 characters.
    #[garde(pattern(r"^pg_[0-9a-hjkmnp-tv-z]{16}$"))]
    pub page: String,
    #[garde(dive)]
    pub lines: Vec<PageLogLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PageLogLine {
    /// Milliseconds since the Unix epoch, by the page's clock.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub at: u64,
    #[garde(skip)]
    pub level: PageLogLevel,
    /// What happened, as `noun.verb` in the past tense.
    #[garde(pattern(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$"))]
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(inner(length(min = 1)))]
    pub conversation_id: Option<String>,
    /// The line's other fields, which the page writes as scalars.
    #[garde(skip)]
    pub fields: BTreeMap<String, serde_json::Value>,
}
