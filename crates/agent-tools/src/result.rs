//! A shell tool's result (`runtime.md` § Results and previews): the text the
//! model reads, the media a binary stdout may attach, and the bounded view
//! the user sees, all from one command status.

use demi_agent_session::ToolOutcome;
use demi_agent_store::images;
use demi_agent_transcript::REPLAY_CHARS;
use demi_core::{
    B64Bytes, CommandId, Model, ModelMediaKind, OutputChunk, ShellToolView, ShellViewStatus,
    ToolView, model_accepts_media_type, sniff_model_media_type,
};
use demi_provider::{MediaBytes, RequestLimits, ResultPart};
use demi_shell::{BinaryOutput, CommandState, CommandStatus, OutputText, Piece, Streams};

use super::PAGE_CHARS;

/// The characters of merged output a shell view keeps, from the end.
pub(super) const VIEW_CHARS: usize = 32_768;
/// The largest video: about ten minutes at a viewing-grade encoding, under
/// the inline payload ceiling the major APIs enforce. An image is fitted
/// instead (`runtime.md` § Images in the transcript).
const VIDEO_CAP_BYTES: u64 = 16 * 1024 * 1024;
/// How many of a running command's newest lines its result points to.
const NEWEST_LINES: u64 = 50;

const RUNNING_NEXT: &str = "next: command is still running; check again with shell_status, or call yield to end this turn and be woken later, or shell_abort to stop it.";

