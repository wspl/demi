//! A command's whole output (`runtime.md` § The whole output): each read of
//! its stdout and stderr with its stream, in the order the runner read them,
//! and where the Host's kept output left bytes out. The backend stores it
//! when the command ends; `demi shell output` and the result that reports
//! the end read it as lines of text ([`OutputText`]).

use bytes::{Bytes, BytesMut};
use demi_shared_types::{BinaryStdout, OutputChunk, StreamKind};

use crate::BinaryOutput;

/// A command's output as kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WholeOutput {
    records: Vec<OutputRecord>,
    missing: Option<Missing>,
}

/// One record of a whole output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputRecord {
    /// One read of stdout or stderr.
    Output(StreamKind, Bytes),
    /// How many bytes the kept output left out here, between its first
    /// part and its last.
    LeftOut(u64),
}

/// Output at the end that the Host held and the backend does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub bytes: u64,
    /// Why, as the line that stands for the bytes says it: `lost with the
    /// Host's connection`, or why the Host's kept output was not read.
    pub reason: String,
}

impl Missing {
    /// The line that stands for the missing bytes.
    pub fn line(&self) -> String {
        format!("[... {} bytes {} ...]", self.bytes, self.reason)
    }
}

impl WholeOutput {
    pub fn new(records: Vec<OutputRecord>, missing: Option<Missing>) -> Self {
        Self { records, missing }
    }

    pub fn records(&self) -> &[OutputRecord] {
        &self.records
    }

    pub fn missing(&self) -> Option<&Missing> {
        self.missing.as_ref()
    }

    /// The kept bytes of `stream`, in order, on each side of where the kept
    /// output left bytes out: the second part is empty when it left none out.
    fn stream_parts(&self, stream: StreamKind) -> [Bytes; 2] {
        let mut parts = [BytesMut::new(), BytesMut::new()];
        let mut part = 0;
        for record in &self.records {
            match record {
                OutputRecord::Output(kind, bytes) if *kind == stream => {
                    parts[part].extend_from_slice(bytes);
                }
                OutputRecord::Output(..) => {}
                OutputRecord::LeftOut(_) => part = 1,
            }
        }
        parts.map(BytesMut::freeze)
    }

    /// The kept length of a stdout that is not text; none when it is text.
    pub fn binary_stdout_length(&self) -> Option<u64> {
        let [first, last] = self.stream_parts(StreamKind::Stdout);
        let gap = !last.is_empty();
        // Where bytes were left out, a character may be cut on either side.
        let text = is_text(&first, false, gap) && is_text(&last, gap, false);
        (!text).then_some((first.len() + last.len()) as u64)
    }

    /// A final stdout that is not text, and its description: its bytes when
    /// the output kept all `length` of them and they are at most `limit`.
    /// None when stdout is text.
    pub fn binary_stdout(&self, length: u64, limit: usize) -> Option<BinaryOutput> {
        self.binary_stdout_length()?;
        let [first, last] = self.stream_parts(StreamKind::Stdout);
        let whole = last.is_empty() && first.len() as u64 == length && first.len() <= limit;
        Some(BinaryOutput {
            bytes: if whole { first } else { Bytes::new() },
            info: BinaryStdout {
                truncated: !whole,
                total_bytes: length,
                limit_bytes: limit as u64,
            },
        })
    }

