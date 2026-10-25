//! A shell tool's result: the text the model reads, the media its job viewed
//! and the line about a binary stdout, and the bounded view the user sees, all
//! from one command status.
//!
//! The model keeps its own place in each command's output, and a result shows
//! the output since its last look; what the pages are sent never moves it. A
//! result is cut once, where it is made, by one rule for every model, so replay
//! sends it unchanged.
//!
//! `idleMs` counts from the last time the output grew, also beyond the first 8
//! KiB the backend holds (the runner reports such growth within seconds), so a
//! build that logged a second ago is not taken for a hung one. A script that
//! has ended while a background task keeps its command running says so, since a
//! model that started `nohup server &` took such a command for a hang.
//!
//! # Media
//!
//! Media reach the model only in the result that reports the job's end, the
//! `shell` result or the end report, so each medium is attached once and a
//! job's media arrive together in order. A `demi shell status` that shows the
//! end attaches nothing, and the end report then still comes carrying the media
//! alone.
//!
//! The limits on what a result attaches exist so that a request stays small and
//! every provider takes it:
//!
//! - One step's result is a small part of a request.
//! - The media's base64 takes at most half of the model's body limit to leave
//!   room for the history.
//! - A medium that does not fit is skipped while a later, smaller one may still
//!   be attached.
//!
//! A line about a medium says it was shown to the model, not to the user,
//! which the model would otherwise take it for, and names the attachment the
//! backend keeps it as, which a reply shows by its `attachment_id`.
//!
//! The runner keeps media apart from the output, so no cut of the output
//! touches them. When the job ends the backend stores each medium the result
//! attaches as a blob before the result reports the end, and stores none it
//! does not attach: the model views the file again when it needs it.

use std::sync::Arc;

use demi_agent_session::ToolOutcome;
use demi_agent_store::images;
use demi_agent_transcript::REPLAY_CHARS;
use demi_host_interface::{
    BinaryOutput, CommandMedium, CommandState, CommandStatus, Newest, OutputText,
    Piece, Seen, Streams, Unreachable, WholeOutput, WholeView,
};
use demi_provider_common::{MediaBytes, RequestLimits, ResultPart, ToolResultKinds};
use demi_shared_types::{
    B64Bytes, CommandId, Model, ModelMediaKind, OutputChunk, OutputView, ShellToolView,
    ShellViewStatus, StreamView, ToolView, model_media_type_for,
};

use super::PAGE_CHARS;

/// The characters of merged output a shell view keeps, from the end.
///
/// Bounded because a running command's output has no end and a view goes into
/// the model's context window.
pub(super) const VIEW_CHARS: usize = 32_768;
/// The largest video or document: about ten minutes of video at a
/// viewing-grade encoding, under the inline payload ceiling the major APIs
/// enforce. An image is fitted instead.
const MEDIUM_CAP_BYTES: u64 = 16 * 1024 * 1024;
/// How many of a running command's newest lines its result points to.
///
/// Bounded because the lines go into the model's context window.
const NEWEST_LINES: u64 = 50;

/// What a look at a command says beside its status and output.
#[derive(Debug, Clone, Copy, Default)]
pub struct Look {
    /// Why the call returned while its command runs, which its result says.
    pub moved: Option<Moved>,
    /// The command's Host is unreachable, so its silence is not the command's:
    /// a look from another Host says so
    /// (`crates/agent-server/src/server/shell_output.rs`).
    pub unreachable: Option<Unreachable>,
}

/// Why a `shell` call returned while its command runs: its command moved to
/// the background, and its result says why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// The user's send now ended the window.
    SentNow,
    /// A steer, an agent's message or a command report arrived, and the
    /// command had run a while.
    Input,
    /// Another call of the step returned, and the command had run a while.
    Call,
    /// The window's time passed (`crates/agent-session/src/reports.rs`).
    TimePassed,
}

