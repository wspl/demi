//! How every `demi` command prints its data: a list as a table whose columns
//! are aligned by the terminal width of their text, one object as labelled
//! lines, each value exact, an instant in RFC 3339 UTC.
//!
//! Commands build their text here rather than each formatting its own. They
//! print as the command-line tools agents and people already read (`kubectl
//! get`, `docker ps`, `gh`) do, so a model needs no new habit.
//!
//! # Streams
//!
//! Stdout holds the data and stderr the rest, so a pipe receives only data.
//! Notes, warnings and progress go to stderr, and an error is one line with
//! nothing on stdout.
//!
//! # Lists
//!
//! - A list's first column is the ID the group's `show` takes, and every ID a
//!   list prints, `show` finds (but the root agent, which `demi agent list`
//!   lists for its place in the tree only).
//! - A cell holds one value, and nested data is `show`'s, never a column.
//! - A list shows what is current, with objects kept as history behind `--all`.
//! - An empty list prints its header row alone.
//!
//! # Values
//!
//! - A number is exact, its unit in the column name.
//! - A relative time is for the web app, not the command line.
//! - `--json` prints the same objects with every field the text shows and more,
//!   in camelCase.
//! - Output is the same in a terminal and a pipe and has no colour, so what the
//!   agent reads is what the user sees.
//!
//! Tables and labelled lines are for Demi's own objects (agents, providers,
//! credentials, Hosts). A command that prints content, such as a page or a
//! file's text, keeps the content's own form.

use std::fmt::Write as _;

use unicode_width::UnicodeWidthStr;

use crate::Timestamp;

/// The text of a missing value.
pub const MISSING: &str = "-";

/// One value of a table's row or of a labelled line: a missing or empty
/// value prints as [`MISSING`], a number as its digits, a yes-or-no fact as
/// `yes` or `no`, an instant as RFC 3339 in UTC to the second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell(String);

impl Cell {
    /// The cell's text, [`MISSING`] for an empty one.
    pub fn text(&self) -> &str {
        if self.0.is_empty() { MISSING } else { &self.0 }
    }
}

impl From<String> for Cell {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<&String> for Cell {
    fn from(text: &String) -> Self {
        Self(text.clone())
    }
}

impl From<&str> for Cell {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<bool> for Cell {
    fn from(fact: bool) -> Self {
        Self(if fact { "yes" } else { "no" }.to_owned())
    }
}

impl From<Timestamp> for Cell {
    fn from(time: Timestamp) -> Self {
        Self(instant(time))
    }
}

impl From<jiff::Timestamp> for Cell {
    fn from(time: jiff::Timestamp) -> Self {
        Self(instant(time))
    }
}

macro_rules! number_cells {
    ($($number:ty),*) => {$(
        impl From<$number> for Cell {
            fn from(number: $number) -> Self {
                Self(number.to_string())
            }
        }
    )*};
}

number_cells!(u8, u16, u32, u64, usize, i32, i64, f64);

impl<T: Into<Cell>> From<Option<T>> for Cell {
    fn from(value: Option<T>) -> Self {
        value.map_or_else(|| Self(String::new()), Into::into)
    }
}

/// `time` as an instant prints: RFC 3339 in UTC to the second, as
/// `2026-10-17T14:02:11Z`.
pub fn instant(time: impl Into<jiff::Timestamp>) -> String {
    let time: jiff::Timestamp = time.into();
    let second = jiff::Timestamp::from_second(time.as_second()).unwrap_or(time);
    second.to_string()
}

/// A note, a warning or progress as `command`, such as `demi agent list`,
/// writes it to stderr: `demi agent list: warning: <what>`, on a line of its
/// own.
pub fn warning(command: &str, what: &str) -> String {
    format!("{command}: warning: {what}\n")
}

/// A note as `command` writes it to stderr, such as the page a list shows
/// and the call that gives the next: `demi <command>: note: <what>`.
pub fn note(command: &str, what: &str) -> String {
    format!("{command}: note: {what}\n")
}

/// A span of milliseconds in whole seconds, rounded to the nearest, as a
/// column or key named with `(S)` holds it.
pub fn seconds(milliseconds: u64) -> u64 {
    milliseconds.saturating_add(500) / 1000
}

/// A list as a table: a header row naming the columns in capitals, then one
/// row per object; columns aligned by the terminal width of their text and
/// separated by two spaces. The last column, free text such as a
/// description, is never padded or cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// A table with these column names, which print in capitals.
    pub fn new<S: AsRef<str>>(columns: impl IntoIterator<Item = S>) -> Self {
        Self {
            columns: columns
                .into_iter()
                .map(|column| column.as_ref().to_uppercase())
                .collect(),
            rows: Vec::new(),
        }
    }

