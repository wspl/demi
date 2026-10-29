//! What `record` prints, as every `demi` command prints its data
//! (`crates/shared-types/src/output.rs`).

use demi_shared_types::output::{Fields, Table};

use crate::{
    FrameRow, Gap, Input, InputBy, InputPhase, InputRow, InputButton, RecordAction, RecordResult, RecordingInfo,
    frame_label,
};

/// What `record`'s tool description says, for the kind of `source` (`tab` or
/// `device`) that `example` (`t1`, `m1`) names.
pub fn about(source: &str, example: &str) -> String {
    format!(
        "Record every frame a {source} draws, and read what was kept: record {example} start returns a recording ID such as r1 \
         and stops by itself after --duration (at most 2 minutes) or at 2 minutes or 200 MB, or when the {source} closes; \
         record r1 stop ends it; record r1 frames lists only the frames that differ from the one before, with their time \
         and the area that changed; record r1 export writes frames as images labelled with frame number and time, one path \
         per line, so record r1 export --frames f005-f010 | xargs demi file view shows them. --mp4 gives the video itself. \
         Exported frames and videos draw the inputs the {source} took, the pointer, clicks, wheel turns, keys and touches; \
         --plain leaves them out. frames lists your commands among the frames, each with how it ended, and what the user \
         did by hand. record {example} list lists the {source}'s recordings. A conversation keeps its five newest recordings."
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
            result.inputs.as_deref().unwrap_or_default(),
        ),
        RecordAction::List => listed(result.recordings.as_deref().unwrap_or_default()),
        RecordAction::Export => match (&result.path, &result.images, result.area) {
            (Some(path), _, _) => {
                let mut text = format!("Saved the video of {} ({} s) to {path}.\n", info.id, seconds(info.duration_ms * 1000));
                if let Some(why) = &result.undrawn {
                    text.push_str(&format!("The inputs are not drawn on it: {why}.\n"));
                }
                text
            }
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

/// A listing of `frames` with `inputs` among them, each at its time; an
/// input at a frame's time comes before it, since the frame shows what it
/// did. A gap of missed frames is a row of its own after the frame it
/// follows, or before the next listed frame when the one it follows is not
/// listed.
pub fn listing(info: &RecordingInfo, frames: &[FrameRow], gaps: &[Gap], inputs: &[InputRow]) -> String {
    let mut table = Table::new(["frame", "time (s)", "changed area or input"]);
    let mut text = summary(info);
    text.push('\n');
    let mut missed = gaps.iter().peekable();
    let mut inputs = inputs.iter().peekable();
    // A gap that follows the frame before the first listed one was listed
    // with it, in the listing of the frames before.
    let first = frames.first().map_or(0, |row| row.frame);
    if first > 1 {
        while missed.next_if(|gap| gap.after < first).is_some() {}
    }
    let missing = |gap: &Gap| [format!("({} missed)", gap.count), String::new(), String::new()];
    let input = |row: &InputRow| [String::new(), seconds(row.time_us), described(&row.input)];
    for row in frames {
        while let Some(gap) = missed.next_if(|gap| gap.after < row.frame) {
            table.row(missing(gap));
        }
        while let Some(early) = inputs.next_if(|input| input.time_us <= row.time_us) {
            table.row(input(early));
        }
        let area = if row.whole { "whole".to_owned() } else { row.changed.to_string() };
        table.row([frame_label(row.frame), seconds(row.time_us), area]);
        while let Some(gap) = missed.next_if(|gap| gap.after == row.frame) {
            table.row(missing(gap));
        }
    }
    for late in inputs {
        table.row(input(late));
    }
    text.push_str(&table.render());
    text
}

/// An input as a listing names it.
fn described(input: &Input) -> String {
    let by = |by: &InputBy| if *by == InputBy::User { "(the user) " } else { "" };
    let point = |x: &f64, y: &f64| format!("{},{}", x.round(), y.round());
    match input {
        Input::Command { text, failed: None } => text.clone(),
        Input::Command { text, failed: Some(why) } => format!("{text} · failed: {why}"),
        Input::Pointer { x, y, phase, button, by: who, .. } => {
            let button = match button {
                Some(InputButton::Right) => "right ",
                Some(InputButton::Middle) => "middle ",
                _ => "",
            };
            let what = match phase {
                InputPhase::Down => "press",
                InputPhase::Up => "release",
                InputPhase::Move => "move",
            };
            format!("{}{button}{what} at {}", by(who), point(x, y))
        }
        Input::Touch { x, y, phase, by: who, .. } => {
            let what = match phase {
                InputPhase::Down => "touch",
                InputPhase::Up => "lift",
                InputPhase::Move => "slide",
            };
            format!("{}{what} at {}", by(who), point(x, y))
        }
        Input::Wheel { x, y, dx, dy, by: who, .. } => {
            format!("{}wheel at {} dx {} dy {}", by(who), point(x, y), dx.round(), dy.round())
        }
        Input::Keys { keys, by: who, .. } => format!("{}key {}", by(who), keys.join("+")),
        Input::Typed { characters: 1, by: who, .. } => format!("{}typed 1 character", by(who)),
        Input::Typed { characters, by: who, .. } => format!("{}typed {characters} characters", by(who)),
        Input::Button { name, by: who, .. } => format!("{}button {name}", by(who)),
    }
}

/// What `list` prints: a row a recording, newest first.
pub fn listed(recordings: &[RecordingInfo]) -> String {
    let mut table = Table::new(["recording", "target", "state", "ran (s)", "frames"]);
    for info in recordings {
        let state = match info.ended {
            Some(reason) => reason.words().to_owned(),
            None => "recording".to_owned(),
        };
        table.row([
            info.id.to_string(),
            info.target.clone(),
            state,
            format!("{:.3}", info.duration_ms as f64 / 1000.0),
            info.frames.to_string(),
        ]);
    }
    table.render()
}

/// What `export` of images prints: one path per line and nothing else, so
/// that `export | xargs demi file view` shows the images; the area they show
/// is in the `--json` answer.
pub fn exported(images: &[(u32, String)]) -> String {
    images.iter().map(|(_, path)| format!("{path}\n")).collect()
}

/// The commands whose `text` is what they type into the screen or give a
/// page, which a recording counts and never keeps; another's `text` is what
/// it looks for.
const TYPING: &[&str] = &["type", "fill", "clipboard.write", "dialog.accept"];

/// The arguments whose values a recording never keeps, only their size: a
/// protocol message or a tool's arguments may carry anything.
const OPAQUE: &[&str] = &["params", "arguments"];

/// The operand every command names first, which a recording names already.
const SUBJECT: &[&str] = &["tab", "device", "target"];

/// Whether a command's failure may be kept as its message says it: one
/// that types, or passes a message a recording keeps only the size of, may
/// quote it in its failure.
pub fn keeps_failure(name: &str) -> bool {
    !TYPING.contains(&name) && !matches!(name, "cdp.send" | "webmcp.call")
}

/// One of the agent's commands as a recording lists it, as the CLI takes
/// it: its name, its operands in `positionals`' order, then its flags; the
/// text it types counted and never quoted, a key that types a character
/// likewise, and an opaque value by its size.
pub fn command(name: &str, arguments: &serde_json::Value, positionals: &[&str]) -> String {
    use serde_json::Value;
    let mut text = name.to_owned();
    let Value::Object(arguments) = arguments else {
        return text;
    };
    let shown = |field: &str, value: &Value| -> Option<String> {
        match (field, value) {
            ("text", Value::String(typed)) if TYPING.contains(&name) => Some(format!("({} characters)", typed.chars().count())),
            ("key", Value::String(key)) if types_a_character(key) => Some("(1 character)".to_owned()),
            (opaque, value) if OPAQUE.contains(&opaque) => {
                let bytes = match value {
                    Value::String(value) => value.len(),
                    value => value.to_string().len(),
                };
                Some(format!("({bytes} bytes)"))
            }
            _ => None,
        }
    };
    let operand = |value: &Value| match value {
        Value::String(value) if !value.is_empty() && !value.contains(char::is_whitespace) => value.clone(),
        Value::String(value) => format!("{value:?}"),
        Value::Number(number) => number_text(number),
        value => value.to_string(),
    };
    for field in positionals.iter().filter(|field| !SUBJECT.contains(field)) {
        match arguments.get(*field) {
            Some(Value::Null) | None => {}
            Some(Value::Array(values)) => {
                for value in values {
                    text.push(' ');
                    text.push_str(&shown(field, value).unwrap_or_else(|| operand(value)));
                }
            }
            Some(value) => {
                text.push(' ');
                text.push_str(&shown(field, value).unwrap_or_else(|| operand(value)));
            }
        }
    }
    for (field, value) in arguments {
        if SUBJECT.contains(&field.as_str()) || positionals.contains(&field.as_str()) || matches!(field.as_str(), "timeout" | "json") {
            continue;
        }
        let values: Vec<&Value> = match value {
            Value::Null => continue,
            Value::Array(values) => values.iter().collect(),
            value => vec![value],
        };
        for value in values {
            match (shown(field, value), value) {
                (Some(shown), _) => text.push_str(&format!(" --{field} {shown}")),
                (None, Value::Bool(true)) => text.push_str(&format!(" --{field}")),
                (None, Value::Bool(false)) => text.push_str(&format!(" --{field}=false")),
                (None, value) => text.push_str(&format!(" --{field} {}", operand(value))),
            }
        }
    }
    text
}

/// Whether the key `key`, as `key` takes it, types a character: one
/// character, alone or with Shift, never with Control or Meta.
fn types_a_character(key: &str) -> bool {
    let mut parts: Vec<&str> = key.split('+').collect();
    let last = parts.pop().unwrap_or_default();
    last.chars().count() == 1 && parts.iter().all(|modifier| *modifier == "Shift" || *modifier == "Alt" || *modifier == "AltGraph")
}

/// A number as a command line writes it: a whole one without its fraction.
fn number_text(number: &serde_json::Number) -> String {
    match number.as_f64() {
        Some(value) if value.fract() == 0.0 && value.abs() < 1e15 => format!("{}", value as i64),
        _ => number.to_string(),
    }
}