/// The `shell` call's outcome for `status`: its text, the media it attaches and
/// the lines about them, and its view.
///
/// `model` is the call's, and its vendor takes requests within `limits`. A
/// command that exited with a status other than 0, or was stopped or lost, is
/// an error to the provider.
pub(super) async fn shell_outcome(
    status: &CommandStatus,
    model: &Model,
    limits: RequestLimits,
    look: Look,
) -> ToolOutcome {
    let text = unseen_output(status);
    let mut output = vec![ResultPart::Text(result_text(status, &text, look))];
    let ended_with = match &status.state {
        CommandState::Exited {
            binary_stdout,
            media,
            ..
        } => Some((media, binary_stdout.as_ref())),
        CommandState::Lost { media, .. } => Some((media, None)),
        CommandState::Running { .. } | CommandState::Aborted => None,
    };
    if let Some((media, binary_stdout)) = ended_with {
        let (parts, lines) = attached_media(media, binary_stdout, model, limits).await;
        output.extend(parts);
        if !lines.is_empty() {
            output.push(ResultPart::Text(lines.join("\n")));
        }
    }
    let is_error = match status.state {
        CommandState::Running { .. } => false,
        CommandState::Exited { exit_code, .. } => exit_code != 0,
        CommandState::Aborted | CommandState::Lost { .. } => true,
    };
    ToolOutcome {
        output,
        is_error,
        view: Some(ToolView::Shell(shell_view(status, &text))),
        effect: None,
    }
}

/// What a look at a command shows the model, as a result shows it: the text
/// of `demi shell status`.
pub fn look_text(status: &CommandStatus, look: Look) -> String {
    result_text(status, &unseen_output(status), look)
}

/// What a look at `command` shows the model from its whole output `whole`, from
/// the node's own place `seen` in it.
///
/// This is the look of a node that does not hold the command, such as at
/// another agent's command, or at one whose shells ended
/// (`crates/agent-server/src/server/shell_output.rs`). The command is in
/// `state`, has run `running_ms` and printed nothing for `idle_ms`.
pub fn whole_look_text(
    command: &CommandId,
    state: CommandState,
    running_ms: u64,
    idle_ms: u64,
    whole: Arc<WholeOutput>,
    seen: Seen,
    look: Look,
) -> String {
    look_text(&whole_status(command, state, running_ms, idle_ms, whole, seen), look)
}

/// The status of `command` as its whole output `whole` shows it from the
/// place `seen` in it, for a look or a report that reads the whole output.
pub fn whole_status(
    command: &CommandId,
    state: CommandState,
    running_ms: u64,
    idle_ms: u64,
    whole: Arc<WholeOutput>,
    seen: Seen,
) -> CommandStatus {
    let stream = || StreamView {
        offset: 0,
        delta: String::new(),
        tail: String::new(),
        bytes: 0,
        truncated: false,
    };
    CommandStatus {
        command_id: command.clone(),
        stdout: stream(),
        stderr: stream(),
        output: OutputView {
            offset: 0,
            line: 1,
            text: String::new(),
            tail: String::new(),
            chunks: Vec::new(),
            bytes: 0,
            truncated: false,
        },
        unreceived: 0,
        newest: Vec::new(),
        whole: Some(WholeView {
            output: whole,
            seen,
        }),
        running_ms,
        idle_ms,
        state,
        files: None,
    }
}

/// The output the model has not seen: once the command ended, the rest of
/// its whole output; while it runs, what the backend received since the
/// model's last look.
fn unseen_output(status: &CommandStatus) -> OutputText {
    match &status.whole {
        Some(whole) => whole
            .output
            .text(Streams::Both, binary_length(status), whole.seen),
        None => OutputText::received(&status.output.text, status.output.line),
    }
}

