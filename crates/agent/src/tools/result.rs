//! A shell tool's result (`runtime.md` § Results and previews): the text the
//! model reads, the media a binary stdout may attach, and the bounded view
//! the user sees, all from one command status.

use demi_core::{
    B64Bytes, Model, ModelMediaKind, OutputChunk, ShellToolView, ShellViewStatus, StreamView,
    ToolMediaSource, ToolResultContentBlock, ToolView, model_accepts_media_type,
    sniff_model_media_type,
};
use demi_shell::{BinaryOutput, CommandState, CommandStatus};

use crate::session::ToolOutcome;

/// The preview budget below a context window of this many tokens.
const SMALL_CONTEXT_PREVIEW_TOKENS: u32 = 10_000;
/// The preview budget at or above it.
const LARGE_CONTEXT_PREVIEW_TOKENS: u32 = 100_000;
const LARGE_CONTEXT_THRESHOLD_TOKENS: u32 = 800_000;
/// The characters a budget token buys.
const CHARS_PER_TOKEN: usize = 4;
/// The characters of merged output a shell view keeps, from the end.
pub(super) const VIEW_CHARS: usize = 32_768;
/// The largest image a result attaches: well past any sane still.
const IMAGE_CAP_BYTES: u64 = 4 * 1024 * 1024;
/// The largest video: about ten minutes at a viewing-grade encoding, under
/// the inline payload ceiling the major APIs enforce.
const VIDEO_CAP_BYTES: u64 = 16 * 1024 * 1024;

/// The preview budget, in tokens, of a request whose model has
/// `context_window` tokens.
pub(super) fn preview_budget_tokens(context_window: u32) -> u32 {
    if context_window >= LARGE_CONTEXT_THRESHOLD_TOKENS {
        LARGE_CONTEXT_PREVIEW_TOKENS
    } else {
        SMALL_CONTEXT_PREVIEW_TOKENS
    }
}

fn budget_chars(budget_tokens: u32) -> usize {
    budget_tokens as usize * CHARS_PER_TOKEN
}

/// The start of `text` within the budget, and whether it was cut.
/// Characters are Unicode scalar values, so a cut never splits one.
fn bounded_preview(text: &str, budget_tokens: u32) -> (&str, bool) {
    let max = budget_chars(budget_tokens);
    match text.char_indices().nth(max) {
        Some((end, _)) => (&text[..end], true),
        None => (text, false),
    }
}

/// Whether a result must carry the command's handle: the command still runs,
/// or the result could not show all of its output.
pub(super) fn handle_required(status: &CommandStatus, budget_tokens: u32) -> bool {
    if matches!(status.state, CommandState::Running { .. }) {
        return true;
    }
    let (_, cut) = bounded_preview(&status.output.text, budget_tokens);
    cut || status.output.truncated
        || status.output.bytes > budget_chars(budget_tokens) as u64
        || status.stdout.truncated
        || status.stderr.truncated
}

/// A shell tool's outcome for `status`: its text, a binary stdout's media or
/// the reason there is none, and its view.
pub(super) fn shell_outcome(
    status: &CommandStatus,
    budget_tokens: u32,
    expose_handle: bool,
    model: &Model,
) -> ToolOutcome {
    let mut output = vec![ToolResultContentBlock::Text {
        text: result_text(status, budget_tokens, expose_handle),
    }];
    if let CommandState::Exited {
        binary_stdout: Some(binary),
        ..
    } = &status.state
    {
        let (block, note) = binary_verdict(binary, status.stdout.path.as_deref(), model);
        output.extend(block);
        output.push(ToolResultContentBlock::Text { text: note });
    }
    ToolOutcome {
        output,
        is_error: false,
        view: Some(ToolView::Shell(shell_view(status))),
        effect: None,
    }
}

