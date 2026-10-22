//! What `record` prints, as every `demi` command prints its data
//! (`crates/shared-types/src/output.rs`).

use demi_shared_types::output::{Fields, Table};

use crate::{FrameRow, Gap, RecordAction, RecordResult, RecordingInfo, frame_label};

/// What `record`'s tool description says, for the kind of `source` (`tab` or
/// `device`) that `example` (`t1`, `m1`) names.
pub fn about(source: &str, example: &str) -> String {
    format!(
        "Record every frame a {source} draws, and read what was kept: record {example} start returns a recording ID such as r1 \
         and stops by itself after --duration (at most 2 minutes) or at 2 minutes or 200 MB, or when the {source} closes; \
         record r1 stop ends it; record r1 frames lists only the frames that differ from the one before, with their time \
         and the area that changed; record r1 export writes frames as images labelled with frame number and time, one path \
         per line, so record r1 export --frames f005-f010 | xargs demi file view shows them. --mp4 gives the video itself. \
         A conversation keeps its five newest recordings."
    )
}

/// What `record` prints for `result`, the answer to `action`; `command` is the
/// command's path, such as `demi browser record`, which the text names the
/// commands to go on with by.
pub fn record(action: RecordAction, command: &str, result: &RecordResult) -> String {
    let info = &result.recording;
    match action {
        RecordAction::Start => started(command, info),
        RecordAction::Stop => summary(info),
        RecordAction::Frames => listing(
            info,
            result.frames.as_deref().unwrap_or_default(),
            result.gaps.as_deref().unwrap_or_default(),
        ),
        RecordAction::Export => match (&result.path, &result.images, result.area) {
            (Some(path), _, _) => format!(
                "Saved the video of {} ({} s) to {path}.\n",
                info.id,
                seconds(info.duration_ms * 1000)
            ),
            (None, Some(images), Some(_)) => {
                let images: Vec<(u32, String)> = images.iter().map(|image| (image.frame, image.path.clone())).collect();
                exported(&images)
            }
            _ => summary(info),
        },
    }
}

/// `seconds` of `microseconds`, three decimals, such as `0.483`.
pub fn seconds(microseconds: u64) -> String {
    format!("{:.3}", microseconds as f64 / 1_000_000.0)
}

/// What `start` says: the recording's id and how to go on.
pub fn started(command: &str, info: &RecordingInfo) -> String {
    format!(
        "Recording {} of {}; stop it with {command} {} stop.\n",
        info.id, info.target, info.id
    )
}

/// What `stop` says, and the head of a listing: how long it ran, why it
/// stopped, how many frames changed, how many were missed, and where.
pub fn summary(info: &RecordingInfo) -> String {
    let mut fields = Fields::new();
    fields.field("Recording", info.id.to_string());
    fields.field("Target", info.target.clone());
    match info.ended {
        Some(reason) => {
            fields.field("Ended", reason.words());
            if let Some(note) = &info.note {
                fields.field("Note", note.clone());
            }
        }
        None => {
            fields.field("Ended", "still recording");
        }
    }
    fields.field("Ran (s)", format!("{:.3}", info.duration_ms as f64 / 1000.0));
    fields.field("Changed frames", info.frames.to_string());
    fields.field("Missed frames", info.missed.to_string());
    if let Some(area) = info.area {
        fields.field("Changed area", area.to_string());
    }
    if let Some(size) = info.size {
        fields.field("Frame size", format!("{}x{}", size.width, size.height));
    }
    fields.render()
}

/// A listing of `frames`. A gap of missed frames is a row of its own after the
/// frame it follows, or before the next listed frame when the one it follows
/// is not listed.
pub fn listing(info: &RecordingInfo, frames: &[FrameRow], gaps: &[Gap]) -> String {
    let mut table = Table::new(["frame", "time (s)", "changed area"]);
    let mut text = summary(info);
    text.push('\n');
    let mut missed = gaps.iter().peekable();
    // A gap that follows the frame before the first listed one was listed
    // with it, in the listing of the frames before.
    let first = frames.first().map_or(0, |row| row.frame);
    if first > 1 {
        while missed.next_if(|gap| gap.after < first).is_some() {}
    }
    let missing = |gap: &Gap| [format!("({} missed)", gap.count), String::new(), String::new()];
    for row in frames {
        while let Some(gap) = missed.next_if(|gap| gap.after < row.frame) {
            table.row(missing(gap));
        }
        let area = if row.whole { "whole".to_owned() } else { row.changed.to_string() };
        table.row([frame_label(row.frame), seconds(row.time_us), area]);
        while let Some(gap) = missed.next_if(|gap| gap.after == row.frame) {
            table.row(missing(gap));
        }
    }
    text.push_str(&table.render());
    text
}

/// What `export` of images prints: one path per line and nothing else, so
/// that `export | xargs demi file view` shows the images; the area they show
/// is in the `--json` answer.
pub fn exported(images: &[(u32, String)]) -> String {
    images.iter().map(|(_, path)| format!("{path}\n")).collect()
}