/// The lines the model reads.
///
/// In order, they are:
///
/// 1. the status, the handles and timings that matter;
/// 2. the output since the model's last look within the replay bound, while the
///    command runs each stream's newest lines beyond its start;
/// 3. the next step, after the line that says why the command moved to the
///    background when the call returned while it ran.
fn result_text(status: &CommandStatus, text: &OutputText, look: Look) -> String {
    let command = &status.command_id;
    let running = matches!(status.state, CommandState::Running { .. });
    let unreachable = look.unreachable.filter(|_| running);
    let mut before = vec![status_line(&status.state, unreachable)];
    if let CommandState::Exited { exit_code, .. } = status.state {
        before.push(format!("exitCode: {exit_code}"));
    }
    before.push(format!("commandId: {command}"));
    if running {
        before.push(format!("runningMs: {}", status.running_ms));
        // While its Host is unreachable, the silence is not the command's.
        if unreachable.is_none() {
            before.push(format!("idleMs: {}", status.idle_ms));
        }
    }
    let newest = newest_of(status);
    let mut after = Vec::new();
    after.extend(unreceived_line(status, &newest));
    match &status.state {
        CommandState::Running { hint, outliving } => {
            match look.moved {
                Some(Moved::SentNow | Moved::Input) => after.push(format!(
                    "[A message arrived, so command {command} moved to the background. It keeps running; its end wakes you.]"
                )),
                Some(Moved::Call) => after.push(format!(
                    "[Another call returned, so command {command} moved to the background. It keeps running; its end wakes you.]"
                )),
                Some(Moved::TimePassed) => {
                    after.push(
                        "[The command is still running, so it moved to the background; its end wakes you.]".to_owned(),
                    );
                }
                None => {}
            }
            after.extend(outliving_line(outliving));
            after.extend(hint.clone());
        }
        CommandState::Exited { .. } | CommandState::Aborted | CommandState::Lost { .. } => {}
    }
    // The other lines, the output's label among them, each with its newline.
    let markers: usize = newest.iter().map(|newest| newest_marker(newest).chars().count() + 1).sum();
    let others: usize = before
        .iter()
        .chain(&after)
        .map(|line| line.chars().count() + 1)
        .sum::<usize>()
        + markers
        + "output:\n".len();
    let output = output_lines(text, command, &newest, REPLAY_CHARS.saturating_sub(others));
    let mut lines = before;
    if output.is_empty() {
        lines.push("output: (empty)".to_owned());
    } else {
        lines.push("output:".to_owned());
        lines.extend(output);
    }
    lines.extend(after);
    lines.join("\n")
}

/// The line that says where the command is, as the model reads it
/// (`crates/agent-server/src/server/shell_output.rs`): running, as far as Demi knows
/// while its Host is `unreachable`; exited; stopped; or lost, with why.
fn status_line(state: &CommandState, unreachable: Option<Unreachable>) -> String {
    match (state, unreachable) {
        (CommandState::Running { .. }, Some(unreachable)) => format!(
            "status: running, as far as Demi knows; its Host has been unreachable for {}, and its runner keeps it running",
            super::duration(unreachable.away_ms),
        ),
        (CommandState::Running { .. }, None) => "status: running".to_owned(),
        (CommandState::Exited { .. }, _) => "status: exited".to_owned(),
        (CommandState::Aborted, _) => "status: stopped".to_owned(),
        (CommandState::Lost { reason, .. }, _) => format!("status: lost: {reason}"),
    }
}

/// What a command's report carries of `status`'s output
/// (`crates/agent-session/src/reports.rs`).
///
/// It is the output since the node's last look as a result shows it, within
/// `budget` characters, the lines that name what it leaves out among them;
/// empty when there is none.
pub fn report_output(status: &CommandStatus, budget: usize) -> String {
    let text = unseen_output(status);
    let newest = newest_of(status);
    let unreceived = unreceived_line(status, &newest);
    let markers: usize = newest.iter().map(|newest| newest_marker(newest).chars().count() + 1).sum();
    let reserved = markers + unreceived.iter().map(|line| line.chars().count() + 1).sum::<usize>();
    let mut lines = output_lines(&text, &status.command_id, &newest, budget.saturating_sub(reserved));
    lines.extend(unreceived);
    lines.join("\n")
}

