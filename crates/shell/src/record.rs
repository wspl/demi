//! One command's record, which `shell_status` reads, and the views cut from
//! it. Every shell environment keeps its commands in these; a record does not
//! know what ran the script.
//!
//! Views count bytes and cut text only between characters: a stream's cursor
//! advances by the bytes a view delivered, and a view never splits a
//! character. Tails are the last [`TAIL_CHARS`] characters.

use bytes::Bytes;
use demi_core::{BinaryStdout, CommandId, OutputChunk, OutputView, ShellId, StreamKind, StreamView};
use tokio::time::Instant;

use crate::{BinaryOutput, CommandState, CommandStatus, EditedFiles};

/// How much of a stream's end a view carries.
pub const TAIL_CHARS: usize = 4096;

/// A command's output and state, and the cursors of its views.
#[derive(Debug)]
pub struct CommandRecord {
    shell_id: ShellId,
    command_id: CommandId,
    output_dir: Option<String>,
    started: Instant,
    last_output: Instant,
    phase: Phase,
    stdout: Stream,
    stderr: Stream,
    /// The merged output, in the order it arrived.
    chunks: Vec<Chunk>,
    /// Where the next merged view starts, in bytes.
    output_cursor: usize,
    files: Option<EditedFiles>,
}

#[derive(Debug)]
enum Phase {
    Running,
    Exited {
        exit_code: i32,
        binary_stdout: Option<BinaryOutput>,
    },
    Aborted,
}

/// One stream's text as the record holds it.
#[derive(Debug, Default)]
struct Stream {
    text: String,
    /// Where the next view starts, in bytes.
    cursor: usize,
    /// The stream's length on the Host when the record holds only a view of
    /// it, such as a runner's head and tail.
    host_bytes: Option<u64>,
}

#[derive(Debug)]
struct Chunk {
    stream: StreamKind,
    text: String,
    /// Where the chunk starts in the merged output, in bytes.
    offset: usize,
}

/// How a command ended, when its streams are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Exited(i32),
    /// It was stopped: its streams settle, its status is the stop.
    Aborted,
}

impl CommandRecord {
    pub fn new(shell_id: ShellId, command_id: CommandId) -> Self {
        let now = Instant::now();
        Self {
            shell_id,
            command_id,
            output_dir: None,
            started: now,
            last_output: now,
            phase: Phase::Running,
            stdout: Stream::default(),
            stderr: Stream::default(),
            chunks: Vec::new(),
            output_cursor: 0,
            files: None,
        }
    }

