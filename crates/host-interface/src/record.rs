//! One command's record: the model's view, which its looks read, and
//! the pages' view (`runtime.md` § Results and previews, § Live output).
//! Every shell environment keeps its commands in these; a record does not
//! know what ran the script.
//!
//! Stream byte views cut only between characters. The model's merged view
//! advances past whole lines, repeating an unfinished line until its newline
//! or the command's end. Tails are the last [`TAIL_CHARS`] characters.

use std::sync::Arc;

use demi_shared_types::{CommandId, OutputChunk, OutputView, StreamKind, StreamView};
use tokio::time::Instant;

use crate::{
    BinaryOutput, CommandMedium, CommandState, CommandStatus, EditedFiles, Newest, Seen, WholeOutput, WholeView,
};

/// How much of a stream's end a view carries.
pub const TAIL_CHARS: usize = 4096;

/// A command's output and state, the cursors of the model's view, and the
/// pages' view.
#[derive(Debug)]
pub struct CommandRecord {
    command_id: CommandId,
    /// The `shell` call that started the command.
    tool_use_id: String,
    started: Instant,
    last_output: Instant,
    phase: Phase,
    stdout: Stream,
    stderr: Stream,
    /// The merged output, in the order it arrived.
    chunks: Vec<Chunk>,
    /// Where the model's next view starts.
    model: Positions,
    page: PageText,
    files: Option<EditedFiles>,
    /// The whole output, once the command ended.
    whole: Option<Arc<WholeOutput>>,
}

/// Where the next byte view of each stream and the model's next merged
/// view start, in bytes.
#[derive(Debug, Default, Clone, Copy)]
struct Positions {
    stdout: usize,
    stderr: usize,
    output: usize,
    /// Whether a view gave the whole output, once the command ended.
    whole: bool,
}

/// The pages' view of a command's output (`runtime.md` § Live output): its
/// last [`TAIL_CHARS`] characters, and how many characters it has held since
/// the command started. No read moves it.
#[derive(Debug, Default)]
struct PageText {
    tail: String,
    chars: u64,
}

impl PageText {
    fn push(&mut self, text: &str) {
        self.chars += text.chars().count() as u64;
        self.tail.push_str(text);
        let cut = self.tail.len() - tail_chars(&self.tail).len();
        self.tail.drain(..cut);
    }
}

/// A command as the pages see it (`runtime.md` § Live output): its handle,
/// the `shell` call that started it, where it is, the last
/// [`TAIL_CHARS`] characters of the pages' view of its output, and how many
/// characters that view has held since the command started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageView {
    pub command_id: CommandId,
    pub tool_use_id: String,
    pub state: PageState,
    pub tail: String,
    pub chars: u64,
    pub running_ms: u64,
}

/// Where a command is, as the pages see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageState {
    Running,
    Exited { exit_code: i32 },
    Aborted,
}

#[derive(Debug)]
enum Phase {
    Running,
    Exited {
        exit_code: i32,
        binary_stdout: Option<BinaryOutput>,
        media: Vec<CommandMedium>,
    },
    Aborted,
}

/// One stream's text as the record holds it.
#[derive(Debug, Default)]
struct Stream {
    text: String,
    /// The stream's length on the Host, which the runner reports while the
    /// record holds only the stream's start.
    host_bytes: Option<u64>,
    /// Its newest bytes beyond its start, while the command runs.
    newest: Option<Newest>,
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
    /// A running command that the `shell` call `tool_use_id` started.
    pub fn new(command_id: CommandId, tool_use_id: String) -> Self {
        let now = Instant::now();
        Self {
            command_id,
            tool_use_id,
            started: now,
            last_output: now,
            phase: Phase::Running,
            stdout: Stream::default(),
            stderr: Stream::default(),
            chunks: Vec::new(),
            model: Positions::default(),
            page: PageText::default(),
            files: None,
            whole: None,
        }
    }