/// What the result of a call that something other than its tool ended shows of
/// the command it watched (`crates/agent-session/src/core.rs`).
///
/// It is the command's output since the node's last look as a report carries
/// it, within `budget` characters, and the user's view of the call.
pub fn interrupted_look(status: &CommandStatus, budget: usize) -> (String, ShellToolView) {
    (report_output(status, budget), shell_view(status, &unseen_output(status)))
}

/// The streams' newest lines a running command's look shows, beyond each
/// stream's start.
fn newest_of(status: &CommandStatus) -> Vec<&Newest> {
    let running = matches!(status.state, CommandState::Running { .. });
    status
        .newest
        .iter()
        .filter(|newest| running && !newest.text.is_empty())
        .collect()
}

/// The line that says how much of a running command's output the backend
/// has not received yet, when no newest lines stand for it.
fn unreceived_line(status: &CommandStatus, newest: &[&Newest]) -> Option<String> {
    let running = matches!(status.state, CommandState::Running { .. });
    (running && status.unreceived > 0 && newest.is_empty()).then(|| {
        format!(
            "[... {} bytes not shown so far; the newest: demi shell output {} --tail {NEWEST_LINES} ...]",
            status.unreceived, status.command_id
        )
    })
}

/// The output lines a result shows within `budget` characters.
///
/// They are the output since the model's last look from its start, and each
/// stream's newest lines after the line that counts what lies between, each
/// part within half of the budget when there are newest lines.
fn output_lines(text: &OutputText, command: &CommandId, newest: &[&Newest], budget: usize) -> Vec<String> {
    // The start takes all of the bound, or half of it beside newest lines.
    let start_budget = if newest.is_empty() {
        budget
    } else {
        budget / 2
    };
    let mut lines = text
        .unseen_line()
        .map_or_else(Vec::new, |from| cut(text, from, command, start_budget));
    let used: usize = lines.iter().map(|line| line.chars().count() + 1).sum();
    if !newest.is_empty() {
        let each = budget.saturating_sub(used) / newest.len();
        for newest in newest {
            lines.push(newest_marker(newest));
            lines.extend(newest_tail(newest, each));
        }
    }
    lines
}

/// The line that names the background tasks that keep a command running
/// once its script has ended, and says its stop stops them; none while the script runs.
fn outliving_line(tasks: &[String]) -> Option<String> {
    let quoted: Vec<String> = tasks.iter().map(|task| format!("\"{task}\"")).collect();
    let (last, rest) = quoted.split_last()?;
    if rest.is_empty() {
        return Some(format!(
            "the script has ended; its background task {last} keeps the command running, and stopping the command stops it"
        ));
    }
    Some(format!(
        "the script has ended; its background tasks {} and {last} keep the command running, and stopping the command stops them",
        rest.join(", ")
    ))
}

/// The line before a stream's newest lines, which counts the bytes left out
/// between the stream's start and them.
fn newest_marker(newest: &Newest) -> String {
    format!(
        "[... {} bytes of {} not shown; its newest lines follow ...]",
        newest.left_out, newest.stream
    )
}

/// A stream's newest whole lines that fit in `budget` characters, from the
/// end: a line cut at their start is left out, and the last line alone shows
/// its end when it is too long.
fn newest_tail(newest: &Newest, budget: usize) -> Vec<String> {
    let text = match newest.text.split_once('\n') {
        Some((_, rest)) if newest.left_out > 0 && !rest.is_empty() => rest,
        _ => newest.text.as_str(),
    };
    take_end(&OutputText::received(text, 1), budget).0
}

/// The length of a binary stdout, which the output shows as one line.
fn binary_length(status: &CommandStatus) -> Option<u64> {
    match &status.state {
        CommandState::Exited {
            binary_stdout: Some(binary),
            ..
        } => Some(binary.info.total_bytes),
        _ => None,
    }
}