/// A shell tool's outcome for `status`: its text, a binary stdout's media or
/// the reason there is none, and its view. `model` is the call's, and its
/// vendor takes requests within `limits`.
pub(super) async fn shell_outcome(
    status: &CommandStatus,
    model: &Model,
    limits: RequestLimits,
) -> ToolOutcome {
    let text = unseen_output(status);
    let mut output = vec![ResultPart::Text(result_text(status, &text))];
    if let CommandState::Exited {
        binary_stdout: Some(binary),
        ..
    } = &status.state
    {
        let (medium, note) = binary_verdict(binary, &status.command_id, model, limits).await;
        output.extend(medium);
        output.push(ResultPart::Text(note));
    }
    ToolOutcome {
        output,
        is_error: false,
        view: Some(ToolView::Shell(shell_view(status, &text))),
        effect: None,
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

/// The lines the model reads: the status, the handles and timings that
/// matter, the output since the model's last look within the replay bound,
/// and the next step.
fn result_text(status: &CommandStatus, text: &OutputText) -> String {
    let command = &status.command_id;
    let running = matches!(status.state, CommandState::Running { .. });
    let mut before = vec![format!("status: {}", view_status(&status.state))];
    if let CommandState::Exited { exit_code, .. } = status.state {
        before.push(format!("exitCode: {exit_code}"));
    }
    before.push(format!("commandId: {command}"));
    if running {
        before.push(format!("shellId: {}", status.shell_id));
        before.push(format!("runningMs: {}", status.running_ms));
        before.push(format!("idleMs: {}", status.idle_ms));
    }
    let mut after = Vec::new();
    if running && status.unreceived > 0 {
        after.push(format!(
            "[... {} bytes not shown so far; the newest: demi shell output {command} --tail {NEWEST_LINES} ...]",
            status.unreceived
        ));
    }
    match &status.state {
        CommandState::Running { hint } => {
            after.push(hint.clone().unwrap_or_else(|| RUNNING_NEXT.to_owned()));
        }
        CommandState::Aborted => after.push("next: command was intentionally stopped.".to_owned()),
        CommandState::Exited { .. } => {}
    }
    // The other lines, the output's label among them, each with its newline.
    let others: usize = before
        .iter()
        .chain(&after)
        .map(|line| line.chars().count() + 1)
        .sum::<usize>()
        + "output:\n".len();
    let shown = text.unseen_line().map_or_else(Vec::new, |from| {
        cut(text, from, command, REPLAY_CHARS.saturating_sub(others))
    });
    let mut lines = before;
    if shown.is_empty() {
        lines.push("output: (empty)".to_owned());
    } else {
        lines.push("output:".to_owned());
        lines.extend(shown);
    }
    lines.extend(after);
    lines.join("\n")
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

/// What a binary final stdout becomes (`runtime.md` § Results and previews):
/// attached as an image or a video only when the output kept all of it, its
/// bytes are a type of the model-media table and the model accepts that
/// type; an image as it is fitted, and a video within its cap and when its
/// base64 takes at most half of the body `limits` allow. Otherwise a note
/// says why not and how to save the bytes.
async fn binary_verdict(
    binary: &BinaryOutput,
    command: &CommandId,
    model: &Model,
    limits: RequestLimits,
) -> (Option<ResultPart>, String) {
    let media = sniff_model_media_type(&binary.bytes);
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
    let Some(media) = media else {
        return (
            None,
            format!("Binary stdout does not match any model-viewable media type; {save}."),
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
    let data = B64Bytes::new(binary.bytes.clone());
    if media.kind == ModelMediaKind::Image {
        return match images::fit(data, media.media_type).await {
            Ok(fitted) => {
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
                (Some(image), note)
            }
            Err(unfit) => (
                None,
                format!(
                    "Binary stdout is {} ({total} bytes), which was not attached because {unfit}; {save}.",
                    media.media_type
                ),
            ),
        };
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
    let half_body = limits.body_bytes.map(|bytes| bytes / 2);
    if let Some(half) = half_body.filter(|half| data.base64_len() > *half) {
        return (
            None,
            format!(
                "Binary stdout is {} ({total} bytes), whose base64 takes more than {half} bytes, half of what this model's requests may carry, so it was not attached; {save}, or produce a smaller version, with fewer frames or a lower resolution, and run it again.",
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
            let chunks = text.unseen().map_or_else(Vec::new, |from| text.chunks(from));
            tail_window(&chunks, VIEW_CHARS)
        }
        None => {
            let (chunks, cut) = tail_window(&status.output.chunks, VIEW_CHARS);
            (chunks, cut || status.output.truncated)
        }
    };
    ShellToolView {
        status: view_status(&status.state),
        shell_id: status.shell_id.clone(),
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
    use demi_core::{BinaryStdout, FileExtension, OutputView, StreamKind, StreamView};
    use demi_shell::{OutputRecord, Seen, WholeOutput, WholeView};

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
            shell_id: "3".try_into().unwrap(),
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
            whole: Some(WholeView {
                output: Arc::new(output),
                seen: Seen::default(),
            }),
            running_ms: 5,
            idle_ms: 1,
            state: CommandState::Exited {
                exit_code: 1,
                binary_stdout: None,
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

    async fn result(status: &CommandStatus) -> String {
        let outcome =
            shell_outcome(status, &test_model().model, RequestLimits::default()).await;
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
        use demi_shell::{CommandRecord, Ending};

        let mut record = CommandRecord::new(
            "3".try_into().unwrap(),
            "17".try_into().unwrap(),
            "call".into(),
        );
        record.append_output(StreamKind::Stdout, "done\nre");
        record.append_output(StreamKind::Stderr, "a");
        for expected in ["done\nrea", "rea"] {
            let shown = result(&record.status(0, None)).await;
            assert!(
                shown.ends_with(&format!("\noutput:\n{expected}\n{RUNNING_NEXT}")),
                "{shown}"
            );
        }
        record.append_output(StreamKind::Stdout, "dy\nprompt");
        for expected in ["ready\nprompt", "prompt"] {
            let shown = result(&record.status(0, None)).await;
            assert!(
                shown.ends_with(&format!("\noutput:\n{expected}\n{RUNNING_NEXT}")),
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
        record.settle(Ending::Exited(0), Arc::new(whole), None, "");
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
        let bytes: usize = (first..=last).map(|number| format!("{number}\n").len()).sum();
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
                "shellId: 3",
                "runningMs: 5",
                "idleMs: 1",
                "output:",
                "building",
                "[... 1048576 bytes not shown so far; the newest: demi shell output 17 --tail 50 ...]",
                RUNNING_NEXT,
            ]
            .join("\n")
        );
        running.state = CommandState::Running {
            hint: Some("waiting for input: answer with shell_write".into()),
        };
        assert!(result(&running).await.ends_with("\nwaiting for input: answer with shell_write"));
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
        };
        let outcome = shell_outcome(&status, model, limits).await;
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
            format!("<image> | Attached stdout as image/png ({} bytes).", png.len())
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
            format!("Binary stdout is video/mp4, which this model does not accept natively; {save}.")
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
                "Binary stdout is video/mp4 (14 bytes), whose base64 takes more than 19 bytes, half of what this model's requests may carry, so it was not attached; {save}, or produce a smaller version, with fewer frames or a lower resolution, and run it again."
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
            format!("Binary stdout does not match any model-viewable media type; {save}.")
        );
        assert_eq!(
            verdict(&model, limits, &png, true, 20_000_000).await,
            "Binary stdout (20000000 bytes) is more than the 16777216 bytes a command's output keeps whole, so it was not attached and is kept whole nowhere; write it to a file instead and run the command again."
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
