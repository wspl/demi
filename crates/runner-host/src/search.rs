//! A search of the names and the text of the files under a directory
//! (`crates/runner-protocol/src/search.rs`), with ripgrep's libraries, as Helix
//! searches: `ignore` walks the directory on several threads and applies the
//! ignore files, `grep-regex` builds the one matcher, and `grep-searcher`
//! finds a file's first line that holds the text.
//!
//! - Hidden files are searched, as VS Code searches them. `.gitignore`,
//!   `.ignore` and git's exclude file leave files out, in a repository or
//!   not. `.git` is never searched, and symbolic links are not followed.
//! - A file whose name holds the text is found, whatever it holds. Its text is
//!   searched when Files shows it as text: up to [`EDIT_FILE_BYTES`], and not
//!   binary by [`is_binary`]. A file that is not UTF-8 is searched as Files
//!   shows it, each byte that is not UTF-8 a U+FFFD, so what a result shows is
//!   what was found.
//! - A file's search stops at its first line that holds the text, which the
//!   result shows.
//! - The search stops at [`MAX_FILES`], or when its output is no longer read,
//!   or its token is cancelled.

use std::io::{self, Read as _};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bytes::Bytes;
use demi_command_protocol::{EDIT_FILE_BYTES, is_binary};
use demi_runner_protocol::search::{FoundLine, LINE_UNITS, MAX_FILES, SearchLine, SearchQuery, TextRange};
use futures_util::Stream;
use grep_matcher::{LineTerminator, Matcher as _};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use ::ignore::{WalkBuilder, WalkState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// How many code units of a long line show before its first match.
const LEAD_UNITS: usize = 40;
/// The lines written ahead of the reader before the walk waits for it.
const QUEUED_LINES: usize = 64;
/// How long the search stays silent before it writes an empty line.
const QUIET: std::time::Duration = std::time::Duration::from_secs(15);
/// The most threads one search walks with: a search shares the Host with the
/// agents' work.
const MAX_THREADS: usize = 8;

/// A search whose directory is there, ready to run.
pub struct Search {
    root: PathBuf,
    matcher: RegexMatcher,
}

impl Search {
    /// The search `query` asks for under `root`, or the directory's own error
    /// when it cannot be read as one.
    pub fn new(root: &Path, query: &SearchQuery) -> io::Result<Self> {
        let metadata = std::fs::metadata(root)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "Not a directory"));
        }
        // A directory it cannot list fails here: the walk skips what it cannot
        // read, and would end finding nothing.
        std::fs::read_dir(root)?;
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(true)
            .fixed_strings(true)
            .line_terminator(Some(b'\n'))
            .crlf(true)
            .build(&query.text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
        Ok(Self { root: root.to_owned(), matcher })
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
        let walk = WalkBuilder::new(&self.root)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .threads(threads)
            .build_parallel();
        let found = Mutex::new(Found::default());
        walk.run(|| {
            // The matcher's CRLF, which a searcher of another line end refuses.
            let mut searcher = SearcherBuilder::new()
                .line_terminator(LineTerminator::crlf())
                .line_number(true)
                .build();
            let mut bytes = Vec::new();
            let (root, matcher, found) = (&self.root, &self.matcher, &found);
            Box::new(move |entry| {
                if cancel.is_cancelled() || lines.is_closed() {
                    return WalkState::Quit;
                }
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                    return WalkState::Continue;
                }
                let name = entry.file_name().to_string_lossy();
                let name = matches(matcher, &name);
                let line = first_line(entry.path(), matcher, &mut searcher, &mut bytes);
                if name.is_empty() && line.is_none() {
                    return WalkState::Continue;
                }
                let file = SearchLine::File { path: relative(root, entry.path()), name, line };
                found.lock().expect("the search's count is intact").write(&file, lines)
            })
        });
        let found = found.into_inner().expect("the search's count is intact");
        if found.closed || cancel.is_cancelled() {
            return;
        }
        let end = SearchLine::End { files: found.files, limit_reached: found.limit_reached };
        drop(lines.blocking_send(json_line(&end)));
    }
}

/// What the search has written so far.
#[derive(Default)]
struct Found {
    files: u32,
    limit_reached: bool,
    /// The output is no longer read.
    closed: bool,
}

