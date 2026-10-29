//! What every recording says of itself:
//! the types that `demi browser record` and `demi mobile record` share, so a
//! recording reads the same wherever it was made.
//!
//! The crate holds types and their checks only. The recording itself, its
//! files and its encoding are `shared-recording`'s.
//!
//! - A recording's identity is `r` and a number, given once across a
//!   conversation's recordings on a Host, whichever program made them.
//! - A frame is `f` and a number from 1, zero-padded to three digits, in the
//!   order the recording kept them; only a frame that differs from the one
//!   before is kept.
//! - Every area is in the pixels of the frames the recording kept.

use std::{fmt, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

#[macro_use]
extern crate demi_shared_types;

use demi_shared_types::MAX_SAFE_INTEGER;

/// The longest a recording runs, whether or not it was given a duration: a
/// still screen draws almost nothing, so no size would ever end it.
pub const MAX_DURATION: Duration = Duration::from_secs(120);
/// The most bytes a recording's files take before it stops itself: they are
/// written to the Host's disk.
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;
/// How many recordings a conversation keeps on a Host; one more removes the
/// oldest, so that the Host's disk does not fill with recordings nobody reads.
pub const KEPT: usize = 5;

numbered!(
    /// A recording's identity, which `start` answers: `r` and a number.
    RecordingId,
    "r",
    "a recording ID, such as r1"
);

closed_set! {
    /// What `record` does.
    pub enum RecordAction {
        Start = "start",
        Stop = "stop",
        Frames = "frames",
        Export = "export",
        List = "list",
    }
}

closed_set! {
    /// Why a recording stopped.
    pub enum EndReason {
        /// Someone ran `stop`, or pressed Stop in the work panel.
        Stopped = "stopped",
        /// It ran as long as `--duration` asked.
        Duration = "duration",
        /// It reached the longest a recording runs.
        TimeLimit = "time-limit",
        /// It reached the most its files may take.
        SizeLimit = "size-limit",
        /// Its tab or device closed.
        Closed = "closed",
        /// The capture or the encoding failed; the note says why.
        Failed = "failed",
        /// The program that recorded it ended before it did.
        Interrupted = "interrupted",
    }
}

impl EndReason {
    /// The reason in words, as `stop` tells it.
    pub fn words(self) -> &'static str {
        match self {
            Self::Stopped => "it was stopped",
            Self::Duration => "it ran for the duration asked",
            Self::TimeLimit => "it reached the two-minute limit",
            Self::SizeLimit => "it reached the 200 MB limit",
            Self::Closed => "its tab or device closed",
            Self::Failed => "it failed",
            Self::Interrupted => "the program that recorded it ended",
        }
    }
}

/// A rectangle of the frames, in their pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    pub fn whole(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// The smallest area around both.
    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    /// The part of it inside `width` × `height` pixels, none when nothing
    /// is.
    pub fn within(self, width: u32, height: u32) -> Option<Self> {
        let right = (self.x + self.width).min(width);
        let bottom = (self.y + self.height).min(height);
        (self.x < right && self.y < bottom).then(|| Self {
            x: self.x,
            y: self.y,
            width: right - self.x,
            height: bottom - self.y,
        })
    }
}

impl fmt::Display for Area {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{},{} {}x{}", self.x, self.y, self.width, self.height)
    }
}

/// The area `--region` names: `x,y,width,height`, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(extend("pattern" = Region::PATTERN, "patternDescription" = "x,y,width,height in pixels, such as 40,120,300,180"))]
pub struct Region(pub Area);

impl Region {
    pub const PATTERN: &str = "^[0-9]{1,5},[0-9]{1,5},[0-9]{1,5},[0-9]{1,5}$";
}

impl TryFrom<String> for Region {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let wrong = || format!("{value:?} is not x,y,width,height in pixels, such as 40,120,300,180");
        let parts: Vec<u32> = value
            .split(',')
            .map(|part| part.trim().parse::<u32>().map_err(|_| wrong()))
            .collect::<Result<_, _>>()?;
        match parts[..] {
            [x, y, width, height] if width > 0 && height > 0 && parts.iter().all(|part| *part <= 99_999) => {
                Ok(Self(Area { x, y, width, height }))
            }
            _ => Err(wrong()),
        }
    }
}

impl From<Region> for String {
    fn from(region: Region) -> Self {
        let Area { x, y, width, height } = region.0;
        format!("{x},{y},{width},{height}")
    }
}