/// The lines the model reads: the status, the handle and timings when they
/// matter, the preview, and the next step.
fn result_text(status: &CommandStatus, budget_tokens: u32, expose_handle: bool) -> String {
    let mut lines = vec![format!("status: {}", view_status(&status.state))];
    if let CommandState::Exited { exit_code, .. } = status.state {
        lines.push(format!("exitCode: {exit_code}"));
    }
    if expose_handle {
        lines.push(format!("shellId: {}", status.shell_id));
        lines.push(format!("commandId: {}", status.command_id));
        lines.push(format!("runningMs: {}", status.running_ms));
        lines.push(format!("idleMs: {}", status.idle_ms));
        stream_lines(&mut lines, "stdout", &status.stdout);
        stream_lines(&mut lines, "stderr", &status.stderr);
    }
    preview_lines(&mut lines, status, budget_tokens);
    match &status.state {
        CommandState::Running { hint } => lines.push(hint.clone().unwrap_or_else(|| {
            "next: command is still running; check again with shell_status, or call yield to end this turn and be woken later, or shell_abort to stop it.".to_owned()
        })),
        CommandState::Aborted => lines.push("next: command was intentionally stopped.".to_owned()),
        CommandState::Exited { .. } if expose_handle => lines.push(
            if status.stdout.path.is_some() {
                "next: command is complete; read the output files on the target only if the preview is insufficient."
            } else {
                "next: command is complete."
            }
            .to_owned(),
        ),
        CommandState::Exited { .. } => {}
    }
    lines.join("\n")
}

fn stream_lines(lines: &mut Vec<String>, label: &str, stream: &StreamView) {
    if let Some(path) = &stream.path {
        lines.push(format!("{label}Path: {path}"));
    }
    lines.push(format!("{label}Bytes: {}", stream.bytes));
}

fn preview_lines(lines: &mut Vec<String>, status: &CommandStatus, budget_tokens: u32) {
    let (preview, cut) = bounded_preview(&status.output.text, budget_tokens);
    lines.push(format!("previewBudgetTokens: {budget_tokens}"));
    if preview.is_empty() {
        lines.push("preview: (empty)".to_owned());
        return;
    }
    lines.push("preview:".to_owned());
    lines.push(preview.to_owned());
    if cut || status.output.truncated {
        lines.push(match (&status.stdout.path, &status.stderr.path) {
            (Some(stdout), Some(stderr)) => format!(
                "previewTruncated: true; read {stdout} or {stderr} on the target for more."
            ),
            _ => "previewTruncated: true; nothing beyond this view was kept — re-run with a narrower command.".to_owned(),
        });
    }
}