    /// Adds one object's row. A cell holds one line: a line break in its
    /// text prints as a space. Missing cells at its end print as missing
    /// values; a cell past the columns is a bug in the caller.
    pub fn row<C: Into<Cell>>(&mut self, cells: impl IntoIterator<Item = C>) -> &mut Self {
        let mut row: Vec<String> = cells
            .into_iter()
            .map(|cell| one_line(cell.into().text()))
            .collect();
        debug_assert!(row.len() <= self.columns.len(), "a row has more cells than the table has columns");
        row.resize(self.columns.len(), MISSING.to_owned());
        self.rows.push(row);
        self
    }

    /// Whether it has no row.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The table's lines, each ending in a line break; an empty table is its
    /// header row alone.
    pub fn render(&self) -> String {
        self.render_indented("")
    }

    /// The table's lines, each after `indent`, as a nested table under a
    /// labelled line prints.
    fn render_indented(&self, indent: &str) -> String {
        let mut widths: Vec<usize> = self.columns.iter().map(|column| column.width()).collect();
        for row in &self.rows {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.width());
            }
        }
        let mut text = String::new();
        for row in std::iter::once(&self.columns).chain(&self.rows) {
            let mut line = String::from(indent);
            for (index, cell) in row.iter().enumerate() {
                line.push_str(cell);
                if index + 1 < row.len() {
                    let pad = widths[index] - cell.width() + 2;
                    line.extend(std::iter::repeat_n(' ', pad));
                }
            }
            text.push_str(line.trim_end());
            text.push('\n');
        }
        text
    }
}

/// One object as labelled lines: `Key: value`, one per line, in the order
/// added; a nested table under its key, indented by two spaces; a block of
/// free text, which comes last, under its key, indented the same.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    text: String,
}

impl Fields {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Key: value`; a line break in the value prints as a space.
    pub fn field(&mut self, key: &str, value: impl Into<Cell>) -> &mut Self {
        let _ = writeln!(self.text, "{key}: {}", one_line(value.into().text()));
        self
    }

    /// `Key:`, then `table` under it.
    pub fn table(&mut self, key: &str, table: &Table) -> &mut Self {
        let _ = writeln!(self.text, "{key}:");
        self.text.push_str(&table.render_indented("  "));
        self
    }

    /// `Key:`, then `text` under it, each line indented; an empty text is a
    /// missing value on the key's line.
    pub fn text(&mut self, key: &str, text: &str) -> &mut Self {
        let text = text.trim_end();
        if text.is_empty() {
            return self.field(key, MISSING);
        }
        let _ = writeln!(self.text, "{key}:");
        for line in text.lines().map(escaped) {
            if line.is_empty() {
                self.text.push('\n');
            } else {
                let _ = writeln!(self.text, "  {line}");
            }
        }
        self
    }

    /// The lines, each ending in a line break.
    pub fn render(&self) -> String {
        self.text.clone()
    }
}

/// `text` on one line: each line break and the space around it a single
/// space, and every other control character a visible escape, so a value
/// can neither break the layout nor command a terminal.
fn one_line(text: &str) -> String {
    let joined = if text.contains('\n') {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        text.to_owned()
    };
    escaped(&joined)
}

/// `text` with each control character but the line break written as an
/// escape a reader sees: `\r`, `\t`, `\x1b`; a C1 control as `\u{9b}`.
fn escaped(text: &str) -> String {
    if !text.chars().any(|char| char != '\n' && char.is_control()) {
        return text.to_owned();
    }
    let mut escaped = String::with_capacity(text.len());
    for char in text.chars() {
        match char {
            '\n' => escaped.push(char),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            char if (char as u32) < 0x80 && char.is_control() => {
                let _ = write!(escaped, "\\x{:02x}", char as u32);
            }
            char if char.is_control() => {
                let _ = write!(escaped, "\\u{{{:x}}}", char as u32);
            }
            char => escaped.push(char),
        }
    }
    escaped
}