    pub fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running)
    }

    /// How long the command has printed nothing (`runtime.md` § Results and
    /// previews), which no look moves.
    pub fn quiet(&self) -> std::time::Duration {
        Instant::now() - self.last_output
    }

    /// How the command ended; none while it runs.
    pub fn ending(&self) -> Option<Ending> {
        match &self.phase {
            Phase::Running => None,
            Phase::Exited { exit_code, .. } => Some(Ending::Exited(*exit_code)),
            Phase::Aborted => Some(Ending::Aborted),
        }
    }

    /// The text of a stream so far.
    pub fn text(&self, stream: StreamKind) -> &str {
        &self.stream(stream).text
    }

    /// Output that arrived while the command runs, which the model's view
    /// and the pages' view hold. True when the pages' view changed: it
    /// changes no more once the command ended.
    pub fn append_output(&mut self, stream: StreamKind, text: &str) -> bool {
        self.stream_mut(stream).text.push_str(text);
        self.push_chunk(stream, text);
        self.last_output = Instant::now();
        self.append_page(text)
    }

    /// Output only the pages' view holds, such as a stream's newest bytes
    /// beyond the runner's view of it. True when the pages' view changed.
    pub fn append_page_output(&mut self, text: &str) -> bool {
        self.last_output = Instant::now();
        self.append_page(text)
    }

    /// `stream` grew to `length` bytes on the Host, beyond what the record
    /// holds, such as past the runner's view of it: the command is not idle
    /// (`runtime.md` § Results and previews).
    pub fn grew(&mut self, stream: StreamKind, length: u64) {
        let host = &mut self.stream_mut(stream).host_bytes;
        *host = Some(host.map_or(length, |known| known.max(length)));
        self.last_output = Instant::now();
    }

    /// The newest bytes of `stream` beyond what the record holds of its
    /// start, as the runner last sent them: they replace the ones before.
    pub fn set_newest(&mut self, stream: StreamKind, offset: u64, left_out: u64, text: String) {
        self.stream_mut(stream).newest = Some(Newest {
            stream,
            offset,
            left_out,
            text,
        });
    }

    fn append_page(&mut self, text: &str) -> bool {
        if !self.is_running() || text.is_empty() {
            return false;
        }
        self.page.push(text);
        true
    }

    pub fn set_files(&mut self, files: EditedFiles) {
        self.files = Some(files);
    }

    /// Ends the command with its whole output, its binary stdout and the
    /// media its commands returned; `page` is what the end adds to the
    /// pages' view (`runtime.md` § Live output). True when the pages' view
    /// changed: a command that had ended already keeps the end the pages
    /// were shown.
    pub fn settle(
        &mut self,
        ending: Ending,
        whole: Arc<WholeOutput>,
        binary_stdout: Option<BinaryOutput>,
        media: Vec<CommandMedium>,
        page: &str,
    ) -> bool {
        let running = self.is_running();
        if running {
            self.page.push(page);
        }
        self.whole = Some(whole);
        self.last_output = Instant::now();
        self.phase = match ending {
            Ending::Exited(exit_code) => Phase::Exited {
                exit_code,
                binary_stdout,
                media,
            },
            Ending::Aborted => Phase::Aborted,
        };
        running
    }

    /// Marks a running command stopped whose streams never ended. True when
    /// it ran until now.
    pub fn mark_aborted(&mut self) -> bool {
        if !self.is_running() {
            return false;
        }
        self.last_output = Instant::now();
        self.phase = Phase::Aborted;
        true
    }

    /// The model's status: each stream's output since the model's last view,
    /// at most `max_output_bytes` of it (all of it when zero), and its tail;
    /// once the command ended, its whole output and how much of each stream
    /// the model had seen. The view moves only the model's positions.
    pub fn status(&mut self, max_output_bytes: usize, hint: Option<String>) -> CommandStatus {
        let whole = self.whole.clone().map(|output| {
            let seen = if self.model.whole {
                Seen {
                    stdout: u64::MAX,
                    stderr: u64::MAX,
                }
            } else {
                // Project the model's merged line boundary onto each stream.
                // The byte views may have delivered part of the next line.
                let mut seen = Seen::default();
                for chunk in &self.chunks {
                    let bytes = self
                        .model
                        .output
                        .saturating_sub(chunk.offset)
                        .min(chunk.text.len()) as u64;
                    match chunk.stream {
                        StreamKind::Stdout => seen.stdout += bytes,
                        StreamKind::Stderr => seen.stderr += bytes,
                    }
                }
                seen
            };
            self.model.whole = true;
            WholeView { output, seen }
        });
        let unreceived = match whole {
            Some(_) => 0,
            None => [&self.stdout, &self.stderr]
                .iter()
                .map(|stream| {
                    stream
                        .host_bytes
                        .unwrap_or(0)
                        .saturating_sub(stream.text.len() as u64)
                })
                .sum(),
        };
        let running = self.is_running();
        let newest = if running {
            [&self.stdout, &self.stderr]
                .into_iter()
                .filter_map(|stream| stream.newest.clone())
                .collect()
        } else {
            Vec::new()
        };
        let positions = &mut self.model;
        let stdout = stream_view(&self.stdout, &mut positions.stdout, max_output_bytes);
        let stderr = stream_view(&self.stderr, &mut positions.stderr, max_output_bytes);
        let output = merged_view(
            &self.chunks,
            &mut positions.output,
            max_output_bytes,
            running,
        );
        let now = Instant::now();
        let state = match &self.phase {
            Phase::Running => CommandState::Running { hint },
            Phase::Exited {
                exit_code,
                binary_stdout,
                media,
            } => CommandState::Exited {
                exit_code: *exit_code,
                binary_stdout: binary_stdout.clone(),
                media: media.clone(),
            },
            Phase::Aborted => CommandState::Aborted,
        };
        CommandStatus {
            command_id: self.command_id.clone(),
            stdout,
            stderr,
            output,
            unreceived,
            newest,
            whole,
            running_ms: millis(now - self.started),
            idle_ms: millis(now - self.last_output),
            state,
            files: self.files.clone(),
        }
    }

    /// The command as the pages see it now.
    pub fn page_view(&self) -> PageView {
        PageView {
            command_id: self.command_id.clone(),
            tool_use_id: self.tool_use_id.clone(),
            state: match &self.phase {
                Phase::Running => PageState::Running,
                Phase::Exited { exit_code, .. } => PageState::Exited {
                    exit_code: *exit_code,
                },
                Phase::Aborted => PageState::Aborted,
            },
            tail: self.page.tail.clone(),
            chars: self.page.chars,
            running_ms: millis(Instant::now() - self.started),
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

    fn push_chunk(&mut self, stream: StreamKind, text: &str) {
        if text.is_empty() {
            return;
        }
        let offset = merged_len(&self.chunks);
        self.chunks.push(Chunk {
            stream,
            text: text.to_owned(),
            offset,
        });
    }
}

/// The merged output's length, in bytes.
fn merged_len(chunks: &[Chunk]) -> usize {
    chunks
        .last()
        .map_or(0, |chunk| chunk.offset + chunk.text.len())
}

/// A reader's view of the merged output from `position`, which it moves.
fn merged_view(
    chunks: &[Chunk],
    position: &mut usize,
    max_output_bytes: usize,
    running: bool,
) -> OutputView {
    let total = merged_len(chunks);
    let start = (*position).min(total);
    let mut remaining = budget(total - start, max_output_bytes);
    let mut views = Vec::new();
    let mut delivered = 0;
    for chunk in chunks {
        if remaining == 0 {
            break;
        }
        let at = start + delivered;
        if chunk.offset + chunk.text.len() <= at {
            continue;
        }
        let from = at - chunk.offset;
        let piece = cut(&chunk.text, from, remaining);
        delivered += piece.len();
        remaining = remaining.saturating_sub(piece.len());
        views.push(OutputChunk {
            stream: chunk.stream,
            text: piece.to_owned(),
        });
        // The budget ended inside this chunk, before a character.
        if from + piece.len() < chunk.text.len() {
            break;
        }
    }
    let next = start + delivered;
    let text: String = views.iter().map(|chunk| chunk.text.as_str()).collect();
    *position = if running {
        start + text.rfind('\n').map_or(0, |at| at + 1)
    } else {
        next
    };
    let line = 1 + chunks
        .iter()
        .map(|chunk| {
            let before = start.saturating_sub(chunk.offset).min(chunk.text.len());
            chunk.text.as_bytes()[..before]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64
        })
        .sum::<u64>();
    let mut tail = String::new();
    for chunk in chunks.iter().rev() {
        if tail.chars().count() >= TAIL_CHARS {
            break;
        }
        tail.insert_str(0, &chunk.text);
    }
    OutputView {
        offset: *position as u64,
        line,
        text,
        tail: tail_chars(&tail).to_owned(),
        chunks: views,
        bytes: total as u64,
        truncated: next < total,
    }
}

/// A reader's view of one stream from `position`, which it moves.
fn stream_view(stream: &Stream, position: &mut usize, max_output_bytes: usize) -> StreamView {
    let total = stream.text.len();
    let start = stream.text.floor_char_boundary((*position).min(total));
    let delta = cut(&stream.text, start, budget(total - start, max_output_bytes)).to_owned();
    let next = start + delta.len();
    *position = next;
    StreamView {
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