/// A piece of the output as a result shows it: its text, and for a line
/// where its bytes start and how many there are.
struct Shown {
    text: String,
    chars: usize,
    line: Option<(usize, usize)>,
}

impl Shown {
    fn new(piece: &Piece<'_>) -> Self {
        let text = piece.text();
        let line = match piece {
            Piece::Line { offset, bytes, .. } => Some((*offset, bytes.len())),
            Piece::Note(_) => None,
        };
        Self {
            chars: text.chars().count(),
            text,
            line,
        }
    }
}

/// The lines of `text` from line `from` on within `budget` characters, each
/// with its newline.
///
/// All of them when they fit. Otherwise as many whole lines from the start and
/// from the end as fit in half each of what the budget leaves after the line
/// between them, which names what it leaves out and the command that prints it.
/// A single line too long for its half shows in part.
fn cut(text: &OutputText, from: u64, command: &CommandId, budget: usize) -> Vec<String> {
    let mut all = Vec::new();
    let mut used = 0;
    for piece in text.forward(from) {
        let shown = Shown::new(&piece);
        used += shown.chars + 1;
        if used > budget {
            return cut_middle(text, from, command, budget);
        }
        all.push(shown.text);
    }
    all
}

fn cut_middle(text: &OutputText, from: u64, command: &CommandId, budget: usize) -> Vec<String> {
    // Room for the line between, with the largest numbers it could name.
    let length = text.bytes().len();
    let widest = [
        lines_marker(command, text.last_line(), text.last_line(), length),
        chars_marker(command, text.last_line(), length, length),
    ]
    .iter()
    .map(|marker| marker.chars().count() + 1)
    .max()
    .unwrap_or(0);
    let half = budget.saturating_sub(widest) / 2;
    let (head, hidden_start) = take_start(text, from, half);
    let (tail, hidden_end) = take_end(text, half);
    let marker = marker(text, command, hidden_start, hidden_end.max(hidden_start));
    head.into_iter().chain([marker]).chain(tail).collect()
}

/// The pieces from line `from` on that fit in `half` characters, whole, or
/// the start of the first when it alone is too long; and where the bytes
/// they leave out start.
fn take_start(text: &OutputText, from: u64, half: usize) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut used = 0;
    let mut end = text.line_offset(from);
    let mut whole_lines = 0;
    for piece in text.forward(from) {
        let shown = Shown::new(&piece);
        if used + shown.chars < half {
            used += shown.chars + 1;
            if let Some((offset, bytes)) = shown.line {
                whole_lines += 1;
                end = offset + bytes;
                if text.bytes().get(end) == Some(&b'\n') {
                    end += 1;
                }
            }
            lines.push(shown.text);
            continue;
        }
        if whole_lines == 0
            && let Some((offset, bytes)) = shown.line
        {
            let part: String = shown.text.chars().take(half.saturating_sub(1)).collect();
            end = offset + part.len().min(bytes);
            lines.push(part);
        }
        break;
    }
    (lines, end)
}

/// The pieces from the end that fit in `half` characters, whole, or the end
/// of the last when it alone is too long; and where the bytes they show
/// start.
fn take_end(text: &OutputText, half: usize) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut used = 0;
    let mut start = text.bytes().len();
    let mut whole_lines = 0;
    for piece in text.backward() {
        let shown = Shown::new(&piece);
        if used + shown.chars < half {
            used += shown.chars + 1;
            if let Some((offset, _)) = shown.line {
                whole_lines += 1;
                start = offset;
            }
            lines.push(shown.text);
            continue;
        }
        if whole_lines == 0
            && let Some((offset, bytes)) = shown.line
        {
            let skip = shown.chars.saturating_sub(half.saturating_sub(1));
            let part: String = shown.text.chars().skip(skip).collect();
            start = offset + bytes - part.len().min(bytes);
            lines.push(part);
        }
        break;
    }
    lines.reverse();
    (lines, start)
}