/// What a binary final stdout becomes (`runtime.md` § Results and previews):
/// attached as an image or a video only when it is whole, its bytes are a
/// type of the model-media table, the model accepts that type, and it fits
/// its kind's cap; otherwise a note says why not and where the bytes are.
fn binary_verdict(
    binary: &BinaryOutput,
    raw_path: Option<&str>,
    model: &Model,
) -> (Option<ToolResultContentBlock>, String) {
    let media = sniff_model_media_type(&binary.bytes);
    let total = binary.info.total_bytes;
    let place = match raw_path {
        Some(path) => format!("the raw bytes remain readable at {path}"),
        None => "the raw bytes were not kept beyond this view".to_owned(),
    };
    if binary.info.truncated {
        let kind = media.map_or(String::new(), |media| format!(", {}", media.media_type));
        return (
            None,
            format!(
                "Binary stdout ({total} bytes{kind}) exceeded the shell's {}-byte binary limit (maxBinaryBytes) and was not attached; {place}. Produce a smaller version and re-run.",
                binary.info.limit_bytes
            ),
        );
    }
    let Some(media) = media else {
        return (
            None,
            format!("Binary stdout does not match any model-viewable media type; {place}."),
        );
    };
    if !model_accepts_media_type(model, media.media_type) {
        return (
            None,
            format!(
                "Binary stdout is {}, which this model does not accept natively; {place}.",
                media.media_type
            ),
        );
    }
    let (cap, kind) = match media.kind {
        ModelMediaKind::Image => (IMAGE_CAP_BYTES, "image"),
        ModelMediaKind::Video => (VIDEO_CAP_BYTES, "video"),
    };
    if total > cap {
        return (
            None,
            format!(
                "Binary stdout is {} ({total} bytes), over the {cap}-byte {kind} cap; it was not attached and {place}. Produce a smaller version — for video, fewer frames or a lower resolution — and re-run.",
                media.media_type
            ),
        );
    }
    let source = ToolMediaSource::Binary {
        data: B64Bytes::new(binary.bytes.clone()),
        media_type: media.media_type.to_owned(),
    };
    let block = match media.kind {
        ModelMediaKind::Image => ToolResultContentBlock::Image { source },
        ModelMediaKind::Video => ToolResultContentBlock::Video { source },
    };
    (
        Some(block),
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

/// The view the user sees: the status, the end of the merged output, and
/// once the command exited the files it changed.
pub(super) fn shell_view(status: &CommandStatus) -> ShellToolView {
    let (chunks, cut) = tail_window(&status.output.chunks, VIEW_CHARS);
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
        view_truncated: cut || status.output.truncated,
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
    use bytes::Bytes;
    use demi_core::{BinaryStdout, FileExtension, OutputView, StreamKind};
    use demi_shell::EditedFiles;

    use super::*;
    use crate::testing::test_model;

    fn chunk(stream: StreamKind, text: &str) -> OutputChunk {
        OutputChunk {
            stream,
            text: text.to_owned(),
        }
    }

    fn stream(text: &str) -> StreamView {
        StreamView {
            path: Some(format!(
                "/out/cmd-1/{}",
                if text.is_empty() {
                    "stderr.txt"
                } else {
                    "stdout.txt"
                }
            )),
            offset: text.len() as u64,
            delta: text.to_owned(),
            tail: text.to_owned(),
            bytes: text.len() as u64,
            truncated: false,
        }
    }

    /// An exited command whose stdout was `text`.
    fn exited(text: &str) -> CommandStatus {
        CommandStatus {
            shell_id: "shell-1".try_into().unwrap(),
            command_id: "cmd-1".try_into().unwrap(),
            output_dir: Some("/out/cmd-1".into()),
            stdout: stream(text),
            stderr: stream(""),
            output: OutputView {
                path: Some("/out/cmd-1".into()),
                offset: text.len() as u64,
                text: text.to_owned(),
                tail: text.to_owned(),
                chunks: vec![chunk(StreamKind::Stdout, text)],
                bytes: text.len() as u64,
                truncated: false,
            },
            running_ms: 5,
            idle_ms: 1,
            state: CommandState::Exited {
                exit_code: 0,
                binary_stdout: None,
            },
            files: None,
        }
    }

    fn text_of(outcome: &ToolOutcome) -> Vec<&str> {
        outcome
            .output
            .iter()
            .map(|block| match block {
                ToolResultContentBlock::Text { text } => text.as_str(),
                ToolResultContentBlock::Image { .. } => "<image>",
                ToolResultContentBlock::Video { .. } => "<video>",
            })
            .collect()
    }

    #[test]
    fn the_budget_follows_the_models_context_window() {
        assert_eq!(preview_budget_tokens(0), 10_000);
        assert_eq!(preview_budget_tokens(799_999), 10_000);
        assert_eq!(preview_budget_tokens(800_000), 100_000);
    }

    #[test]
    fn a_short_result_shows_everything_and_a_long_one_its_handle_and_paths() {
        let short = exited("done\n");
        assert!(!handle_required(&short, 1_000));
        let mut edited = short.clone();
        edited.files = Some(EditedFiles {
            files: Vec::new(),
            truncated: true,
        });
        let outcome = shell_outcome(&edited, 1_000, false, &test_model().model);
        assert_eq!(
            text_of(&outcome),
            ["status: exited\nexitCode: 0\npreviewBudgetTokens: 1000\npreview:\ndone\n"]
        );
        let Some(ToolView::Shell(view)) = &outcome.view else {
            panic!("a shell view")
        };
        assert_eq!(
            (view.exit_code, view.files_truncated),
            (Some(0), Some(true))
        );

        let long = exited(&format!("{}tail", "x".repeat(4_200)));
        assert!(handle_required(&long, 1_000));
        let text = text_of(&shell_outcome(&long, 1_000, true, &test_model().model))[0].to_owned();
        for line in [
            "shellId: shell-1",
            "commandId: cmd-1",
            "stdoutPath: /out/cmd-1/stdout.txt",
            "stdoutBytes: 4204",
            "stderrPath: /out/cmd-1/stderr.txt",
            "previewTruncated: true; read /out/cmd-1/stdout.txt or /out/cmd-1/stderr.txt on the target for more.",
            "next: command is complete; read the output files on the target only if the preview is insufficient.",
        ] {
            assert!(
                text.lines().any(|candidate| candidate == line),
                "{line} in {text}"
            );
        }
        assert!(!text.contains("tail"));
    }

    #[test]
    fn a_running_command_names_its_hint_or_the_generic_next_step() {
        let mut running = exited("");
        running.state = CommandState::Running { hint: None };
        assert!(handle_required(&running, 1_000));
        let text =
            text_of(&shell_outcome(&running, 1_000, true, &test_model().model))[0].to_owned();
        assert!(text.contains("preview: (empty)"));
        assert!(text.ends_with("next: command is still running; check again with shell_status, or call yield to end this turn and be woken later, or shell_abort to stop it."));
        running.state = CommandState::Running {
            hint: Some("waiting for input: answer with shell_write".into()),
        };
        let text =
            text_of(&shell_outcome(&running, 1_000, true, &test_model().model))[0].to_owned();
        assert!(text.ends_with("\nwaiting for input: answer with shell_write"));
        let mut aborted = exited("");
        aborted.state = CommandState::Aborted;
        let text =
            text_of(&shell_outcome(&aborted, 1_000, true, &test_model().model))[0].to_owned();
        assert!(text.ends_with("next: command was intentionally stopped."));
    }

    #[test]
    fn a_binary_stdout_is_attached_only_when_whole_known_accepted_and_small_enough() {
        let png = Bytes::from_static(b"\x89PNG\r\n\x1a\n\x00\xff\xfe\x01");
        let mp4 = Bytes::from_static(b"\0\0\0\x20ftypisom\xff\xfe");
        let opaque = Bytes::from_static(b"\xde\xad\xbe\xef\xff\xfe\0\x01\x02\x03\x04\x05");
        let mut model = test_model().model;
        model.accepted_extensions = Some(vec![FileExtension::Png]);
        let verdict = |bytes: &Bytes, truncated: bool, total: u64| {
            let mut status = exited("<binary stdout>\n");
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
            let outcome = shell_outcome(&status, 1_000, true, &model);
            text_of(&outcome)[1..].join(" | ")
        };
        let place = "the raw bytes remain readable at /out/cmd-1/stdout.txt";
        assert_eq!(
            verdict(&png, false, 12),
            "<image> | Attached stdout as image/png (12 bytes)."
        );
        assert_eq!(
            verdict(&mp4, false, 14),
            format!(
                "Binary stdout is video/mp4, which this model does not accept natively; {place}."
            )
        );
        assert_eq!(
            verdict(&opaque, false, 12),
            format!("Binary stdout does not match any model-viewable media type; {place}.")
        );
        assert_eq!(
            verdict(&png, true, 20_000_000),
            format!(
                "Binary stdout (20000000 bytes, image/png) exceeded the shell's 16777216-byte binary limit (maxBinaryBytes) and was not attached; {place}. Produce a smaller version and re-run."
            )
        );
        assert_eq!(
            verdict(&png, false, 5 * 1024 * 1024),
            format!(
                "Binary stdout is image/png (5242880 bytes), over the 4194304-byte image cap; it was not attached and {place}. Produce a smaller version — for video, fewer frames or a lower resolution — and re-run."
            )
        );
    }

    #[test]
    fn a_preview_cut_keeps_whole_characters() {
        let text = format!("{}🙂{}", "x".repeat(3_999), "y".repeat(100));
        let (preview, cut) = bounded_preview(&text, 1_000);
        assert!(cut);
        assert_eq!(preview.chars().count(), 4_000);
        assert!(preview.ends_with('🙂'));
        assert_eq!(bounded_preview("short", 1_000), ("short", false));
    }

    #[test]
    fn the_view_window_keeps_the_newest_characters_of_the_merged_output() {
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