    /// The output of `streams` as lines of text. A binary stdout of
    /// `binary_stdout` bytes shows as its line where stdout first shows;
    /// otherwise the bytes are as kept. `seen` says how much of each stream
    /// a reader has seen.
    pub fn text(&self, streams: Streams, binary_stdout: Option<u64>, seen: Seen) -> OutputText {
        let mut bytes = Vec::new();
        let mut gap = None;
        let mut unseen = None;
        // Each stream's position, until bytes were left out: how those bytes
        // split between the streams is not known.
        let mut positions = Some([0u64; 2]);
        let mut spans: Vec<(usize, StreamKind)> = Vec::new();
        let span = |spans: &mut Vec<(usize, StreamKind)>, at: usize, stream: StreamKind| {
            if spans.last().is_none_or(|(_, last)| *last != stream) {
                spans.push((at, stream));
            }
        };
        let mut binary_shown = false;
        for record in &self.records {
            let (stream, data) = match record {
                OutputRecord::LeftOut(count) => {
                    gap = Some((bytes.len(), *count));
                    positions = None;
                    // No reader saw the bytes left out.
                    unseen.get_or_insert(bytes.len());
                    continue;
                }
                OutputRecord::Output(stream, data) => (*stream, data),
            };
            let position = positions.as_mut().map(|positions| {
                let index = usize::from(stream == StreamKind::Stderr);
                let position = positions[index];
                positions[index] += data.len() as u64;
                position
            });
            if !streams.takes(stream) {
                continue;
            }
            if stream == StreamKind::Stdout
                && let Some(length) = binary_stdout
            {
                if !binary_shown {
                    binary_shown = true;
                    // The line is new to every reader.
                    unseen.get_or_insert(bytes.len());
                    span(&mut spans, bytes.len(), stream);
                    bytes.extend_from_slice(binary_line(length).as_bytes());
                    bytes.push(b'\n');
                }
                continue;
            }
            if unseen.is_none() {
                let seen = match stream {
                    StreamKind::Stdout => seen.stdout,
                    StreamKind::Stderr => seen.stderr,
                };
                match position {
                    Some(position) if position + data.len() as u64 <= seen => {}
                    Some(position) => {
                        let skip = usize::try_from(seen.saturating_sub(position))
                            .expect("within the record");
                        unseen = Some(bytes.len() + skip);
                    }
                    None => unseen = Some(bytes.len()),
                }
            }
            span(&mut spans, bytes.len(), stream);
            bytes.extend_from_slice(data);
        }
        if self.missing.is_some() {
            unseen.get_or_insert(bytes.len());
        }
        OutputText {
            first_line: 1,
            bytes,
            spans,
            gap,
            missing: self.missing.clone(),
            unseen,
        }
    }
}

/// Whether `bytes` are UTF-8, allowing a character cut at the start or at
/// the end where bytes were left out.
fn is_text(bytes: &[u8], cut_start: bool, cut_end: bool) -> bool {
    let skipped = if cut_start {
        bytes
            .iter()
            .take(3)
            .take_while(|byte| **byte & 0b1100_0000 == 0b1000_0000)
            .count()
    } else {
        0
    };
    match std::str::from_utf8(&bytes[skipped..]) {
        Ok(_) => true,
        Err(error) => cut_end && error.error_len().is_none(),
    }
}

/// The line that stands for a binary stdout of `length` bytes wherever
/// output is shown as text (`runtime.md` § Results and previews).
pub fn binary_line(length: u64) -> String {
    format!("<binary stdout: {length} bytes>")
}

/// Which streams a reading of a whole output takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Streams {
    Both,
    Only(StreamKind),
}

impl Streams {
    fn takes(self, stream: StreamKind) -> bool {
        match self {
            Self::Both => true,
            Self::Only(only) => only == stream,
        }
    }
}

/// How many bytes of each stream a reader has seen, from the start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    pub stdout: u64,
    pub stderr: u64,
}

/// A whole output as lines of text (`runtime.md` § The whole output): the
/// kept bytes of the streams read, in the order the runner read them, as
/// `--raw` prints them, and the notes that stand for bytes they do not hold:
/// where the kept output left bytes out, and at the end the bytes the backend
/// misses. Lines are numbered as the bytes are, so `grep -n` on `--raw` gives
/// the numbers a page shows; a note has no number, and a line that bytes were
/// left out of shows in two parts around the note, both with its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputText {
    /// The number of the first line: 1 for a whole output.
    first_line: u64,
    bytes: Vec<u8>,
    /// Where each run of one stream's bytes starts in `bytes`.
    spans: Vec<(usize, StreamKind)>,
    /// Where the kept output left bytes out, in `bytes`, and how many.
    gap: Option<(usize, u64)>,
    missing: Option<Missing>,
    /// Where the first byte a reader had not seen lies in `bytes`: its
    /// length when only the missing bytes are new; none when the reader saw
    /// everything.
    unseen: Option<usize>,
}

/// One line of an output's text, as a page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    /// A line, or its part on one side of a note: its number, where its
    /// bytes start in the text, and its bytes without the newline.
    Line {
        number: u64,
        offset: usize,
        bytes: &'a [u8],
    },
    /// A line that stands for bytes the text does not hold.
    Note(String),
}

impl Piece<'_> {
    /// The piece as text, a line's bytes decoded lossily.
    pub fn text(&self) -> String {
        match self {
            Self::Line { bytes, .. } => String::from_utf8_lossy(bytes).into_owned(),
            Self::Note(note) => note.clone(),
        }
    }
}

impl OutputText {
    /// Output a reader has not seen, received as `text`, whose first line is
    /// line `first_line` of the output.
    pub fn received(text: &str, first_line: u64) -> Self {
        Self {
            first_line: first_line.max(1),
            bytes: text.as_bytes().to_vec(),
            spans: vec![(0, StreamKind::Stdout)],
            gap: None,
            missing: None,
            unseen: (!text.is_empty()).then_some(0),
        }
    }