/// The frames `--frames` names: ranges and single frames separated by
/// commas, such as `f005-f010,f020`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(extend("pattern" = FrameSet::PATTERN, "patternDescription" = "frames and ranges separated by commas, such as f005-f010,f020"))]
pub struct FrameSet(Vec<(u32, u32)>);

impl FrameSet {
    pub const PATTERN: &str = "^f[0-9]+(-f[0-9]+)?(,f[0-9]+(-f[0-9]+)?)*$";

    /// Whether frame `number` is among them.
    pub fn contains(&self, number: u32) -> bool {
        self.0.iter().any(|(from, to)| (*from..=*to).contains(&number))
    }

    /// The highest frame the set names.
    pub fn last(&self) -> u32 {
        self.0.iter().map(|(_, to)| *to).max().unwrap_or(0)
    }
}

impl TryFrom<String> for FrameSet {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let wrong = || format!("{value:?} is not frames such as f005-f010,f020");
        let number = |text: &str| -> Result<u32, String> {
            let digits = text.trim().strip_prefix('f').ok_or_else(wrong)?;
            match digits.parse::<u32>() {
                Ok(number) if number >= 1 && digits.chars().all(|digit| digit.is_ascii_digit()) => Ok(number),
                _ => Err(wrong()),
            }
        };
        let mut ranges = Vec::new();
        for part in value.split(',') {
            let (from, to) = match part.split_once('-') {
                Some((from, to)) => (number(from)?, number(to)?),
                None => {
                    let single = number(part)?;
                    (single, single)
                }
            };
            if from > to {
                return Err(format!("{part:?} runs backwards"));
            }
            ranges.push((from, to));
        }
        Ok(Self(ranges))
    }
}