impl Found {
    /// Writes a file that matched; whether the walk goes on.
    fn write(&mut self, file: &SearchLine, lines: &mpsc::Sender<Bytes>) -> WalkState {
        if self.closed || self.limit_reached {
            return WalkState::Quit;
        }
        if lines.blocking_send(json_line(file)).is_err() {
            self.closed = true;
            return WalkState::Quit;
        }
        self.files += 1;
        if self.files >= MAX_FILES {
            self.limit_reached = true;
            return WalkState::Quit;
        }
        WalkState::Continue
    }
}

/// Where `text` holds what `matcher` finds, in bytes, each range on character
/// boundaries.
fn found_in(matcher: &RegexMatcher, text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    // A text that cannot be searched is one that holds nothing.
    drop(matcher.find_iter(text.as_bytes(), |range| {
        found.push(range.start()..range.end());
        true
    }));
    found.retain(|range| !range.is_empty() && text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
    found
}

/// Where `text` holds what `matcher` finds, in UTF-16 code units.
fn matches(matcher: &RegexMatcher, text: &str) -> Vec<TextRange> {
    found_in(matcher, text)
        .into_iter()
        .map(|range| TextRange { start: units(&text[..range.start]), end: units(&text[..range.end]) })
        .collect()
}

/// The first line of the file at `path` that holds the text, none when it
/// holds none or is no text Files shows. `bytes` is the thread's buffer.
fn first_line(path: &Path, matcher: &RegexMatcher, searcher: &mut Searcher, bytes: &mut Vec<u8>) -> Option<FoundLine> {
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
    // A lone CR ends a line too, as the editor that opens the file at the
    // line counts lines; the searcher's lines end at LF.
    for at in 0..bytes.len() {
        if bytes[at] == b'\r' && bytes.get(at + 1) != Some(&b'\n') {
            bytes[at] = b'\n';
        }
    }
    let mut sink = First { matcher, line: None };
    searcher.search_slice(matcher, bytes, &mut sink).ok()?;
    sink.line
}

/// Keeps the first line a file's search matched, and stops it there.
struct First<'a> {
    matcher: &'a RegexMatcher,
    line: Option<FoundLine>,
}

impl Sink for &mut First<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        // The file is UTF-8, as `first_line` made it.
        let text = String::from_utf8_lossy(found.bytes());
        let text = text.trim_end_matches(['\n', '\r']);
        let number = u32::try_from(found.line_number().unwrap_or(0)).unwrap_or(u32::MAX);
        self.line = Some(shown_line(self.matcher, number, text));
        Ok(false)
    }
}

/// A line as its result shows it: without the space it starts with, and a
/// long one cut around its first match, with its matches in what shows.
fn shown_line(matcher: &RegexMatcher, number: u32, text: &str) -> FoundLine {
    let first = matcher.find(text.as_bytes()).ok().flatten().map_or(0, |found| found.start());
    // The space the line starts with shows only when the text is in it.
    let indent = (text.len() - text.trim_start().len()).min(first);
    // Where the shown part starts, in bytes: the line's text, or a little
    // before the first match of a line too long to show whole.
    let mut start = indent;
    let mut cut_before = false;
    if units_over(&text[indent..], LINE_UNITS) {
        start = first;
        let mut lead = 0;
        for (offset, character) in text[indent..first].char_indices().rev() {
            if lead + character.len_utf16() > LEAD_UNITS {
                cut_before = true;
                break;
            }
            lead += character.len_utf16();
            start = indent + offset;
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
    let prefix = if cut_before { "…" } else { "" };
    let offset = units(prefix);
    // The matches of the whole line, cut to what shows, so a match longer
    // than that still shows marked.
    let matches = found_in(matcher, text)
        .into_iter()
        .filter(|range| range.end > start && range.start < end)
        .map(|range| TextRange {
            start: offset + units(&text[start..range.start.max(start)]),
            end: offset + units(&text[start..range.end.min(end)]),
        })
        .collect();
    let suffix = if end < text.len() { "…" } else { "" };
    FoundLine { number, text: format!("{prefix}{}{suffix}", &text[start..end]), matches }
}

/// Whether `text` is longer than `limit` UTF-16 code units, read no further
/// than that.
fn units_over(text: &str, limit: usize) -> bool {
    let mut count = 0;
    text.chars().any(|character| {
        count += character.len_utf16();
        count > limit
    })
}

/// `text`'s length in UTF-16 code units, as the page measures a string.
fn units(text: &str) -> u32 {
    u32::try_from(text.chars().map(char::len_utf16).sum::<usize>()).unwrap_or(u32::MAX)
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
