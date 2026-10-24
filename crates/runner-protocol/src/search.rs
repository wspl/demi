//! A search of the names and the text of the files under a directory
//! (`crates/runner-host/src/search.rs`): the text the page sends, and what the
//! search writes, one JSON object per line, as the relay's route and a direct
//! `search` channel both carry it.
//!
//! - The text is found as it is typed, letter case aside, in a file's name
//!   and in the lines of its text.
//! - Each file that matches is one [`SearchLine::File`], as the walk finds it,
//!   in no order: the page sorts them.
//! - The last line is [`SearchLine::End`]. A search whose output ends without it
//!   was cut short.
//! - An empty line, written after 15 seconds without another, says the search
//!   still runs.

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_command_protocol::without_nul;

/// The search stops once this many files matched, as Zed's does: a list
/// longer than that is no longer read, and more text narrows it.
pub const MAX_FILES: u32 = 5_000;
/// The most characters of the text searched for.
pub const TEXT_CHARS: usize = 1_000;
/// The most UTF-16 code units of a line a result shows: a longer line shows
/// the part around its first match.
pub const LINE_UNITS: usize = 240;

/// What to search for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchQuery {
    /// Found as it is, letter case aside, never as a pattern; one line.
    #[garde(length(chars, min = 1, max = TEXT_CHARS), custom(one_line))]
    pub text: String,
}

/// A text without NUL, CR or LF: a match never spans lines.
fn one_line(text: &str, context: &()) -> garde::Result {
    without_nul(text, context)?;
    if text.contains(['\n', '\r']) {
        return Err(garde::Error::new("holds a line break"));
    }
    Ok(())
}

/// One line of a search's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum SearchLine {
    /// A file whose name or text holds the text.
    File {
        /// Relative to the directory searched, with `/` between its parts.
        path: String,
        /// Where the text is in the file's name, in UTF-16 code units; empty
        /// when only its text holds it.
        name: Vec<TextRange>,
        /// The first line of the file's text that holds it; none when only
        /// its name does.
        line: Option<FoundLine>,
    },
    /// How the search ended: the files it found, and whether
    /// [`MAX_FILES`] stopped it.
    End { files: u32, limit_reached: bool },
}

/// A line that holds the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FoundLine {
    /// 1-based; a file Files shows as text has fewer lines than this holds.
    pub number: u32,
    /// The line without its end and the space it starts with, or of a line
    /// longer than [`LINE_UNITS`] the part around its first match, with `…`
    /// where it was cut.
    pub text: String,
    /// Where `text` holds the text, in order, in UTF-16 code units of `text`,
    /// as the page slices a string.
    pub matches: Vec<TextRange>,
}

/// A part of a text, in UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}
