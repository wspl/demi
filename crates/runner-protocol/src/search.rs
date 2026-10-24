//! A search of the text of the files under a directory
//! (`crates/runner-host/src/search.rs`): the query the page sends, and what the
//! search writes, one JSON object per line, as the relay's route and a direct
//! `search` channel both carry it.
//!
//! - Each file that matches is one [`SearchLine::File`], as the walk finds it,
//!   in no order: the page sorts them.
//! - Each match is one result, as VS Code lists them: a line that holds three
//!   matches is three results, each with the part of the line around it.
//! - The last line is [`SearchLine::End`]. A search whose output ends without it
//!   was cut short.
//! - An empty line, written after 15 seconds without another, says the search
//!   still runs.

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_command_protocol::without_nul;

/// The search stops once it has found this many matches, as Zed's does.
pub const MAX_MATCHES: u32 = 10_000;
/// The search stops once this many files matched, as Zed's does.
pub const MAX_FILES: u32 = 5_000;
/// The most characters of a query's pattern or one of its globs.
pub const PATTERN_CHARS: usize = 1_000;
/// The most globs a query includes or excludes, each.
pub const MAX_GLOBS: usize = 64;
/// The most UTF-16 code units of a line a result shows: a longer line shows
/// the part around the result's match.
pub const LINE_UNITS: usize = 240;

/// What to search for, and in which files under the directory searched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchQuery {
    /// The text to find, or a regular expression when `regex`. A match never
    /// spans lines.
    #[garde(length(chars, min = 1, max = PATTERN_CHARS), custom(without_nul))]
    pub pattern: String,
    #[serde(default)]
    #[garde(skip)]
    pub case_sensitive: bool,
    #[serde(default)]
    #[garde(skip)]
    pub whole_word: bool,
    #[serde(default)]
    #[garde(skip)]
    pub regex: bool,
    /// Globs of the paths to search, relative to the directory searched, as
    /// `.gitignore` reads them; none searches every file. A folder's glob
    /// takes what it holds. They choose among the files the ignore files
    /// leave in, unless `include_ignored`.
    #[serde(default)]
    #[garde(length(max = MAX_GLOBS), inner(length(chars, min = 1, max = PATTERN_CHARS), custom(without_nul)))]
    pub include: Vec<String>,
    /// Globs of the paths to leave out, as `include`'s.
    #[serde(default)]
    #[garde(length(max = MAX_GLOBS), inner(length(chars, min = 1, max = PATTERN_CHARS), custom(without_nul)))]
    pub exclude: Vec<String>,
    /// Also searches what `.gitignore`, `.ignore` and git's exclude file
    /// leave out.
    #[serde(default)]
    #[garde(skip)]
    pub include_ignored: bool,
}

/// One line of a search's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum SearchLine {
    /// A file that matched, with each of its matches, in order.
    File {
        /// Relative to the directory searched, with `/` between its parts.
        path: String,
        matches: Vec<TextMatch>,
    },
    /// How the search ended: the files and the matches it found, and whether
    /// a limit stopped it.
    End {
        files: u32,
        matches: u32,
        limit_reached: bool,
    },
}

/// A match, with the part of its line that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextMatch {
    /// The line's number, 1-based; a file Files shows as text has fewer lines
    /// than this holds.
    pub line: u32,
    /// The line without its end and the space it starts with, or of a line
    /// longer than [`LINE_UNITS`] the part around the match, with `…` where
    /// it was cut.
    pub text: String,
    /// Where the match starts in `text`, in UTF-16 code units, as the page
    /// slices a string.
    pub start: u32,
    /// Where it ends; `start` for a pattern that matches only a place, as `^`.
    pub end: u32,
}
