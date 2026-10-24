//! A search of the text of the files under a directory
//! (`crates/runner-protocol/src/search.rs`), with ripgrep's libraries, as Helix
//! searches: `ignore` walks the directory on several threads and applies the
//! ignore files, `grep-regex` builds the one matcher, and `grep-searcher`
//! finds the matching lines of each file.
//!
//! - Hidden files are searched, as VS Code searches them. `.gitignore`,
//!   `.ignore` and git's exclude file leave files out unless the query
//!   includes ignored files, in a repository or not. `.git` is never searched,
//!   and symbolic links are not followed.
//! - The query's globs are read as `.gitignore` lines and choose among the
//!   files the walk leaves in, as Zed's filters do: a glob to include never
//!   brings back a file an ignore file leaves out. ripgrep's `--glob`, which
//!   overrides the ignore files, would bring back an ignored file but not one
//!   in an ignored folder.
//! - A file is searched when Files shows it as text: up to
//!   [`EDIT_FILE_BYTES`], and not binary by [`is_binary`]. A file that is not
//!   UTF-8 is searched as Files shows it, each byte that is not UTF-8 a
//!   U+FFFD, so a pattern matches what its result shows.
//! - Each match is one result, with the part of its line around it. A file's
//!   search stops at the matches that still fit under [`MAX_MATCHES`], so a
//!   file of millions of matches costs no more than the limit, and the search
//!   stops there or at [`MAX_FILES`].
//! - The search stops when its output is no longer read, or its token is
//!   cancelled, between files and between the lines of one.

use std::io::{self, Read as _};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bytes::Bytes;
use demi_command_protocol::{EDIT_FILE_BYTES, is_binary};
use demi_runner_protocol::search::{LINE_UNITS, MAX_FILES, MAX_MATCHES, SearchLine, SearchQuery, TextMatch};
use futures_util::Stream;
use grep_matcher::{LineTerminator, Matcher as _};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use ::ignore::gitignore::{Gitignore, GitignoreBuilder};
use ::ignore::{WalkBuilder, WalkState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// How many code units of a long line show before a result's match.
const LEAD_UNITS: usize = 40;
/// The lines written ahead of the reader before the walk waits for it.
const QUEUED_LINES: usize = 64;
/// How long the search stays silent before it writes an empty line.
const QUIET: std::time::Duration = std::time::Duration::from_secs(15);
/// The most threads one search walks with: a search shares the Host with the
/// agents' work.
const MAX_THREADS: usize = 8;

/// A search whose query was read and whose directory is there, ready to run.
pub struct Search {
    root: PathBuf,
    matcher: RegexMatcher,
    include: Option<Gitignore>,
    exclude: Option<Gitignore>,
    include_ignored: bool,
}

impl Search {
    /// The search `query` asks for under `root`: an error of kind
    /// `InvalidInput` for a pattern or a glob that is not one, and the
    /// directory's own error when it cannot be read as one.
    pub fn new(root: &Path, query: &SearchQuery) -> io::Result<Self> {
        let metadata = std::fs::metadata(root)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "Not a directory"));
        }
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(!query.case_sensitive)
            .word(query.whole_word)
            .fixed_strings(!query.regex)
            // A match never spans lines, and `$` matches before a CRLF's CR.
            .line_terminator(Some(b'\n'))
            .crlf(true)
            .build(&query.pattern)
            .map_err(invalid)?;
        Ok(Self {
            root: root.to_owned(),
            matcher,
            include: globs(root, &query.include)?,
            exclude: globs(root, &query.exclude)?,
            include_ignored: query.include_ignored,
        })
    }

    /// Runs the search on threads of its own, and streams what it finds, one
    /// JSON line after another, the last one its end.
    pub fn run(self, cancel: CancellationToken) -> impl Stream<Item = io::Result<Bytes>> + Send + 'static {
        let (lines, received) = mpsc::channel(QUEUED_LINES);
        tokio::task::spawn_blocking(move || self.walk(&lines, &cancel));
        futures_util::stream::unfold(received, |mut received| async move {
            let line = match tokio::time::timeout(QUIET, received.recv()).await {
                Ok(line) => line?,
                // Something moves on a connection that would close when idle.
                Err(_) => Bytes::from_static(b"\n"),
            };
            Some((Ok(line), received))
        })
    }

    fn walk(self, lines: &mpsc::Sender<Bytes>, cancel: &CancellationToken) {
        let threads = std::thread::available_parallelism()
            .map_or(1, |cores| cores.get().saturating_sub(1))
            .clamp(1, MAX_THREADS);
        let filtered = !self.include_ignored;
        let exclude = self.exclude.clone();
        let walk = WalkBuilder::new(&self.root)
            .hidden(false)
            .ignore(filtered)
            .git_ignore(filtered)
            .git_global(filtered)
            .git_exclude(filtered)
            .parents(filtered)
            .require_git(false)
            .follow_links(false)
            .filter_entry(move |entry| {
                let excluded = entry.depth() > 0
                    && exclude.as_ref().is_some_and(|globs| {
                        let folder = entry.file_type().is_some_and(|kind| kind.is_dir());
                        globs.matched(entry.path(), folder).is_ignore()
                    });
                entry.file_name() != ".git" && !excluded
            })
            .threads(threads)
            .build_parallel();
        let found = Mutex::new(Found::default());
        let stopped = || cancel.is_cancelled() || lines.is_closed();
        walk.run(|| {
            // The matcher's CRLF, which a searcher of another line end refuses.
            let mut searcher = SearcherBuilder::new()
                .line_terminator(LineTerminator::crlf())
                .line_number(true)
                .build();
            let mut bytes = Vec::new();
            let (root, matcher, include, found, stopped) = (&self.root, &self.matcher, &self.include, &found, &stopped);
            Box::new(move |entry| {
                if stopped() {
                    return WalkState::Quit;
                }
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                    return WalkState::Continue;
                }
                // A file is included by its own glob or one of its folders'.
                if include
                    .as_ref()
                    .is_some_and(|globs| !globs.matched_path_or_any_parents(entry.path(), false).is_ignore())
                {
                    return WalkState::Continue;
                }
                let room = found.lock().expect("the search's count is intact").room();
                if room == 0 {
                    return WalkState::Quit;
                }
                let file = File { matcher, room, stopped };
                let Some(matches) = file.search(entry.path(), &mut searcher, &mut bytes) else {
                    return WalkState::Continue;
                };
                let path = relative(root, entry.path());
                found.lock().expect("the search's count is intact").write(path, matches, lines)
            })
        });
        let found = found.into_inner().expect("the search's count is intact");
        if found.closed || cancel.is_cancelled() {
            return;
        }
        let end = SearchLine::End {
            files: found.files,
            matches: found.matches,
            limit_reached: found.limit_reached,
        };
        drop(lines.blocking_send(json_line(&end)));
    }
}