impl From<FrameSet> for String {
    fn from(set: FrameSet) -> Self {
        set.0
            .iter()
            .map(|(from, to)| {
                if from == to {
                    frame_label(*from)
                } else {
                    format!("{}-{}", frame_label(*from), frame_label(*to))
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// A frame's label, `f` and its number to at least three digits.
pub fn frame_label(number: u32) -> String {
    format!("f{number:03}")
}

/// How long `start --duration` records: a number of seconds, or numbers
/// with the units `ms`, `s` and `m`, such as `30`, `1.5s` or `1m30s`; at
/// most [`MAX_DURATION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(extend("pattern" = RecordingDuration::PATTERN, "patternDescription" = "a duration of at most two minutes, such as 30s or 1m30s"))]
pub struct RecordingDuration(Duration);

impl RecordingDuration {
    pub const PATTERN: &str = "^([0-9]+(\\.[0-9]+)?(ms|s|m)?)+$";

    pub fn get(self) -> Duration {
        self.0
    }
}

impl TryFrom<String> for RecordingDuration {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let wrong = || format!("{value:?} is not a duration such as 30s, 1.5s or 1m30s");
        let text = value.trim();
        if text.is_empty() {
            return Err(wrong());
        }
        let mut total = 0f64;
        let mut rest = text;
        // A bare number is seconds, and may stand alone only.
        if let Ok(seconds) = text.parse::<f64>() {
            total = seconds;
            rest = "";
        }
        while !rest.is_empty() {
            let digits = rest
                .find(|character: char| !character.is_ascii_digit() && character != '.')
                .ok_or_else(wrong)?;
            let amount: f64 = rest[..digits].parse().map_err(|_| wrong())?;
            let (unit, after) = if let Some(after) = rest[digits..].strip_prefix("ms") {
                (0.001, after)
            } else if let Some(after) = rest[digits..].strip_prefix('s') {
                (1.0, after)
            } else if let Some(after) = rest[digits..].strip_prefix('m') {
                (60.0, after)
            } else {
                return Err(wrong());
            };
            total += amount * unit;
            rest = after;
        }
        if !total.is_finite() || total < 0.001 {
            return Err(format!("{value:?} is too short for a recording"));
        }
        let duration = Duration::from_secs_f64(total);
        if duration > MAX_DURATION {
            return Err(format!("{value:?} is longer than the two minutes a recording lasts"));
        }
        Ok(Self(duration))
    }
}

impl From<RecordingDuration> for String {
    fn from(duration: RecordingDuration) -> Self {
        let milliseconds = duration.0.as_millis();
        if milliseconds % 1000 == 0 {
            format!("{}s", milliseconds / 1000)
        } else {
            format!("{milliseconds}ms")
        }
    }
}

/// What `start` and `stop` say of a recording, and what a listing starts
/// with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingInfo {
    pub id: RecordingId,
    /// The tab or device it records, such as t1 or m1.
    pub target: String,
    /// Whether it still records.
    pub running: bool,
    /// Why it stopped, once it has.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "EndReason")]
    pub ended: Option<EndReason>,
    /// What went wrong, when it stopped because it failed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub note: Option<String>,
    /// How long it ran, in milliseconds.
    #[schemars(range(max = MAX_SAFE_INTEGER))]
    pub duration_ms: u64,
    /// How many frames it kept: those that differed from the one before.
    #[schemars(range(max = MAX_SAFE_INTEGER))]
    pub frames: u64,
    /// How many frames the screen drew that the capture could not take.
    #[schemars(range(max = MAX_SAFE_INTEGER))]
    pub missed: u64,
    /// The area all the frames changed in together.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Area")]
    pub area: Option<Area>,
    /// The size of the frames, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "FrameSize")]
    pub size: Option<FrameSize>,
}

/// A frame's size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

/// One frame the recording kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameRow {
    /// Its number, from 1.
    pub frame: u32,
    /// When it was drawn, in microseconds from the first.
    #[schemars(range(max = MAX_SAFE_INTEGER))]
    pub time_us: u64,
    /// Where it differs from the frame before.
    pub changed: Area,
    /// Whether it differs everywhere, as the first frame and one of another
    /// size do.
    pub whole: bool,
}

closed_set! {
    /// Who gave an input: the agent's command, or the user's hand in the
    /// work panel's live view.
    pub enum InputBy {
        Agent = "agent",
        User = "user",
    }
}

closed_set! {
    /// A mouse button.
    pub enum InputButton {
        Left = "left",
        Middle = "middle",
        Right = "right",
    }
}

closed_set! {
    /// Where a pointer or a finger is in a press.
    pub enum InputPhase {
        Down = "down",
        Move = "move",
        Up = "up",
    }
}

/// An input a recording's source took while it ran. A place is in the
/// frames' pixels, and `scale` is how many of them a point is there, so
/// that what an export draws for it is as large on every screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Input {
    /// A mouse's move, or a press or release of `button`.
    Pointer {
        x: f64,
        y: f64,
        scale: f64,
        phase: InputPhase,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "InputButton")]
        button: Option<InputButton>,
        by: InputBy,
    },
    /// A turn of the wheel by `dx` and `dy` pixels, a positive `dy` down.
    Wheel { x: f64, y: f64, scale: f64, dx: f64, dy: f64, by: InputBy },
    /// A press of a key that is not plain typing, with its modifiers, as
    /// `["Meta", ","]` or `["Enter"]`.
    Keys { keys: Vec<String>, scale: f64, by: InputBy },
    /// Text typed, inserted or filled: how many characters, never what.
    Typed { characters: u32, scale: f64, by: InputBy },
    /// A finger's touch on a device's screen.
    Touch { x: f64, y: f64, scale: f64, phase: InputPhase, by: InputBy },
    /// A device's button, as `home` or `volume-up`.
    Button { name: String, scale: f64, by: InputBy },
    /// One of the agent's commands, at the time it began, and why it failed
    /// when it did.
    Command {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        failed: Option<String>,
    },
}

/// An input at its time, in microseconds from the recording's first frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputRow {
    #[schemars(range(max = MAX_SAFE_INTEGER))]
    pub time_us: u64,
    pub input: Input,
}

impl Input {
    /// Whether `frames` lists it: the agent's commands and the user's
    /// discrete inputs, not the moves of a pointer or a finger, and not the
    /// low-level inputs a command gave, which its own line stands for.
    pub fn listed(&self) -> bool {
        match self {
            Self::Command { .. } => true,
            Self::Pointer { phase, by, .. } | Self::Touch { phase, by, .. } => {
                *by == InputBy::User && *phase == InputPhase::Down
            }
            Self::Wheel { by, .. } | Self::Keys { by, .. } | Self::Typed { by, .. } | Self::Button { by, .. } => {
                *by == InputBy::User
            }
        }
    }
}

/// Where frames the screen drew were missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Gap {
    /// The last frame kept before them; 0 for before the first.
    pub after: u32,
    pub count: u32,
}

/// One image `export` wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportedFrame {
    pub frame: u32,
    pub path: String,
}