    pub fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    pub fn shell_id(&self) -> &ShellId {
        &self.shell_id
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running)
    }

    /// The text of a stream so far.
    pub fn text(&self, stream: StreamKind) -> &str {
        &self.stream(stream).text
    }

    /// Output that arrived while the command runs.
    pub fn append_output(&mut self, stream: StreamKind, text: &str) {
        self.stream_mut(stream).text.push_str(text);
        self.push_chunk(stream, text);
        self.last_output = Instant::now();
    }

    /// Where the command's output files are on the Host.
    pub fn set_output_dir(&mut self, directory: String) {
        self.output_dir = Some(directory);
    }

    /// The lengths of the streams on the Host, when the record holds only a
    /// view of them.
    pub fn set_host_bytes(&mut self, stdout: u64, stderr: u64) {
        self.stdout.host_bytes = Some(stdout);
        self.stderr.host_bytes = Some(stderr);
    }

    pub fn set_files(&mut self, files: EditedFiles) {
        self.files = Some(files);
    }

    /// Ends the command with its final streams. What the end adds to a
    /// stream that was streamed follows the chunks the views showed, so a
    /// merged cursor stays valid; a binary stdout is presented anew, from its
    /// placeholder.
    pub fn settle(
        &mut self,
        ending: Ending,
        stdout: String,
        stderr: String,
        binary_stdout: Option<BinaryOutput>,
    ) {
        if binary_stdout.is_some() {
            self.chunks.clear();
            self.output_cursor = 0;
            self.stdout.cursor = 0;
        } else {
            for (stream, text) in [(StreamKind::Stdout, &stdout), (StreamKind::Stderr, &stderr)] {
                if let Some(rest) = text.strip_prefix(self.stream(stream).text.as_str()) {
                    let rest = rest.to_owned();
                    self.push_chunk(stream, &rest);
                }
            }
        }
        self.stdout.text = stdout;
        self.stderr.text = stderr;
        if self.chunks.is_empty() {
            self.push_chunk(StreamKind::Stdout, &self.stdout.text.clone());
            self.push_chunk(StreamKind::Stderr, &self.stderr.text.clone());
        }
        self.cover();
        self.last_output = Instant::now();
        self.phase = match ending {
            Ending::Exited(exit_code) => Phase::Exited {
                exit_code,
                binary_stdout,
            },
            Ending::Aborted => Phase::Aborted,
        };
    }

    /// Marks a running command stopped whose streams never ended.
    pub fn mark_aborted(&mut self) {
        if self.is_running() {
            self.last_output = Instant::now();
            self.phase = Phase::Aborted;
        }
    }

    /// The status the tools see: each stream's output since the last view,
    /// at most `max_output_bytes` of it (all of it when zero), and its tail.
    /// The view advances the cursors.
    pub fn status(&mut self, max_output_bytes: usize, hint: Option<String>) -> CommandStatus {
        let stdout = stream_view(&mut self.stdout, &self.output_dir, "stdout", max_output_bytes);
        let stderr = stream_view(&mut self.stderr, &self.output_dir, "stderr", max_output_bytes);
        let output = self.merged_view(max_output_bytes);
        let now = Instant::now();
        let state = match &self.phase {
            Phase::Running => CommandState::Running { hint },
            Phase::Exited {
                exit_code,
                binary_stdout,
            } => CommandState::Exited {
                exit_code: *exit_code,
                binary_stdout: binary_stdout.clone(),
            },
            Phase::Aborted => CommandState::Aborted,
        };
        CommandStatus {
            shell_id: self.shell_id.clone(),
            command_id: self.command_id.clone(),
            output_dir: self.output_dir.clone(),
            stdout,
            stderr,
            output,
            running_ms: millis(now - self.started),
            idle_ms: millis(now - self.last_output),
            state,
            files: self.files.clone(),
        }
    }

    fn stream(&self, stream: StreamKind) -> &Stream {
        match stream {
            StreamKind::Stdout => &self.stdout,
            StreamKind::Stderr => &self.stderr,
        }
    }

    fn stream_mut(&mut self, stream: StreamKind) -> &mut Stream {
        match stream {
            StreamKind::Stdout => &mut self.stdout,
            StreamKind::Stderr => &mut self.stderr,
        }
    }

    fn merged_len(&self) -> usize {
        self.chunks
            .last()
            .map_or(0, |chunk| chunk.offset + chunk.text.len())
    }

    fn push_chunk(&mut self, stream: StreamKind, text: &str) {
        if text.is_empty() {
            return;
        }
        let offset = self.merged_len();
        self.chunks.push(Chunk {
            stream,
            text: text.to_owned(),
            offset,
        });
    }

    /// Rebuilds the chunks, and the merged cursor with them, when they no
    /// longer describe the streams' texts.
    fn cover(&mut self) {
        let covered = |stream: StreamKind| -> usize {
            self.chunks
                .iter()
                .filter(|chunk| chunk.stream == stream)
                .map(|chunk| chunk.text.len())
                .sum()
        };
        if covered(StreamKind::Stdout) == self.stdout.text.len()
            && covered(StreamKind::Stderr) == self.stderr.text.len()
        {
            return;
        }
        self.chunks.clear();
        self.output_cursor = 0;
        self.push_chunk(StreamKind::Stdout, &self.stdout.text.clone());
        self.push_chunk(StreamKind::Stderr, &self.stderr.text.clone());
    }

    fn merged_view(&mut self, max_output_bytes: usize) -> OutputView {
        let total = self.merged_len();
        let start = self.output_cursor.min(total);
        let mut remaining = budget(total - start, max_output_bytes);
        let mut chunks = Vec::new();
        let mut delivered = 0;
        for chunk in &self.chunks {
            if remaining == 0 {
                break;
            }
            let position = start + delivered;
            if chunk.offset + chunk.text.len() <= position {
                continue;
            }
            let from = position - chunk.offset;
            let piece = cut(&chunk.text, from, remaining);
            delivered += piece.len();
            remaining = remaining.saturating_sub(piece.len());
            chunks.push(OutputChunk {
                stream: chunk.stream,
                text: piece.to_owned(),
            });
            // The budget ended inside this chunk, before a character.
            if from + piece.len() < chunk.text.len() {
                break;
            }
        }
        let next = start + delivered;
        self.output_cursor = next;
        let text: String = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
        let mut tail = String::new();
        for chunk in self.chunks.iter().rev() {
            if tail.chars().count() >= TAIL_CHARS {
                break;
            }
            tail.insert_str(0, &chunk.text);
        }
        OutputView {
            path: self.output_dir.clone(),
            offset: next as u64,
            text,
            tail: tail_chars(&tail).to_owned(),
            chunks,
            bytes: total as u64,
            truncated: next < total,
        }
    }
}