/// The globs of a query's field as `.gitignore` lines; none for an empty field.
fn globs(root: &Path, globs: &[String]) -> io::Result<Option<Gitignore>> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = GitignoreBuilder::new(root);
    for glob in globs {
        builder.add_line(None, glob).map_err(invalid)?;
    }
    builder.build().map(Some).map_err(invalid)
}

/// What the search has written so far.
#[derive(Default)]
struct Found {
    files: u32,
    matches: u32,
    limit_reached: bool,
    /// The output is no longer read.
    closed: bool,
}

impl Found {
    /// How many more matches the search takes.
    fn room(&self) -> usize {
        if self.closed || self.limit_reached {
            return 0;
        }
        (MAX_MATCHES - self.matches) as usize
    }

    /// Writes a file that matched with the matches that fit under the
    /// limits; whether the walk goes on.
    fn write(&mut self, path: String, mut matches: Vec<TextMatch>, lines: &mpsc::Sender<Bytes>) -> WalkState {
        // Another thread may have written matches since this file's search began.
        matches.truncate(self.room());
        if matches.is_empty() {
            return if self.room() == 0 { WalkState::Quit } else { WalkState::Continue };
        }
        self.files += 1;
        self.matches += to_u32(matches.len());
        if lines.blocking_send(json_line(&SearchLine::File { path, matches })).is_err() {
            self.closed = true;
            return WalkState::Quit;
        }
        if self.matches >= MAX_MATCHES || self.files >= MAX_FILES {
            self.limit_reached = true;
            return WalkState::Quit;
        }
        WalkState::Continue
    }
}

/// The search of one file: its matcher, how many matches it may still take,
/// and whether the search stopped.
struct File<'a> {
    matcher: &'a RegexMatcher,
    room: usize,
    stopped: &'a (dyn Fn() -> bool + Sync),
}

impl File<'_> {
    /// The matches of the file at `path`, none when it matches nothing or is
    /// no text Files shows. `bytes` is the thread's buffer.
    fn search(&self, path: &Path, searcher: &mut Searcher, bytes: &mut Vec<u8>) -> Option<Vec<TextMatch>> {
        bytes.clear();
        let file = std::fs::File::open(path).ok()?;
        // One byte past the limit tells a file that is too large.
        file.take(EDIT_FILE_BYTES as u64 + 1).read_to_end(bytes).ok()?;
        if bytes.len() > EDIT_FILE_BYTES || is_binary(bytes) {
            return None;
        }
        if std::str::from_utf8(bytes).is_err() {
            *bytes = String::from_utf8_lossy(bytes).into_owned().into_bytes();
        }
        let mut sink = Matches { file: self, found: Vec::new() };
        searcher.search_slice(self.matcher, bytes, &mut sink).ok()?;
        (!sink.found.is_empty()).then_some(sink.found)
    }
}