/// What `record` answers, whichever of its actions ran: the recording, and
/// what the action adds to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordResult {
    pub recording: RecordingInfo,
    /// For `frames`: the frames kept.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<FrameRow>")]
    pub frames: Option<Vec<FrameRow>>,
    /// For `frames`: where frames were missed between them.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<Gap>")]
    pub gaps: Option<Vec<Gap>>,
    /// For `export`: the images written.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<ExportedFrame>")]
    pub images: Option<Vec<ExportedFrame>>,
    /// For `export`: the area of the frames every image shows.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Area")]
    pub area: Option<Area>,
    /// For `export --mp4 --output`: the video saved.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub path: Option<String>,
    /// For `frames`: the agent's commands and the user's inputs among the
    /// frames listed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<InputRow>")]
    pub inputs: Option<Vec<InputRow>>,
    /// For `export`: why the inputs are not drawn on the frames that were
    /// to have them, as on a Host that cannot load the video coder.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub undrawn: Option<String>,
    /// For `list`: the recordings of the tab or device, newest first;
    /// `recording` is the first of them.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<RecordingInfo>")]
    pub recordings: Option<Vec<RecordingInfo>>,
}

impl RecordResult {
    pub fn of(recording: RecordingInfo) -> Self {
        Self {
            recording,
            frames: None,
            gaps: None,
            images: None,
            area: None,
            path: None,
            inputs: None,
            undrawn: None,
            recordings: None,
        }
    }
}

/// Declares `record`'s input, the same for every source of recordings: the
/// operand and the action, the duration of a start, and what `frames` and
/// `export` take. `$target_doc` says what the operand names, `$length` is the
/// longest `--output` the caller's limits allow, and `$extra` is the fields the
/// caller's inputs all take, such as a deadline.
#[macro_export]
macro_rules! record_input {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            target: $target:ty, $target_doc:literal;
            output_length: $length:ident;
            $($extra:tt)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, garde::Validate)]
        #[serde(rename_all = "kebab-case", deny_unknown_fields)]
        pub struct $name {
            #[doc = $target_doc]
            #[garde(skip)]
            pub target: $target,
            /// start, stop, frames, export or list; list names the tab or device, not a recording
            #[garde(skip)]
            pub action: $crate::RecordAction,
            /// For start: stop by itself after this long, such as 30s or 1m30s; at most 2 minutes
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "RecordingDuration")]
            #[garde(skip)]
            pub duration: Option<$crate::RecordingDuration>,
            /// For frames and export: the frames to list or write, such as f005-f010,f020; every frame by default
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "FrameSet")]
            #[garde(skip)]
            pub frames: Option<$crate::FrameSet>,
            /// For export: write the whole frame, not only the area the chosen frames changed in
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "bool")]
            #[garde(skip)]
            pub whole: Option<bool>,
            /// For export: write this area of the frames, x,y,width,height in pixels of the recorded frames
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "Region")]
            #[garde(skip)]
            pub region: Option<$crate::Region>,
            /// For export: write the recording's video, an MP4, to stdout or to --output, not images
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "bool")]
            #[garde(skip)]
            pub mp4: Option<bool>,
            /// For export: the frames as they were captured, without the inputs drawn on them
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "bool")]
            #[garde(skip)]
            pub plain: Option<bool>,
            /// For export: the directory for the images, or with --mp4 a new file on this Host
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(chars, min = 1, max = $length))]
            pub output: Option<String>,
            /// Replace files that exist
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "bool")]
            #[garde(skip)]
            pub overwrite: Option<bool>,
            $($extra)*
        }
    };
}

/// Declares the first operand of `record`: the tab or device a `start`
/// names, or the recording every other action names, told apart by the
/// identifier's prefix.
#[macro_export]
macro_rules! recording_target {
    ($(#[$meta:meta])* pub enum $name:ident { $source:ident($source_type:ty), $prefix:literal, $what:literal }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
        #[serde(try_from = "String", into = "String")]
        #[schemars(extend("pattern" = $name::PATTERN, "patternDescription" = $what))]
        pub enum $name {
            $source($source_type),
            Recording($crate::RecordingId),
        }

        impl $name {
            pub const PATTERN: &str = concat!("^(", $prefix, "|r)[1-9][0-9]{0,14}$");
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                if value.starts_with('r') {
                    value.parse().map(Self::Recording)
                } else {
                    value.parse().map(Self::$source)
                }
                .map_err(|_| format!("{value:?} is not {}", $what))
            }
        }

        impl From<$name> for String {
            fn from(target: $name) -> Self {
                match target {
                    $name::$source(source) => source.to_string(),
                    $name::Recording(recording) => recording.to_string(),
                }
            }
        }
    };
}

pub mod text;