fn stream_view(
    stream: &mut Stream,
    output_dir: &Option<String>,
    name: &str,
    max_output_bytes: usize,
) -> StreamView {
    let total = stream.text.len();
    let start = stream.text.floor_char_boundary(stream.cursor.min(total));
    let delta = cut(&stream.text, start, budget(total - start, max_output_bytes)).to_owned();
    let next = start + delta.len();
    stream.cursor = next;
    StreamView {
        path: output_dir.as_ref().map(|directory| format!("{directory}/{name}.txt")),
        offset: next as u64,
        delta,
        tail: tail_chars(&stream.text).to_owned(),
        bytes: stream.host_bytes.unwrap_or(total as u64),
        truncated: next < total,
    }
}

/// How many bytes a view may take of `available`: all of them when the
/// limit is zero.
fn budget(available: usize, limit: usize) -> usize {
    if limit == 0 {
        available
    } else {
        available.min(limit)
    }
}

/// At most `take` bytes of `text` from `from`, ending between characters. A
/// budget smaller than the next character still takes that character, so a
/// view always moves on.
fn cut(text: &str, from: usize, take: usize) -> &str {
    let from = text.floor_char_boundary(from.min(text.len()));
    if take == 0 || from == text.len() {
        return "";
    }
    let mut end = text.floor_char_boundary((from + take).min(text.len()));
    if end == from {
        end = text.ceil_char_boundary(from + 1);
    }
    &text[from..end]
}

/// The last [`TAIL_CHARS`] characters of `text`.
fn tail_chars(text: &str) -> &str {
    let start = text
        .char_indices()
        .rev()
        .nth(TAIL_CHARS - 1)
        .map_or(0, |(index, _)| index);
    &text[start..]
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// A final stdout at the boundary above the shell: valid UTF-8 is its text;
/// anything else stays bytes, at most `limit` of them, with a placeholder in
/// the text, so raw binary never enters a text view. `raw_path` is where the
/// whole stream is on the Host, when it is kept.
pub fn final_stdout_boundary(
    bytes: Bytes,
    limit: usize,
    raw_path: Option<&str>,
) -> (String, Option<BinaryOutput>) {
    if let Ok(text) = std::str::from_utf8(&bytes) {
        return (text.to_owned(), None);
    }
    let total = bytes.len();
    let truncated = total > limit;
    let exceeds = if truncated {
        format!(", exceeds the {limit}-byte binary limit")
    } else {
        String::new()
    };
    let kept = match raw_path {
        Some(path) => format!("; raw bytes at {path}"),
        None => "; not kept beyond this view".into(),
    };
    let text = format!("<binary stdout: {total} bytes{exceeds}{kept}>\n");
    let bytes = if truncated { bytes.slice(..limit) } else { bytes };
    let info = BinaryStdout {
        truncated,
        total_bytes: total as u64,
        limit_bytes: limit as u64,
    };
    (text, Some(BinaryOutput { bytes, info }))
}
