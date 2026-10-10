//! A shell tool's result (`runtime.md` § Results and previews): the text the
//! model reads, the media it attaches, the command's returned media and its
//! binary stdout (`runtime.md` § What a result attaches), and the bounded
//! view the user sees, all from one command status.

use std::sync::Arc;

use demi_agent_session::ToolOutcome;
use demi_agent_store::images;
use demi_agent_transcript::REPLAY_CHARS;
use demi_command_protocol::sniff_media_type;
use demi_host_interface::{
    BinaryOutput, CommandMedium, CommandState, CommandStatus, Newest, OutputText, Piece, Seen,
    Streams, WholeOutput, WholeView,
};
use demi_provider_common::{MediaBytes, RequestLimits, ResultPart};
use demi_shared_types::{
    B64Bytes, CommandId, Model, ModelMediaKind, OutputChunk, OutputView, ShellToolView,
    ShellViewStatus, StreamView, ToolView, model_accepts_media_type, model_media_type_for,
};

use super::PAGE_CHARS;

/// The characters of merged output a shell view keeps, from the end.
pub(super) const VIEW_CHARS: usize = 32_768;
/// The largest video: about ten minutes at a viewing-grade encoding, under
/// the inline payload ceiling the major APIs enforce. An image is fitted
/// instead (`runtime.md` § Images in the transcript).
const VIDEO_CAP_BYTES: u64 = 16 * 1024 * 1024;
/// How many of a running command's newest lines its result points to.
const NEWEST_LINES: u64 = 50;

/// What a look at a command says beside its status and output.
#[derive(Debug, Clone, Copy, Default)]
pub struct Look<'a> {
    /// The line it starts with, such as the one that says the interval
    /// asked for was outside the bounds (`runtime.md` § Tool input).
    pub note: Option<&'a str>,
    /// The user's send now ended the call's window (`runtime.md` § Send
    /// now).
    pub sent_now: bool,
    /// How the command reports to its node while it runs: every so many
    /// milliseconds, or only its end when none; not known when the outer
    /// option is none, as for a look at another agent's command.
    pub interval_ms: Option<Option<u32>>,
    /// The look is a progress report, which says its next step its own way
    /// in place of the generic one.
    pub report: bool,
}

/// The `shell` call's outcome for `status`: its text, the media it attaches
/// and the lines about them, and its view. `model` is the call's, and its
/// vendor takes requests within `limits`. A command that exited with a
/// status other than 0, or was stopped, is an error to the provider
/// (`runtime.md` § Results and previews).
pub(super) async fn shell_outcome(
    status: &CommandStatus,
    model: &Model,
    limits: RequestLimits,
    look: Look<'_>,
) -> ToolOutcome {
    let text = unseen_output(status);
    let mut output = vec![ResultPart::Text(result_text(status, &text, look))];
    if let CommandState::Exited {
        binary_stdout,
        media,
        ..
    } = &status.state
    {
        let (parts, lines) = attached_media(
            media,
            binary_stdout.as_ref(),
            &status.command_id,
            model,
            limits,
        )
        .await;
        output.extend(parts);
        if !lines.is_empty() {
            output.push(ResultPart::Text(lines.join("\n")));
        }
    }
    let is_error = match status.state {
        CommandState::Running { .. } => false,
        CommandState::Exited { exit_code, .. } => exit_code != 0,
        CommandState::Aborted => true,
    };
    ToolOutcome {
        output,
        is_error,
        view: Some(ToolView::Shell(shell_view(status, &text))),
        effect: None,
    }
}

/// What a look at a command shows the model, as a result shows it: the text
/// of `demi shell status`, and of a command's progress report.
pub fn look_text(status: &CommandStatus, look: Look<'_>) -> String {
    result_text(status, &unseen_output(status), look)
}