/// Collects a file's matches until it has as many as fit, or the search
/// stopped.
struct Matches<'a> {
    file: &'a File<'a>,
    found: Vec<TextMatch>,
}

impl Sink for &mut Matches<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let left = self.file.room - self.found.len();
        if left == 0 || (self.file.stopped)() {
            return Ok(false);
        }
        // The file is UTF-8, as `File::search` made it.
        let text = String::from_utf8_lossy(found.bytes());
        let text = text.trim_end_matches(['\n', '\r']);
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut place = None;
        self.file
            .matcher
            .find_iter(text.as_bytes(), |found| {
                // A pattern of bytes, as `(?-u:.)`, can match part of a character.
                let range = on_chars(text, found.start()..found.end());
                if range.is_empty() {
                    place.get_or_insert(range);
                } else if ranges.last() != Some(&range) {
                    ranges.push(range);
                }
                ranges.len() < left
            })
            .map_err(io::Error::other)?;
        // A pattern that matches only a place, as `^`, matched the line once.
        if ranges.is_empty() {
            ranges.push(place.unwrap_or(0..0));
        }
        let line = Line::new(u32::try_from(found.line_number().unwrap_or(0)).unwrap_or(u32::MAX), text);
        self.found.extend(ranges.into_iter().map(|range| line.result(range)));
        Ok(self.found.len() < self.file.room)
    }
}

/// `range` widened to whole characters of `text`.
fn on_chars(text: &str, range: Range<usize>) -> Range<usize> {
    let mut start = range.start;
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = range.end.max(start);
    while !text.is_char_boundary(end) {
        end += 1;
    }
    start..end
}

/// A line that matched, as its results show it.
struct Line<'a> {
    number: u32,
    text: &'a str,
    /// Where the line starts after the space it starts with, in bytes.
    indent: usize,
    /// The code units from `indent` to the line's end.
    units: usize,
}

impl<'a> Line<'a> {
    fn new(number: u32, text: &'a str) -> Self {
        let indent = text.len() - text.trim_start().len();
        Self { number, text, indent, units: units(&text[indent..]) }
    }

    /// The result of the match at `range`: the line without the space it
    /// starts with, unless the match is in it, and a long line cut around
    /// the match, with the match in UTF-16 code units of what shows.
    fn result(&self, range: Range<usize>) -> TextMatch {
        let text = self.text;
        let range = if range.is_empty() && range.start < self.indent { self.indent..self.indent } else { range };
        let from = range.start.min(self.indent);
        let whole = self.units + units(&text[from..self.indent]) <= LINE_UNITS;
        // Where the shown part starts, in bytes: `from`, or a little before
        // the match of a line too long to show whole.
        let mut start = from;
        let mut cut_before = false;
        if !whole {
            start = range.start;
            let mut lead = 0;
            for (offset, character) in text[from..range.start].char_indices().rev() {
                if lead + character.len_utf16() > LEAD_UNITS {
                    cut_before = true;
                    break;
                }
                lead += character.len_utf16();
                start = from + offset;
            }
        }
        let mut end = start;
        let mut shown_units = 0;
        for (offset, character) in text[start..].char_indices() {
            if shown_units + character.len_utf16() > LINE_UNITS {
                break;
            }
            shown_units += character.len_utf16();
            end = start + offset + character.len_utf8();
        }
        let cut_after = end < text.len();
        let prefix = if cut_before { "…" } else { "" };
        let offset = units(prefix);
        TextMatch {
            line: self.number,
            text: format!("{prefix}{}{}", &text[start..end], if cut_after { "…" } else { "" }),
            start: to_u32(offset + units(&text[start..range.start])),
            end: to_u32(offset + units(&text[start..range.end.min(end).max(range.start)])),
        }
    }
}

/// `text`'s length in UTF-16 code units, as the page measures a string.
fn units(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

fn to_u32(units: usize) -> u32 {
    u32::try_from(units).unwrap_or(u32::MAX)
}

/// `path` relative to `root`, with `/` between its parts on every platform.
fn relative(root: &Path, path: &Path) -> String {
    let inside = path.strip_prefix(root).unwrap_or(path);
    inside
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn json_line(line: &SearchLine) -> Bytes {
    let mut bytes = serde_json::to_vec(line).expect("a search line serializes");
    bytes.push(b'\n');
    Bytes::from(bytes)
}

/// What is wrong with a pattern or a glob, as a sentence: the regex library draws the
/// pattern with a caret under the fault, which only a fixed-width font lines
/// up, and says the fault on its last line.
fn pattern_error(message: &str) -> String {
    let fault = message.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or(message);
    let fault = fault.trim().trim_start_matches("error:").trim();
    let mut sentence: String = fault.chars().take(1).flat_map(char::to_uppercase).collect();
    sentence.push_str(&fault[fault.chars().next().map_or(0, char::len_utf8)..]);
    if !sentence.ends_with('.') {
        sentence.push('.');
    }
    sentence
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, pattern_error(&error.to_string()))
}