    /// The bytes, as `--raw` prints them.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The notes, in order, as `--raw` writes them to stderr.
    pub fn notes(&self) -> Vec<String> {
        self.gap
            .map(|(_, bytes)| gap_note(bytes))
            .into_iter()
            .chain(self.missing.as_ref().map(Missing::line))
            .collect()
    }

    /// The number of the last line: how many lines a whole output has.
    pub fn last_line(&self) -> u64 {
        self.first_line + self.line_count() - 1
    }

    /// Where the bytes a reader has not seen start: the bytes' length when
    /// only the missing bytes are new; none when it saw everything.
    pub fn unseen(&self) -> Option<usize> {
        self.unseen
    }

    /// The text from `from` on as runs of one stream each, the notes among
    /// them on lines of their own, as stderr: what a view of the output in
    /// a terminal shows.
    pub fn chunks(&self, from: usize) -> Vec<OutputChunk> {
        let mut chunks: Vec<OutputChunk> = Vec::new();
        let mut gap = self.gap.filter(|(at, _)| *at >= from);
        for (index, (start, stream)) in self.spans.iter().enumerate() {
            let end = self
                .spans
                .get(index + 1)
                .map_or(self.bytes.len(), |(next, _)| *next);
            let mut at = (*start).max(from);
            if at >= end {
                continue;
            }
            if let Some((left_at, left_out)) = gap
                && (at..end).contains(&left_at)
            {
                push_chunk(&mut chunks, *stream, &self.bytes[at..left_at]);
                push_note(&mut chunks, &gap_note(left_out));
                gap = None;
                at = left_at;
            }
            push_chunk(&mut chunks, *stream, &self.bytes[at..end]);
        }
        if let Some((_, left_out)) = gap {
            push_note(&mut chunks, &gap_note(left_out));
        }
        if let Some(missing) = &self.missing {
            push_note(&mut chunks, &missing.line());
        }
        chunks
    }

    /// How many numbered lines the text has.
    pub fn line_count(&self) -> u64 {
        let newlines = self.bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
        match self.bytes.last() {
            Some(b'\n') | None => newlines,
            Some(_) => newlines + 1,
        }
    }

    /// The line where the bytes a reader has not seen start; none when it
    /// saw everything, and one past the last line when only the missing
    /// bytes are new.
    pub fn unseen_line(&self) -> Option<u64> {
        let offset = self.unseen?;
        if offset == self.bytes.len() {
            return Some(self.last_line() + 1);
        }
        Some(self.line_of(offset))
    }

    /// The pieces from line `from` on, notes included.
    pub fn forward(&self, from: u64) -> Forward<'_> {
        let from = from.max(self.first_line);
        let start = self.line_start(from - self.first_line);
        Forward {
            text: self,
            offset: start.unwrap_or(self.bytes.len()),
            number: from,
            // The gap's note is part of a reading that starts before it.
            gap: self
                .gap
                .filter(|(at, _)| start.is_some_and(|start| *at >= start)),
            missing: self.missing.is_some(),
        }
    }

    /// The pieces from the last back to the first, notes included.
    pub fn backward(&self) -> Backward<'_> {
        let newline = self.bytes.last() == Some(&b'\n');
        Backward {
            text: self,
            end: self.bytes.len(),
            line_start: newline,
            number: self.last_line() + u64::from(newline),
            gap: self.gap,
            missing: self.missing.is_some(),
            done: self.bytes.is_empty(),
        }
    }

    /// Every piece, one to a line: what the pages' view ends with.
    pub fn display(&self) -> String {
        let mut text = String::new();
        for piece in self.forward(self.first_line) {
            text.push_str(&piece.text());
            text.push('\n');
        }
        text
    }

    /// Where line `number` starts in the bytes; their length past the last
    /// line.
    pub fn line_offset(&self, number: u64) -> usize {
        self.line_start(number.saturating_sub(self.first_line))
            .unwrap_or(self.bytes.len())
    }

    /// The number of the line that the byte at `offset` belongs to.
    pub fn line_of(&self, offset: usize) -> u64 {
        let offset = offset.min(self.bytes.len());
        let before = self.bytes[..offset]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count() as u64;
        self.first_line + before
    }

    /// Whether a line starts at `offset`.
    pub fn is_line_start(&self, offset: usize) -> bool {
        offset == 0 || self.bytes.get(offset - 1) == Some(&b'\n')
    }

    /// How many characters of its line lie before `offset`.
    pub fn column(&self, offset: usize) -> usize {
        let offset = offset.min(self.bytes.len());
        let start = self.bytes[..offset]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        String::from_utf8_lossy(&self.bytes[start..offset])
            .chars()
            .count()
    }

    /// Where the line `index` lines after the first starts in `bytes`; none
    /// past the last line.
    fn line_start(&self, index: u64) -> Option<usize> {
        if index == 0 {
            return (!self.bytes.is_empty()).then_some(0);
        }
        let start = self
            .bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .nth(usize::try_from(index - 1).ok()?)
            .map(|(at, _)| at + 1)?;
        (start < self.bytes.len()).then_some(start)
    }
}