/// The line between the start and the end a result shows of an output,
/// for the bytes from `start` to `end` it leaves out: the lines they fall
/// in, or, within one line, its characters.
fn marker(text: &OutputText, command: &CommandId, start: usize, end: usize) -> String {
    let first = text.line_of(start);
    let last = text.line_of(end.saturating_sub(1).max(start));
    let whole = text.is_line_start(start) && text.is_line_start(end);
    if first != last || whole {
        return lines_marker(command, first, last, end - start);
    }
    // Up to the line's end when the bytes left out end with its newline.
    let through = if end > start && text.is_line_start(end) {
        text.column(end - 1)
    } else {
        text.column(end)
    };
    chars_marker(command, first, text.column(start) + 1, through)
}

fn lines_marker(command: &CommandId, first: u64, last: u64, bytes: usize) -> String {
    format!(
        "[... lines {first}-{last} not shown ({bytes} bytes); read them: demi shell output {command} --lines {first}-{last} ...]"
    )
}

fn chars_marker(command: &CommandId, line: u64, from: usize, to: usize) -> String {
    let part_end = to.min(from + PAGE_CHARS - 1);
    format!(
        "[... characters {from}-{to} of line {line} not shown; read them: demi shell output {command} --raw | sed -n {line}p | cut -c {from}-{part_end} ...]"
    )
}

/// The most media one result attaches.
///
/// Bounded because the vendors limit the media of one request.
const RESULT_MEDIA: usize = 20;

/// What a result has attached so far, against its bounds: at most
/// [`RESULT_MEDIA`] media, whose base64 takes at most half of the body the
/// model's requests may carry.
struct Budget {
    attached: usize,
    base64: u64,
    half_body: Option<u64>,
}

impl Budget {
    fn new(limits: RequestLimits) -> Self {
        Self {
            attached: 0,
            base64: 0,
            half_body: limits.body_bytes.map(|bytes| bytes / 2),
        }
    }

    /// Why one more medium may not be attached; none when it may.
    fn full(&self) -> Option<String> {
        (self.attached >= RESULT_MEDIA)
            .then(|| format!("a result shows at most {RESULT_MEDIA} media"))
    }

    /// Takes a medium whose base64 is `base64` bytes long, or says why it
    /// does not fit.
    fn take(&mut self, base64: u64) -> Result<(), String> {
        if let Some(half) = self.half_body
            && self.base64 + base64 > half
        {
            return Err(format!(
                "its base64 would take the result's media past {half} bytes, half of what this model's requests may carry"
            ));
        }
        self.attached += 1;
        self.base64 += base64;
        Ok(())
    }
}

/// What the end report of a command whose job viewed `media` attaches, as the
/// `shell` result would have.
///
/// It attaches the media, by number, while `model` reads them and they fit the
/// bounds of requests within `limits`, then the text of the lines about them;
/// nothing when it viewed none.
pub async fn report_media(
    media: &[CommandMedium],
    model: &Model,
    limits: RequestLimits,
) -> Vec<ResultPart> {
    let (mut parts, lines) = attached_media(media, None, model, limits).await;
    if !lines.is_empty() {
        parts.push(ResultPart::Text(lines.join("\n")));
    }
    parts
}

/// What a result that reports a command's end attaches.
///
/// - The media its job viewed, by number, each while the model reads it in a
///   tool result and it fits the result's bounds.
/// - The lines that tell of each one not attached or attached in another form
///   than it came, and of a binary stdout, which is never shown.
async fn attached_media(
    media: &[CommandMedium],
    binary: Option<&BinaryOutput>,
    model: &Model,
    limits: RequestLimits,
) -> (Vec<ResultPart>, Vec<String>) {
    let mut budget = Budget::new(limits);
    let mut parts = Vec::new();
    let mut lines = Vec::new();
    for medium in media {
        let (part, line) = viewed_medium(medium, model, limits.tool_results, &mut budget).await;
        parts.extend(part);
        lines.extend(line);
    }
    if let Some(binary) = binary {
        lines.push(format!(
            "[binary stdout, {} bytes: not shown; to look at an image, a video or a PDF, pipe it into demi file view; to keep it, redirect it to a file]",
            binary.info.total_bytes
        ));
    }
    (parts, lines)
}