/// What a look at `command` shows the model from its whole output `whole`,
/// from the node's own place `seen` in it: the look of a node that does not
/// hold the command, such as at another agent's command, or at one whose
/// shells ended (`runtime.md` § The `demi shell` commands). The command is
/// in `state`, has run `running_ms` and printed nothing for `idle_ms`.
pub fn whole_look_text(
    command: &CommandId,
    state: CommandState,
    running_ms: u64,
    idle_ms: u64,
    whole: Arc<WholeOutput>,
    seen: Seen,
    look: Look<'_>,
) -> String {
    let stream = || StreamView {
        offset: 0,
        delta: String::new(),
        tail: String::new(),
        bytes: 0,
        truncated: false,
    };
    let status = CommandStatus {
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
    };
    look_text(&status, look)
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

/// The lines the model reads: first the look's note, then the status, the
/// handles and timings that matter, the output since the model's last look
/// within the replay bound, while the command runs each stream's newest
/// lines beyond its start, and the next step, after the line that says the
/// command moved to the background when the user's send now ended the
/// window (`runtime.md` § Send now).
fn result_text(status: &CommandStatus, text: &OutputText, look: Look<'_>) -> String {
    let command = &status.command_id;
    let running = matches!(status.state, CommandState::Running { .. });
    let mut before: Vec<String> = look.note.into_iter().map(str::to_owned).collect();
    before.push(format!("status: {}", view_status(&status.state)));
    if let CommandState::Exited { exit_code, .. } = status.state {
        before.push(format!("exitCode: {exit_code}"));
    }
    before.push(format!("commandId: {command}"));
    if running {
        before.push(format!("runningMs: {}", status.running_ms));
        before.push(format!("idleMs: {}", status.idle_ms));
    }
    let newest: Vec<&Newest> = status
        .newest
        .iter()
        .filter(|newest| running && !newest.text.is_empty())
        .collect();
    let mut after = Vec::new();
    if running && status.unreceived > 0 && newest.is_empty() {
        after.push(format!(
            "[... {} bytes not shown so far; the newest: demi shell output {command} --tail {NEWEST_LINES} ...]",
            status.unreceived
        ));
    }
    match &status.state {
        CommandState::Running { hint } => {
            if look.sent_now {
                after.push(format!(
                    "[The user sent a message, so command {command} moved to the background. It keeps running.]"
                ));
            }
            match hint {
                Some(hint) => after.push(hint.clone()),
                None if look.report => {}
                None => after.push(running_next(command, look.interval_ms)),
            }
        }
        CommandState::Aborted => after.push("next: command was intentionally stopped.".to_owned()),
        CommandState::Exited { .. } => {}
    }
    // The other lines, the output's label among them, each with its newline.
    let markers: Vec<String> = newest.iter().map(|newest| newest_marker(newest)).collect();
    let others: usize = before
        .iter()
        .chain(&after)
        .chain(&markers)
        .map(|line| line.chars().count() + 1)
        .sum::<usize>()
        + "output:\n".len();
    let budget = REPLAY_CHARS.saturating_sub(others);
    // The start takes all of the bound, or half of it beside newest lines.
    let start_budget = if newest.is_empty() {
        budget
    } else {
        budget / 2
    };
    let shown = text
        .unseen_line()
        .map_or_else(Vec::new, |from| cut(text, from, command, start_budget));
    let used: usize = shown.iter().map(|line| line.chars().count() + 1).sum();
    let mut newest_lines = Vec::new();
    if !newest.is_empty() {
        let each = budget.saturating_sub(used) / newest.len();
        for (newest, marker) in newest.iter().zip(markers) {
            newest_lines.push(marker);
            newest_lines.extend(newest_tail(newest, each));
        }
    }
    let mut lines = before;
    if shown.is_empty() && newest_lines.is_empty() {
        lines.push("output: (empty)".to_owned());
    } else {
        lines.push("output:".to_owned());
        lines.extend(shown);
        lines.extend(newest_lines);
    }
    lines.extend(after);
    lines.join("\n")
}

/// The next step for a command that runs: it reports to the node as its
/// interval says, and the `demi shell` commands look at it, answer it and
/// stop it (`runtime.md` § Results and previews).
fn running_next(command: &CommandId, interval_ms: Option<Option<u32>>) -> String {
    let reports = match interval_ms {
        Some(Some(interval)) => format!(
            " and reports to you every {} until it ends",
            super::duration(interval.into())
        ),
        Some(None) => " and reports to you when it ends".to_owned(),
        None => String::new(),
    };
    format!(
        "next: command {command} keeps running{reports}; look at it with demi shell status {command}, answer a prompt with demi shell input {command}, stop it with demi shell stop {command}."
    )
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
/// with its newline: all of them when they fit; otherwise as many whole lines
/// from the start and from the end as fit in half each of what the budget
/// leaves after the line between them, which names what it leaves out and
/// the command that prints it. A single line too long for its half shows in
/// part.
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

/// The most media one result attaches (`runtime.md` § What a result
/// attaches).
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
            .then(|| format!("a result attaches at most {RESULT_MEDIA} media"))
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

/// What a result that reports a command's end attaches (`runtime.md`
/// § What a result attaches): the media its declared commands returned, by
/// number, then a binary stdout, each while the model accepts it and it
/// fits the result's bounds; and the lines that tell of each one not
/// attached or attached in another form than it came, and of the binary
/// stdout.
async fn attached_media(
    media: &[CommandMedium],
    binary: Option<&BinaryOutput>,
    command: &CommandId,
    model: &Model,
    limits: RequestLimits,
) -> (Vec<ResultPart>, Vec<String>) {
    let mut budget = Budget::new(limits);
    let mut parts = Vec::new();
    let mut lines = Vec::new();
    for medium in media {
        let (part, line) = returned_medium(medium, command, model, &mut budget).await;
        parts.extend(part);
        lines.extend(line);
    }
    if let Some(binary) = binary {
        let (part, line) = binary_verdict(binary, command, model, &mut budget).await;
        parts.extend(part);
        lines.push(line);
    }
    (parts, lines)
}

/// What one returned medium becomes: attached, as it came or fitted, and
/// its line when it was not attached or was fitted.
async fn returned_medium(
    medium: &CommandMedium,
    command: &CommandId,
    model: &Model,
    budget: &mut Budget,
) -> (Option<ResultPart>, Option<String>) {
    let number = medium.number;
    let media_type = medium.media_type.as_str();
    let not_attached = |reason: &str| {
        Some(format!(
            "[medium {number}: not attached: {reason}; save it: demi shell output {command} --medium {number} > <file>]"
        ))
    };
    let bytes = match &medium.bytes {
        Ok(bytes) => bytes,
        Err(reason) => return (None, Some(format!("[medium {number}: not attached: {reason}]"))),
    };
    let Some(entry) = model_media_type_for(media_type) else {
        return (None, not_attached(&format!("{media_type} is no medium a model reads")));
    };
    if !model_accepts_media_type(model, media_type) {
        return (None, not_attached(&format!("the model does not accept {media_type}")));
    }
    if let Some(reason) = budget.full() {
        return (None, not_attached(&reason));
    }
    let data = B64Bytes::new(bytes.clone());
    if entry.kind == ModelMediaKind::Video {
        if let Err(reason) = budget.take(data.base64_len()) {
            return (None, not_attached(&reason));
        }
        let video = ResultPart::Video(MediaBytes {
            data,
            media_type: media_type.to_owned(),
        });
        return (Some(video), None);
    }
    let fitted = match images::fit(data, media_type).await {
        Ok(fitted) => fitted,
        Err(unfit) => return (None, not_attached(&unfit.to_string())),
    };
    if let Err(reason) = budget.take(fitted.data.base64_len()) {
        return (None, not_attached(&reason));
    }
    let line = fitted.reencoded.then(|| {
        format!(
            "[medium {number}: attached as {} of {} × {} px, fitted from {media_type} of {} × {} px; the original: demi shell output {command} --medium {number} > <file>]",
            fitted.media_type,
            fitted.entered.0,
            fitted.entered.1,
            fitted.came.0,
            fitted.came.1
        )
    });
    let image = ResultPart::Image(MediaBytes {
        data: fitted.data,
        media_type: fitted.media_type.to_owned(),
    });
    (Some(image), line)
}

/// What a binary final stdout becomes (`runtime.md` § What a result
/// attaches): attached as an image or a video only when the output kept all
/// of it, its bytes are a type of the model-media table, the model accepts
/// that type and it fits the result's bounds; an image as it is fitted, and
/// a video within its cap. Otherwise a note says why not and how to save
/// the bytes.
async fn binary_verdict(
    binary: &BinaryOutput,
    command: &CommandId,
    model: &Model,
    budget: &mut Budget,
) -> (Option<ResultPart>, String) {
    let media = sniff_media_type(&binary.bytes).and_then(model_media_type_for);
    let total = binary.info.total_bytes;
    let save = format!("save it: demi shell output {command} --raw --stdout > <file>");
    if binary.info.truncated {
        return (
            None,
            format!(
                "Binary stdout ({total} bytes) is more than the {} bytes a command's output keeps whole, so it was not attached and is kept whole nowhere; write it to a file instead and run the command again.",
                binary.info.limit_bytes
            ),
        );
    }
    // Bytes of no medium are most often a medium a pipe after the command
    // cut short, as `demi browser screenshot t1 | tail -3` does, so the
    // line says so first.
    let Some(media) = media else {
        return (
            None,
            format!(
                "[binary stdout, {total} bytes: not an image or video; a pipe after the command may have cut it — run the command without it; {save}]"
            ),
        );
    };
    if !model_accepts_media_type(model, media.media_type) {
        return (
            None,
            format!(
                "Binary stdout is {}, which this model does not accept natively; {save}.",
                media.media_type
            ),
        );
    }
    let not_attached = |reason: &str| {
        format!(
            "Binary stdout is {} ({total} bytes), which was not attached because {reason}; {save}.",
            media.media_type
        )
    };
    if let Some(reason) = budget.full() {
        return (None, not_attached(&reason));
    }
    let data = B64Bytes::new(binary.bytes.clone());
    if media.kind == ModelMediaKind::Image {
        let fitted = match images::fit(data, media.media_type).await {
            Ok(fitted) => fitted,
            Err(unfit) => return (None, not_attached(&unfit.to_string())),
        };
        if let Err(reason) = budget.take(fitted.data.base64_len()) {
            return (None, not_attached(&reason));
        }
        let note = if fitted.reencoded {
            format!(
                "Attached stdout, {} of {}x{} px ({total} bytes), as {} of {}x{} px ({} bytes), fitted to what every model accepts; to keep the original, {save}.",
                media.media_type,
                fitted.came.0,
                fitted.came.1,
                fitted.media_type,
                fitted.entered.0,
                fitted.entered.1,
                fitted.data.len()
            )
        } else {
            format!("Attached stdout as {} ({total} bytes).", media.media_type)
        };
        let image = ResultPart::Image(MediaBytes {
            data: fitted.data,
            media_type: fitted.media_type.to_owned(),
        });
        return (Some(image), note);
    }
    if total > VIDEO_CAP_BYTES {
        return (
            None,
            format!(
                "Binary stdout is {} ({total} bytes), over the {VIDEO_CAP_BYTES}-byte video cap, so it was not attached; {save}, or produce a smaller version, with fewer frames or a lower resolution, and run it again.",
                media.media_type
            ),
        );
    }
    if let Err(reason) = budget.take(data.base64_len()) {
        return (
            None,
            format!(
                "Binary stdout is {} ({total} bytes), which was not attached because {reason}; {save}, or produce a smaller version, with fewer frames or a lower resolution, and run it again.",
                media.media_type
            ),
        );
    }
    let video = ResultPart::Video(MediaBytes {
        data,
        media_type: media.media_type.to_owned(),
    });
    (
        Some(video),
        format!("Attached stdout as {} ({total} bytes).", media.media_type),
    )
}

fn view_status(state: &CommandState) -> ShellViewStatus {
    match state {
        CommandState::Running { .. } => ShellViewStatus::Running,
        CommandState::Exited { .. } => ShellViewStatus::Exited,
        CommandState::Aborted => ShellViewStatus::Aborted,
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
        status: view_status(&status.state),
        command_id: status.command_id.clone(),
        exit_code: match status.state {
            CommandState::Exited { exit_code, .. } => Some(exit_code),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use demi_host_interface::{Newest, OutputRecord, Seen, WholeOutput, WholeView};
    use demi_shared_types::{BinaryStdout, FileExtension, OutputView, StreamKind, StreamView};

    use super::*;
    use demi_agent_store::testing::test_model;

    fn stream() -> StreamView {
        StreamView {
            offset: 0,
            delta: String::new(),
            tail: String::new(),
            bytes: 0,
            truncated: false,
        }
    }

    /// Command 17, which exited printing `text`, which the model had not
    /// seen.
    fn exited(text: &str) -> CommandStatus {
        let output = WholeOutput::new(
            vec![OutputRecord::Output(
                StreamKind::Stdout,
                Bytes::from(text.to_owned()),
            )],
            None,
        );
        CommandStatus {
            command_id: "17".try_into().unwrap(),
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
                output: Arc::new(output),
                seen: Seen::default(),
            }),
            running_ms: 5,
            idle_ms: 1,
            state: CommandState::Exited {
                exit_code: 1,
                binary_stdout: None,
                media: Vec::new(),
            },
            files: None,
        }
    }

    fn text_of(outcome: &ToolOutcome) -> Vec<&str> {
        outcome
            .output
            .iter()
            .map(|part| match part {
                ResultPart::Text(text) => text.as_str(),
                ResultPart::Image(_) => "<image>",
                ResultPart::Video(_) => "<video>",
            })
            .collect()
    }

    /// The next step of a running command 17 that reports every five
    /// minutes.
    const NEXT: &str = "next: command 17 keeps running and reports to you every 5m until it ends; look at it with demi shell status 17, answer a prompt with demi shell input 17, stop it with demi shell stop 17.";

    async fn result(status: &CommandStatus) -> String {
        let look = Look {
            interval_ms: Some(Some(300_000)),
            ..Look::default()
        };
        let outcome = shell_outcome(status, &test_model().model, RequestLimits::default(), look).await;
        text_of(&outcome)[0].to_owned()
    }

    #[tokio::test]
    async fn an_output_within_the_bound_shows_whole_with_the_command_to_read_it() {
        assert_eq!(
            result(&exited("one\ntwo\n")).await,
            "status: exited\nexitCode: 1\ncommandId: 17\noutput:\none\ntwo"
        );
    }

    #[tokio::test]
    async fn a_look_repeats_an_unfinished_line_until_its_newline_or_the_command_ends() {
        use demi_host_interface::{CommandRecord, Ending};

        let mut record = CommandRecord::new(
            "17".try_into().unwrap(),
            "call".into(),
        );
        record.append_output(StreamKind::Stdout, "done\nre");
        record.append_output(StreamKind::Stderr, "a");
        for expected in ["done\nrea", "rea"] {
            let shown = result(&record.status(0, None)).await;
            assert!(
                shown.ends_with(&format!("\noutput:\n{expected}\n{NEXT}")),
                "{shown}"
            );
        }
        record.append_output(StreamKind::Stdout, "dy\nprompt");
        for expected in ["ready\nprompt", "prompt"] {
            let shown = result(&record.status(0, None)).await;
            assert!(
                shown.ends_with(&format!("\noutput:\n{expected}\n{NEXT}")),
                "{shown}"
            );
        }
        let whole = WholeOutput::new(
            vec![
                OutputRecord::Output(StreamKind::Stdout, Bytes::from_static(b"done\nre")),
                OutputRecord::Output(StreamKind::Stderr, Bytes::from_static(b"a")),
                OutputRecord::Output(StreamKind::Stdout, Bytes::from_static(b"dy\nprompt")),
            ],
            None,
        );
        record.settle(Ending::Exited(0), Arc::new(whole), None, Vec::new(), "");
        let final_look = result(&record.status(0, None)).await;
        assert!(final_look.ends_with("\noutput:\nprompt"), "{final_look}");
        let read_again = result(&record.status(0, None)).await;
        assert!(read_again.ends_with("\noutput: (empty)"), "{read_again}");
    }

    /// `seq 1 5000`: the result shows whole lines from both ends within the
    /// replay bound, and the line between names the lines it leaves out,
    /// their bytes, and the command that prints exactly them.
    #[tokio::test]
    async fn a_long_output_shows_its_first_and_last_lines_and_names_the_rest() {
        let numbers: String = (1..=5000).map(|number| format!("{number}\n")).collect();
        let text = result(&exited(&numbers)).await;
        assert!(text.chars().count() <= REPLAY_CHARS, "{}", text.len());
        let lines: Vec<&str> = text.lines().collect();
        let marker = lines
            .iter()
            .position(|line| line.starts_with("[... lines "))
            .expect("a marker");
        let before: u64 = lines[marker - 1].parse().unwrap();
        let after: u64 = lines[marker + 1].parse().unwrap();
        assert_eq!(lines[4], "1");
        assert_eq!(*lines.last().unwrap(), "5000");
        let (first, last) = (before + 1, after - 1);
        let bytes: usize = (first..=last)
            .map(|number| format!("{number}\n").len())
            .sum();
        assert_eq!(
            lines[marker],
            format!(
                "[... lines {first}-{last} not shown ({bytes} bytes); read them: demi shell output 17 --lines {first}-{last} ...]"
            )
        );
        // Each half is within a line of the other.
        assert!(before.abs_diff(5000 - after) < 1000, "{before} {after}");
    }

    /// One line longer than the bound, such as minified JSON, shows its
    /// start and its end, and the line between names its characters left
    /// out and a command that prints them a page at a time.
    #[tokio::test]
    async fn a_single_long_line_shows_in_part_with_its_characters_named() {
        let line = "x".repeat(40_000);
        let text = result(&exited(&line)).await;
        assert!(text.chars().count() <= REPLAY_CHARS);
        let lines: Vec<&str> = text.lines().collect();
        let (head, marker, tail) = (lines[4], lines[5], lines[6]);
        let from = head.len() + 1;
        let to = 40_000 - tail.len();
        assert_eq!(
            marker,
            format!(
                "[... characters {from}-{to} of line 1 not shown; read them: demi shell output 17 --raw | sed -n 1p | cut -c {from}-{} ...]",
                from + PAGE_CHARS - 1
            )
        );
    }

    #[tokio::test]
    async fn a_running_command_shows_its_first_and_its_newest_lines_within_the_bound() {
        let mut running = exited("");
        running.whole = None;
        running.state = CommandState::Running { hint: None };
        running.output.text = "building\n".into();
        running.unreceived = 1_048_576;
        running.newest = vec![Newest {
            stream: StreamKind::Stdout,
            offset: 1_048_576,
            left_out: 1_040_384,
            text: "ne 998\nline 999\nline 1000\n".into(),
        }];
        assert_eq!(
            result(&running).await,
            [
                "status: running",
                "commandId: 17",
                "runningMs: 5",
                "idleMs: 1",
                "output:",
                "building",
                "[... 1040384 bytes of stdout not shown; its newest lines follow ...]",
                "line 999",
                "line 1000",
                NEXT,
            ]
            .join("\n")
        );

        // A long start and long newest bytes take half of the bound each.
        running.output.text = "a\n".repeat(8_000);
        running.newest[0].text = "b\n".repeat(8_000);
        let text = result(&running).await;
        assert!(
            text.chars().count() <= REPLAY_CHARS,
            "{}",
            text.chars().count()
        );
        let first = text.lines().filter(|line| *line == "a").count();
        let newest = text.lines().filter(|line| *line == "b").count();
        assert!(first > 3_000 && newest > 3_000, "{first} {newest}");
        assert!(text.contains("bytes of stdout not shown; its newest lines follow"));
    }

    #[tokio::test]
    async fn a_running_command_names_its_handles_and_the_newest_output_it_has_not_received() {
        let mut running = exited("");
        running.whole = None;
        running.state = CommandState::Running { hint: None };
        running.output.text = "building\n".into();
        running.unreceived = 1_048_576;
        assert_eq!(
            result(&running).await,
            [
                "status: running",
                "commandId: 17",
                "runningMs: 5",
                "idleMs: 1",
                "output:",
                "building",
                "[... 1048576 bytes not shown so far; the newest: demi shell output 17 --tail 50 ...]",
                NEXT,
            ]
            .join("\n")
        );
        running.state = CommandState::Running {
            hint: Some("waiting for input: answer with demi shell input".into()),
        };
        assert!(
            result(&running)
                .await
                .ends_with("\nwaiting for input: answer with demi shell input")
        );
        let mut aborted = exited("");
        aborted.state = CommandState::Aborted;
        assert_eq!(
            result(&aborted).await,
            "status: aborted\ncommandId: 17\noutput: (empty)\nnext: command was intentionally stopped."
        );
    }

    /// The notes, and a placeholder for each medium, of the result of a
    /// command whose binary stdout is `bytes`, `total` bytes in all.
    async fn verdict(
        model: &Model,
        limits: RequestLimits,
        bytes: &Bytes,
        truncated: bool,
        total: u64,
    ) -> String {
        let mut status = exited("");
        status.state = CommandState::Exited {
            exit_code: 0,
            binary_stdout: Some(BinaryOutput {
                bytes: bytes.clone(),
                info: BinaryStdout {
                    truncated,
                    total_bytes: total,
                    limit_bytes: 16 * 1024 * 1024,
                },
            }),
            media: Vec::new(),
        };
        let outcome = shell_outcome(&status, model, limits, Look::default()).await;
        text_of(&outcome)[1..].join(" | ")
    }

    #[tokio::test]
    async fn a_binary_stdout_is_attached_only_when_whole_known_accepted_and_small_enough() {
        let png = demi_agent_store::testing::png(4, 3, 1).into_bytes();
        let wide = demi_agent_store::testing::png(2_400, 10, 1).into_bytes();
        let broken = Bytes::from_static(b"\x89PNG\r\n\x1a\n\x00\xff\xfe\x01");
        let mp4 = Bytes::from_static(b"\0\0\0\x20ftypisom\xff\xfe");
        let opaque = Bytes::from_static(b"\xde\xad\xbe\xef\xff\xfe\0\x01\x02\x03\x04\x05");
        let mut model = test_model().model;
        model.accepted_extensions = Some(vec![FileExtension::Png]);
        let mut video_model = test_model().model;
        video_model.accepted_extensions = Some(vec![FileExtension::Mp4]);
        let limits = RequestLimits::default();
        let save = "save it: demi shell output 17 --raw --stdout > <file>";
        let size = |bytes: &Bytes| bytes.len() as u64;
        assert_eq!(
            verdict(&model, limits, &png, false, size(&png)).await,
            format!(
                "<image> | Attached stdout as image/png ({} bytes).",
                png.len()
            )
        );
        // An image enters fitted, and the note says from what.
        let fitted = verdict(&model, limits, &wide, false, size(&wide)).await;
        let prefix = format!(
            "<image> | Attached stdout, image/png of 2400x10 px ({} bytes), as image/png of 2000x8 px (",
            wide.len()
        );
        assert!(fitted.starts_with(&prefix), "{fitted}");
        assert!(
            fitted.ends_with(&format!(
                " bytes), fitted to what every model accepts; to keep the original, {save}."
            )),
            "{fitted}"
        );
        let unread = verdict(&model, limits, &broken, false, 12).await;
        assert!(
            unread.starts_with("Binary stdout is image/png (12 bytes), which was not attached because it could not be decoded"),
            "{unread}"
        );
        assert!(unread.ends_with(&format!("; {save}.")), "{unread}");
        assert_eq!(
            verdict(&model, limits, &mp4, false, 14).await,
            format!(
                "Binary stdout is video/mp4, which this model does not accept natively; {save}."
            )
        );
        // A video within the cap is attached while its base64 takes at most
        // half of the body limit: its 14 bytes are 20 as base64.
        let body = |bytes| RequestLimits {
            body_bytes: Some(bytes),
            images: None,
        };
        assert_eq!(
            verdict(&video_model, body(40), &mp4, false, 14).await,
            "<video> | Attached stdout as video/mp4 (14 bytes)."
        );
        assert_eq!(
            verdict(&video_model, body(39), &mp4, false, 14).await,
            format!(
                "Binary stdout is video/mp4 (14 bytes), which was not attached because its base64 would take the result's media past 19 bytes, half of what this model's requests may carry; {save}, or produce a smaller version, with fewer frames or a lower resolution, and run it again."
            )
        );
        assert_eq!(
            verdict(&video_model, limits, &mp4, false, 17 * 1024 * 1024).await,
            format!(
                "Binary stdout is video/mp4 (17825792 bytes), over the 16777216-byte video cap, so it was not attached; {save}, or produce a smaller version, with fewer frames or a lower resolution, and run it again."
            )
        );
        assert_eq!(
            verdict(&model, limits, &opaque, false, 12).await,
            format!(
                "[binary stdout, 12 bytes: not an image or video; a pipe after the command may have cut it — run the command without it; {save}]"
            )
        );
        assert_eq!(
            verdict(&model, limits, &png, true, 20_000_000).await,
            "Binary stdout (20000000 bytes) is more than the 16777216 bytes a command's output keeps whole, so it was not attached and is kept whole nowhere; write it to a file instead and run the command again."
        );
    }

    /// The test model, reading PNG images.
    fn png_model() -> Model {
        let mut model = test_model().model;
        model.accepted_extensions = Some(vec![FileExtension::Png]);
        model
    }

    /// Medium `number` of command 17, of `media_type`, with `bytes`.
    fn returned(number: u32, media_type: &str, bytes: &Bytes) -> CommandMedium {
        CommandMedium {
            number,
            media_type: media_type.to_owned(),
            size: bytes.len() as u64,
            bytes: Ok(bytes.clone()),
        }
    }

    /// What the result of command 17 attaches and says after its output,
    /// one placeholder per attached medium, when its commands returned
    /// `media` and its stdout is `binary`.
    async fn attached(
        media: Vec<CommandMedium>,
        binary: Option<&Bytes>,
        model: &Model,
        limits: RequestLimits,
    ) -> Vec<String> {
        let mut status = exited("");
        status.state = CommandState::Exited {
            exit_code: 0,
            binary_stdout: binary.map(|bytes| BinaryOutput {
                bytes: bytes.clone(),
                info: BinaryStdout {
                    truncated: false,
                    total_bytes: bytes.len() as u64,
                    limit_bytes: 16 * 1024 * 1024,
                },
            }),
            media,
        };
        let outcome = shell_outcome(&status, model, limits, Look::default()).await;
        text_of(&outcome)[1..]
            .iter()
            .flat_map(|part| part.lines())
            .map(str::to_owned)
            .collect()
    }

    /// Planted defects this catches: a count bound that is not 20, or one
    /// that the binary stdout escapes; a binary stdout attached before the
    /// returned media; and lines out of the media's order.
    #[tokio::test]
    async fn a_result_attaches_at_most_20_media_in_order_then_names_the_others() {
        let png = demi_agent_store::testing::png(4, 3, 1).into_bytes();
        let media = (1..=25)
            .map(|number| returned(number, "image/png", &png))
            .collect();
        let shown = attached(media, Some(&png), &png_model(), RequestLimits::default()).await;
        assert!(shown[..20].iter().all(|part| part == "<image>"), "{shown:?}");
        let mut lines: Vec<String> = (21..=25)
            .map(|number| format!("[medium {number}: not attached: a result attaches at most 20 media; save it: demi shell output 17 --medium {number} > <file>]"))
            .collect();
        lines.push(format!("Binary stdout is image/png ({} bytes), which was not attached because a result attaches at most 20 media; save it: demi shell output 17 --raw --stdout > <file>.", png.len()));
        assert_eq!(shown[20..], lines);
    }

    /// Planted defects this catches: a budget that counts each medium alone
    /// rather than all of the result's (the fourth would be attached), and
    /// one that stops at the first medium that breaks it rather than
    /// skipping it (the third would not).
    #[tokio::test]
    async fn a_medium_past_half_the_body_limit_is_skipped_and_a_later_smaller_one_attached() {
        let small = demi_agent_store::testing::png(4, 3, 1);
        let large = demi_agent_store::testing::png(64, 64, 2);
        let half = 2 * small.base64_len();
        let limits = RequestLimits {
            body_bytes: Some(2 * half),
            images: None,
        };
        let (small, large) = (small.into_bytes(), large.into_bytes());
        let media = vec![
            returned(1, "image/png", &small),
            returned(2, "image/png", &large),
            returned(3, "image/png", &small),
            returned(4, "image/png", &small),
        ];
        let shown = attached(media, None, &png_model(), limits).await;
        let past = |number: u32| {
            format!("[medium {number}: not attached: its base64 would take the result's media past {half} bytes, half of what this model's requests may carry; save it: demi shell output 17 --medium {number} > <file>]")
        };
        assert_eq!(
            shown,
            ["<image>".to_owned(), "<image>".to_owned(), past(2), past(4)]
        );
    }

    /// Planted defects this catches: a medium attached in a type the model
    /// does not read, a fitted image attached without its line, and a
    /// medium the backend does not have given a way to read it.
    #[tokio::test]
    async fn each_medium_not_attached_or_fitted_has_its_line() {
        let wide = demi_agent_store::testing::png(2_400, 10, 1).into_bytes();
        let webp = Bytes::from_static(b"RIFFWEBPVP8 ");
        let mut model = test_model().model;
        model.accepted_extensions = Some(vec![FileExtension::Png]);
        let lost = CommandMedium {
            number: 3,
            media_type: "image/png".into(),
            size: 412_000,
            bytes: Err("lost with the Host's connection".into()),
        };
        let media = vec![
            returned(1, "image/png", &wide),
            returned(2, "image/webp", &webp),
            lost,
        ];
        let shown = attached(media, None, &model, RequestLimits::default()).await;
        assert_eq!(
            shown,
            [
                "<image>",
                "[medium 1: attached as image/png of 2000 × 8 px, fitted from image/png of 2400 × 10 px; the original: demi shell output 17 --medium 1 > <file>]",
                "[medium 2: not attached: the model does not accept image/webp; save it: demi shell output 17 --medium 2 > <file>]",
                "[medium 3: not attached: lost with the Host's connection]",
            ]
        );
    }

    #[test]
    fn the_view_window_keeps_the_newest_characters_of_the_merged_output() {
        let chunk = |stream, text: &str| OutputChunk {
            stream,
            text: text.to_owned(),
        };
        let chunks = [
            chunk(StreamKind::Stdout, "ab"),
            chunk(StreamKind::Stderr, ""),
            chunk(StreamKind::Stderr, "cdé"),
        ];
        assert_eq!(
            tail_window(&chunks, 10),
            (
                chunks[..]
                    .iter()
                    .filter(|chunk| !chunk.text.is_empty())
                    .cloned()
                    .collect(),
                false
            )
        );
        assert_eq!(
            tail_window(&chunks, 4),
            (
                vec![
                    chunk(StreamKind::Stdout, "b"),
                    chunk(StreamKind::Stderr, "cdé")
                ],
                true
            )
        );
        assert_eq!(
            tail_window(&chunks, 3),
            (vec![chunk(StreamKind::Stderr, "cdé")], true)
        );
    }
}