fn push_chunk(chunks: &mut Vec<OutputChunk>, stream: StreamKind, bytes: &[u8]) {
    if !bytes.is_empty() {
        chunks.push(OutputChunk {
            stream,
            text: String::from_utf8_lossy(bytes).into_owned(),
        });
    }
}

/// A note on a line of its own.
fn push_note(chunks: &mut Vec<OutputChunk>, note: &str) {
    let after_line = chunks.last().is_none_or(|chunk| chunk.text.ends_with('\n'));
    let separator = if after_line { "" } else { "\n" };
    chunks.push(OutputChunk {
        stream: StreamKind::Stderr,
        text: format!("{separator}{note}\n"),
    });
}

/// The note where the kept output left `bytes` out.
fn gap_note(bytes: u64) -> String {
    format!("[... {bytes} bytes left out ...]")
}

/// The pieces of an output's text from a line on.
pub struct Forward<'a> {
    text: &'a OutputText,
    offset: usize,
    number: u64,
    /// The gap's note, until it is given.
    gap: Option<(usize, u64)>,
    missing: bool,
}

impl<'a> Iterator for Forward<'a> {
    type Item = Piece<'a>;

    fn next(&mut self) -> Option<Piece<'a>> {
        let bytes = &self.text.bytes;
        if let Some((at, left_out)) = self.gap
            && at == self.offset
        {
            self.gap = None;
            return Some(Piece::Note(gap_note(left_out)));
        }
        if self.offset < bytes.len() {
            let end = bytes[self.offset..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |at| self.offset + at);
            let number = self.number;
            // The line goes on after the note, with its number.
            let offset = self.offset;
            if let Some((at, _)) = self.gap
                && offset < at
                && at <= end
            {
                self.offset = at;
                return Some(Piece::Line {
                    number,
                    offset,
                    bytes: &bytes[offset..at],
                });
            }
            self.offset = (end + 1).min(bytes.len());
            if end < bytes.len() {
                self.number += 1;
            }
            return Some(Piece::Line {
                number,
                offset,
                bytes: &bytes[offset..end],
            });
        }
        if self.missing {
            self.missing = false;
            return self
                .text
                .missing
                .as_ref()
                .map(|missing| Piece::Note(missing.line()));
        }
        None
    }
}

/// The pieces of an output's text from its end back to its start.
pub struct Backward<'a> {
    text: &'a OutputText,
    /// Where the text still to give ends.
    end: usize,
    /// Whether `end` is where a line starts, after a newline.
    line_start: bool,
    /// The number of the line that ends at `end`, or starts there.
    number: u64,
    gap: Option<(usize, u64)>,
    missing: bool,
    done: bool,
}

impl<'a> Iterator for Backward<'a> {
    type Item = Piece<'a>;

    fn next(&mut self) -> Option<Piece<'a>> {
        if self.missing {
            self.missing = false;
            if let Some(missing) = &self.text.missing {
                return Some(Piece::Note(missing.line()));
            }
        }
        if let Some((at, left_out)) = self.gap
            && at == self.end
        {
            self.gap = None;
            return Some(Piece::Note(gap_note(left_out)));
        }
        if self.done {
            return None;
        }
        if self.line_start {
            if self.end == 0 {
                self.done = true;
                return None;
            }
            // The newline that ends the line before.
            self.end -= 1;
            self.number -= 1;
            self.line_start = false;
        }
        let bytes = &self.text.bytes;
        let start = bytes[..self.end]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let number = self.number;
        if let Some((at, _)) = self.gap
            && start < at
            && at < self.end
        {
            let piece = &bytes[at..self.end];
            self.end = at;
            return Some(Piece::Line {
                number,
                offset: at,
                bytes: piece,
            });
        }
        let piece = &bytes[start..self.end];
        self.end = start;
        self.line_start = true;
        Some(Piece::Line {
            number,
            offset: start,
            bytes: piece,
        })
    }
}