/// What one medium the job viewed becomes: attached, as it came or fitted,
/// and its line: that it was shown, fitted or not shown, and its
/// `attachment_id` or why it has none. The original is the file the model
/// viewed, so a line needs no way to read it.
async fn viewed_medium(
    medium: &CommandMedium,
    model: &Model,
    kinds: ToolResultKinds,
    budget: &mut Budget,
) -> (Option<ResultPart>, Option<String>) {
    let number = medium.number;
    let media_type = medium.media_type.as_str();
    let entry = model_media_type_for(media_type);
    let kind = entry.map_or("medium", |entry| entry.kind.name());
    // The line says the medium was shown to the model and not to the user,
    // which the model would otherwise take it for, and names its attachment,
    // which a reply shows (`crates/plugin-file/command/src/files.rs`).
    let line = |viewing: Viewing| -> Option<String> {
        let told = match (viewing, &medium.attachment) {
            (Viewing::Whole, None) => return None,
            (Viewing::Whole, Some(_)) => "shown to you, not to the user".to_owned(),
            (Viewing::Fitted(how), None) => format!("shown to you as {how}"),
            (Viewing::Fitted(how), Some(_)) => format!("shown to you as {how}, not to the user"),
            (Viewing::Not(reason), _) => format!("not shown: {reason}"),
        };
        Some(match &medium.attachment {
            Some(Ok(id)) => format!("[{kind} {number}: {told}; attachment_id: {id}]"),
            Some(Err(reason)) => format!("[{kind} {number}: {told}; not an attachment: {reason}]"),
            None => format!("[{kind} {number}: {told}]"),
        })
    };
    let not_attached = |reason: &str| line(Viewing::Not(reason.to_owned()));
    let bytes = match &medium.bytes {
        Ok(bytes) => bytes,
        Err(reason) => return (None, not_attached(reason)),
    };
    let Some(entry) = entry else {
        return (None, not_attached(&format!("{media_type} is no medium a model reads")));
    };
    if !kinds.reads(model, media_type) {
        return (None, not_attached(&format!("the model does not read {media_type}")));
    }
    if let Some(reason) = budget.full() {
        return (None, not_attached(&format!("{reason}; view it in another call")));
    }
    let data = B64Bytes::new(bytes.clone());
    let whole = |data: B64Bytes| MediaBytes {
        data,
        media_type: media_type.to_owned(),
    };
    match entry.kind {
        ModelMediaKind::Video | ModelMediaKind::Document => {
            if bytes.len() as u64 > MEDIUM_CAP_BYTES {
                return (
                    None,
                    not_attached(&format!("over the {MEDIUM_CAP_BYTES}-byte cap")),
                );
            }
            if let Err(reason) = budget.take(data.base64_len()) {
                return (None, not_attached(&reason));
            }
            let part = if entry.kind == ModelMediaKind::Video {
                ResultPart::Video(whole(data))
            } else {
                ResultPart::Document {
                    bytes: whole(data),
                    file_name: medium.name.clone().unwrap_or_else(|| document_name(number)),
                }
            };
            (Some(part), line(Viewing::Whole))
        }
        ModelMediaKind::Image => {
            let fitted = match images::fit(data, media_type).await {
                Ok(fitted) => fitted,
                Err(unfit) => return (None, not_attached(&unfit.to_string())),
            };
            if let Err(reason) = budget.take(fitted.data.base64_len()) {
                return (None, not_attached(&reason));
            }
            let viewing = if fitted.reencoded {
                Viewing::Fitted(format!(
                    "{} of {} × {} px, fitted from {media_type} of {} × {} px",
                    fitted.media_type,
                    fitted.entered.0,
                    fitted.entered.1,
                    fitted.came.0,
                    fitted.came.1
                ))
            } else {
                Viewing::Whole
            };
            let image = ResultPart::Image(MediaBytes {
                data: fitted.data,
                media_type: fitted.media_type.to_owned(),
            });
            (Some(image), line(viewing))
        }
    }
}

/// How the model saw a medium: as it came, in the fitted form the text
/// says, or not, for the reason the text says.
enum Viewing {
    Whole,
    Fitted(String),
    Not(String),
}

/// The name a document viewed from stdin goes by: its kind and number, as
/// its line names it.
fn document_name(number: u32) -> String {
    format!("document-{number}.pdf")
}

/// Where the command is, as the user's view of the call shows it: a loss is
/// its own end, which the call's row tags as its notice does.
fn view_status(state: &CommandState) -> ShellViewStatus {
    match state {
        CommandState::Running { .. } => ShellViewStatus::Running,
        CommandState::Exited { .. } => ShellViewStatus::Exited,
        CommandState::Aborted => ShellViewStatus::Aborted,
        CommandState::Lost { .. } => ShellViewStatus::Lost,
    }
}

/// The view the user sees: the status, the end of the output the model had
/// not seen, `text`, by stream, and once the command exited the files it
/// changed.
fn shell_view(status: &CommandStatus, text: &OutputText) -> ShellToolView {
    let (chunks, cut) = match &status.whole {
        Some(_) => {
            let chunks = text
                .unseen()
                .map_or_else(Vec::new, |from| text.chunks(from));
            tail_window(&chunks, VIEW_CHARS)
        }
        None => {
            let (chunks, cut) = tail_window(&status.output.chunks, VIEW_CHARS);
            (chunks, cut || status.output.truncated)
        }
    };
    ShellToolView {
        request_files: None,
        run_files: None,
        status: view_status(&status.state),
        command_id: status.command_id.clone(),
        exit_code: match status.state {
            CommandState::Exited { exit_code, .. } => Some(exit_code),
            CommandState::Running { .. } | CommandState::Aborted | CommandState::Lost { .. } => None,
        },
        lost_reason: match &status.state {
            CommandState::Lost { reason, .. } => Some(reason.clone()),
            _ => None,
        },
        running_ms: status.running_ms,
        idle_ms: status.idle_ms,
        chunks,
        view_truncated: cut,
        files: status.files.as_ref().map(|files| files.files.clone()),
        files_truncated: status.files.as_ref().map(|files| files.truncated),
        path_changes: status
            .files
            .as_ref()
            .map_or_else(Vec::new, |files| files.path_changes.clone()),
    }
}

/// The last `max` characters of `chunks`, each kept chunk tagged with its
/// stream, and whether anything was left out. Counting from the end keeps
/// the newest output.
fn tail_window(chunks: &[OutputChunk], max: usize) -> (Vec<OutputChunk>, bool) {
    let mut kept = Vec::new();
    let mut total = 0;
    for chunk in chunks.iter().rev() {
        if chunk.text.is_empty() {
            continue;
        }
        let remaining = max - total;
        if remaining == 0 {
            kept.reverse();
            return (kept, true);
        }
        let length = chunk.text.chars().count();
        if length <= remaining {
            kept.push(chunk.clone());
            total += length;
            continue;
        }
        let (start, _) = chunk
            .text
            .char_indices()
            .nth(length - remaining)
            .expect("the chunk is longer than what remains");
        kept.push(OutputChunk {
            stream: chunk.stream,
            text: chunk.text[start..].to_owned(),
        });
        kept.reverse();
        return (kept, true);
    }
    kept.reverse();
    (kept, false)
}
