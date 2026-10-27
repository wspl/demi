//! The `mobile.*` operations: their limits, the values they share, their
//! failures, and each operation's input and result.
//!
//! - Inputs are the flat argument objects a command's flags and positionals
//!   make.
//! - Results are what `--json` prints.
//! - Every coordinate an input takes or a result gives is a pixel of the
//!   device's screenshot, whose long side is at most [`SCREENSHOT_LONG_SIDE`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

pub use demi_shared_recording_protocol::{
    RecordAction, RecordResult, RecordingDuration, FrameSet, Region, record_input, recording_target,
};
pub use demi_shared_recording_protocol::text::about as record_about;

/// The prefix of every operation's name, such as `mobile.open`.
pub const PREFIX: &str = "mobile.";

/// The longest side of a screenshot, the most a model reads at full detail;
/// every coordinate is in this image's pixels.
pub const SCREENSHOT_LONG_SIDE: u32 = 1568;
/// How long `wait` waits when it names no timeout, and the longest it may
/// name, in seconds.
pub const WAIT_SECONDS: u64 = 30;
/// How long a command waits for the element a `text=` or `id=` target
/// names, by default: long enough for a page to load and slide in, short
/// enough that a misspelled text fails soon; a longer load is waited for
/// with `wait` or `--timeout`.
pub const FIND_SECONDS: u64 = 10;
pub const MAX_WAIT_SECONDS: u64 = 300;
/// The longest output path, in characters: the longest path the systems the
/// Host runs on take.
pub const PATH_LENGTH: usize = 4096;
/// The most bytes of a device's log Demi keeps, as its entries' JSON: the
/// log is followed into memory, never to the disk, for as long as the
/// device is open; about half an hour of an app that logs heavily.
pub const LOG_BYTES: usize = 16 * 1024 * 1024;
/// How many entries `logs` lists when the input names no limit, as
/// `demi browser logs` does.
pub const LOG_ENTRIES: usize = 100;

numbered!(
    /// A device's handle in the conversation, which `open` answers: `m` and
    /// the device's number, given once across the conversation's Hosts.
    DeviceId,
    "m",
    "a device handle, such as m1"
);
numbered!(
    /// An element of the device's screen, which `inspect` answers: `e` and
    /// a number unique within the device. It refers to the element the last
    /// `inspect` listed.
    ElementRef,
    "e",
    "an element ref, such as e7"
);

closed_set! {
    /// The system a simulator runs.
    pub enum MobilePlatform {
        Ios = "ios",
        Android = "android",
    }
}

closed_set! {
    /// Whether a simulator runs.
    pub enum PowerState {
        ShutDown = "shut-down",
        Booting = "booting",
        Booted = "booted",
        ShuttingDown = "shutting-down",
    }
}

closed_set! {
    /// Which way the device's screen is turned, as it shows.
    pub enum Orientation {
        Portrait = "portrait",
        LandscapeLeft = "landscape-left",
        PortraitUpsideDown = "portrait-upside-down",
        LandscapeRight = "landscape-right",
    }
}

impl Orientation {
    /// Whether the screen's long side lies across.
    pub fn landscape(self) -> bool {
        matches!(self, Self::LandscapeLeft | Self::LandscapeRight)
    }

    /// The orientation a quarter turn counterclockwise, as Simulator's
    /// Rotate Left turns the device.
    pub fn left(self) -> Self {
        match self {
            Self::Portrait => Self::LandscapeLeft,
            Self::LandscapeLeft => Self::PortraitUpsideDown,
            Self::PortraitUpsideDown => Self::LandscapeRight,
            Self::LandscapeRight => Self::Portrait,
        }
    }

    /// The orientation a quarter turn clockwise.
    pub fn right(self) -> Self {
        match self {
            Self::Portrait => Self::LandscapeRight,
            Self::LandscapeRight => Self::PortraitUpsideDown,
            Self::PortraitUpsideDown => Self::LandscapeLeft,
            Self::LandscapeLeft => Self::Portrait,
        }
    }
}

closed_set! {
    /// What `rotate` asks for: an orientation, or the panel's quarter turn
    /// from the one `demi-mobile` set.
    pub enum Rotation {
        Portrait = "portrait",
        Landscape = "landscape",
        Left = "left",
        Right = "right",
    }
}

closed_set! {
    /// How far a foldable is open, as its view's buttons name the postures.
    pub enum Posture {
        Closed = "closed",
        HalfOpen = "half-open",
        Open = "open",
    }
}

closed_set! {
    /// Which of a foldable's two displays is lit.
    pub enum FoldDisplay {
        Outer = "outer",
        Inner = "inner",
    }
}

/// Where `fold` moves a foldable: a posture, or a hinge's angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldTo {
    Posture(Posture),
    /// From 0 (closed) to [`FLAT_DEGREES`].
    Degrees(u16),
}

/// A hinge's angle when the device lies flat, fully open.
pub const FLAT_DEGREES: u16 = 180;
/// The hinge's angle Half Open moves it to, as Xcode's Device Hub sets it.
pub const HALF_OPEN_DEGREES: u16 = 130;

impl FoldTo {
    /// What `fold`'s `to` names: closed, half-open, open, or whole degrees.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "closed" => Some(Self::Posture(Posture::Closed)),
            "half-open" => Some(Self::Posture(Posture::HalfOpen)),
            "open" => Some(Self::Posture(Posture::Open)),
            degrees => degrees
                .trim_end_matches('°')
                .parse::<u16>()
                .ok()
                .filter(|degrees| *degrees <= FLAT_DEGREES)
                .map(Self::Degrees),
        }
    }

    /// The hinge's angle it moves to.
    pub fn degrees(self) -> u16 {
        match self {
            Self::Posture(Posture::Closed) => 0,
            Self::Posture(Posture::HalfOpen) => HALF_OPEN_DEGREES,
            Self::Posture(Posture::Open) => FLAT_DEGREES,
            Self::Degrees(degrees) => degrees,
        }
    }
}

impl Posture {
    /// The posture a hinge at `degrees` is in: closed shut, open flat, half
    /// open between.
    pub fn of(degrees: u16) -> Self {
        match degrees {
            0 => Self::Closed,
            FLAT_DEGREES.. => Self::Open,
            _ => Self::HalfOpen,
        }
    }
}

closed_set! {
    /// The system's light or dark appearance.
    pub enum Appearance {
        Light = "light",
        Dark = "dark",
    }
}

closed_set! {
    /// What `appearance` asks for: one of them, or the panel's toggle, which
    /// reads the appearance once and sets the other.
    pub enum AppearanceChoice {
        Light = "light",
        Dark = "dark",
        Toggle = "toggle",
    }
}

closed_set! {
    /// How much an entry of a device's log matters, the least first: Android's
    /// verbose to fatal (`F` and `A` are fault), and the unified log's debug
    /// to fault, whose default level is notice, as Swift's `Logger` names it.
    #[derive(PartialOrd, Ord)]
    pub enum LogLevel {
        Verbose = "verbose",
        Debug = "debug",
        Info = "info",
        Notice = "notice",
        Warning = "warning",
        Error = "error",
        Fault = "fault",
    }
}

closed_set! {
    /// A hardware button or a key `key` presses.
    pub enum Key {
        Home = "home",
        Back = "back",
        Lock = "lock",
        VolumeUp = "volume-up",
        VolumeDown = "volume-down",
        Enter = "enter",
        Delete = "delete",
        Tab = "tab",
        Escape = "escape",
    }
}

closed_set! {
    /// Which way `swipe` moves an element's content: the finger moves the
    /// other way, as a person scrolls a list up by swiping up.
    pub enum Direction {
        Up = "up",
        Down = "down",
        Left = "left",
        Right = "right",
    }
}

/// A point of the screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// A rectangle of the screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Frame {
    /// Its centre, where an action on its element lands.
    pub fn centre(&self) -> Point {
        Point {
            x: self.x + i32::try_from(self.width / 2).unwrap_or(i32::MAX),
            y: self.y + i32::try_from(self.height / 2).unwrap_or(i32::MAX),
        }
    }
}

/// One simulator as `list` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Simulator {
    pub name: String,
    pub platform: MobilePlatform,
    /// The kind of device it simulates, such as iPhone 17 Pro.
    pub model: String,
    /// Its system and version, such as iOS 26.5.
    pub os: String,
    pub state: PowerState,
    /// The `simctl` id or AVD name the native tools take.
    pub native_id: String,
    /// Its handle while this conversation has it open.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "DeviceId")]
    pub open: Option<DeviceId>,
    /// Whether another conversation has it open.
    pub elsewhere: bool,
}

/// A platform this Host cannot run simulators of, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Unavailable {
    pub platform: MobilePlatform,
    pub reason: String,
}

/// An element as an action names it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElementSummary {
    #[serde(rename = "ref")]
    pub r#ref: ElementRef,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub label: Option<String>,
}

/// One element of the screen's tree: its
/// reference, role, text or label, value, identifier, frame and state. A
/// state the tree does not carry is left out rather than reported false.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Element {
    #[serde(rename = "ref")]
    pub r#ref: ElementRef,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub label: Option<String>,
    /// A field's text, never a secure field's.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub identifier: Option<String>,
    pub frame: Frame,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    pub selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    pub focused: Option<bool>,
    /// A secure field, whose value is never read.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    pub protected: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Element>,
}

impl Element {
    pub fn summary(&self) -> ElementSummary {
        ElementSummary {
            r#ref: self.r#ref.clone(),
            role: self.role.clone(),
            label: self.label.clone(),
        }
    }
}

/// Declares the operations: each name, its input and its result, the
/// [`MobileOperation`] that decodes them, and [`OPERATIONS`].
macro_rules! operations {
    ($($name:literal => $variant:ident($input:ident) -> $result:ident;)*) => {
        /// A decoded `mobile.*` operation and its checked input.
        #[derive(Debug, Clone, PartialEq)]
        pub enum MobileOperation {
            $($variant($input),)*
        }

        /// Every operation's full name, as the descriptor lists them.
        pub const OPERATIONS: &[&str] = &[$(concat!("mobile.", $name),)*];

        impl MobileOperation {
            /// Decodes the operation `name`, without its prefix.
            pub fn parse(name: &str, args: serde_json::Value) -> Result<Self, crate::OperationError> {
                match name {
                    $($name => Ok(Self::$variant(demi_shared_types::decode_value(args)?)),)*
                    _ => Err(crate::OperationError::Unknown(format!("{PREFIX}{name}"))),
                }
            }

            /// Its name, without the prefix.
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $name,)*
                }
            }

            /// The device it acts on; `list` and `open` act on none yet.
            pub fn device(&self) -> Option<&DeviceId> {
                match self {
                    $(Self::$variant(input) => input.device(),)*
                }
            }
        }
    };
}

operations! {
    "list" => List(ListInput) -> ListResult;
    "open" => Open(OpenInput) -> OpenResult;
    "close" => Close(CloseInput) -> CloseResult;
    "screenshot" => Screenshot(ScreenshotInput) -> ScreenshotResult;
    "inspect" => Inspect(InspectInput) -> InspectResult;
    "tap" => Tap(TapInput) -> TapResult;
    "swipe" => Swipe(SwipeInput) -> SwipeResult;
    "type" => Type(TypeInput) -> TypeResult;
    "key" => Key(KeyInput) -> KeyResult;
    "rotate" => Rotate(RotateInput) -> RotateResult;
    "fold" => Fold(FoldInput) -> FoldResult;
    "appearance" => Appearance(AppearanceInput) -> AppearanceResult;
    "wait" => Wait(WaitInput) -> WaitResult;
    "record" => Record(RecordInput) -> RecordResult;
    "install" => Install(InstallInput) -> InstallResult;
    "launch" => Launch(LaunchInput) -> LaunchResult;
    "logs" => Logs(LogsInput) -> LogsResult;
    "erase" => Erase(EraseInput) -> EraseResult;
}

/// What every input says of the device it acts on.
pub trait DeviceInput {
    fn device(&self) -> Option<&DeviceId>;
}

/// Declares an input that acts on an open device, named first.
macro_rules! device_input {
    ($(#[$meta:meta])* pub struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
        #[serde(rename_all = "kebab-case", deny_unknown_fields)]
        pub struct $name {
            /// Device handle open returned, such as m1
            #[garde(skip)]
            pub device: DeviceId,
            $($body)*
        }

        impl DeviceInput for $name {
            fn device(&self) -> Option<&DeviceId> {
                Some(&self.device)
            }
        }
    };
}

/// `list`: every simulator of this Host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ListInput {}

impl DeviceInput for ListInput {
    fn device(&self) -> Option<&DeviceId> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListResult {
    pub devices: Vec<Simulator>,
    /// The platforms this Host runs no simulators of, and why.
    pub unavailable: Vec<Unavailable>,
}

/// `open`: boots a simulator when it is shut down and opens it in the
/// conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct OpenInput {
    /// A simulator's name, simctl id or AVD name, as list shows it
    #[garde(length(chars, min = 1))]
    pub device: String,
}

impl DeviceInput for OpenInput {
    fn device(&self) -> Option<&DeviceId> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenResult {
    pub device: DeviceId,
    pub name: String,
    pub platform: MobilePlatform,
    pub os: String,
    pub native_id: String,
    /// The `adb` serial of an Android emulator, such as `emulator-5554`,
    /// which `adb -s` takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    pub orientation: Orientation,
    /// The screenshot's size.
    pub width: u32,
    pub height: u32,
    /// How far a foldable is open; none for a device that does not fold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<Posture>,
    /// Whether this open booted it, so closing it shuts it down.
    pub booted: bool,
}

device_input! {
    /// `close`: detaches the device, and shuts it down when Demi booted it.
    pub struct CloseInput {}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseResult {
    pub device: DeviceId,
    pub shut_down: bool,
}

device_input! {
    /// `screenshot`: a PNG of the settled screen.
    pub struct ScreenshotInput {
        /// New output file on this Host; without it the PNG goes to stdout
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1, max = PATH_LENGTH))]
        pub output: Option<String>,
        /// Replace an existing output file
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub overwrite: Option<bool>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScreenshotResult {
    pub device: DeviceId,
    pub path: String,
    pub width: u32,
    pub height: u32,
    /// Whether the screen had stopped changing.
    pub settled: bool,
}

device_input! {
    /// `inspect`: the screen's tree, every window, system alerts and the
    /// keyboard included.
    pub struct InspectInput {
        /// List the software keyboard's keys instead of one Keyboard element
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub keyboard: Option<bool>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectResult {
    pub device: DeviceId,
    pub orientation: Orientation,
    /// The screenshot's size, whose pixels the frames are in.
    pub width: u32,
    pub height: u32,
    pub settled: bool,
    pub elements: Vec<Element>,
}

device_input! {
    /// `tap`: a tap at a point or on an element.
    pub struct TapInput {
        /// An element: a ref from the last inspect, text=<text it shows> or id=<identifier>; or x and y in screenshot pixels
        #[garde(length(min = 1, max = 2), inner(length(chars, min = 1)))]
        pub at: Vec<String>,
        /// Hold for a second, a long press
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub long: Option<bool>,
        /// Seconds to wait for a text= or id= element; default 10, at most 300
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "u64")]
        #[garde(range(min = 1, max = MAX_WAIT_SECONDS))]
        pub timeout: Option<u64>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TapResult {
    pub device: DeviceId,
    pub point: Point,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ElementSummary")]
    pub element: Option<ElementSummary>,
}

device_input! {
    /// `swipe`: a finger from one point to another, or across an element.
    pub struct SwipeInput {
        /// x1 y1 x2 y2 in screenshot pixels, or an element (a ref, text=<text> or id=<identifier>) and up, down, left or right
        #[garde(length(min = 2, max = 4), inner(length(chars, min = 1)))]
        pub at: Vec<String>,
        /// Seconds to wait for a text= or id= element; default 10, at most 300
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "u64")]
        #[garde(range(min = 1, max = MAX_WAIT_SECONDS))]
        pub timeout: Option<u64>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SwipeResult {
    pub device: DeviceId,
    pub from: Point,
    pub to: Point,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ElementSummary")]
    pub element: Option<ElementSummary>,
}

device_input! {
    /// `type`: text to the focused field, or into an element.
    pub struct TypeInput {
        /// The text to type
        #[garde(length(chars, min = 1))]
        pub text: String,
        /// Tap this element first, then type into it: a ref, text=<text> or id=<identifier>
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1))]
        pub into: Option<String>,
        /// Set the field's text at once instead of adding to it; needs --into
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub replace: Option<bool>,
        /// Seconds to wait for a text= or id= element; default 10, at most 300
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "u64")]
        #[garde(range(min = 1, max = MAX_WAIT_SECONDS))]
        pub timeout: Option<u64>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeResult {
    pub device: DeviceId,
    /// How many characters it typed or set.
    #[schemars(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub characters: usize,
    pub replaced: bool,
    /// The fields the text went into, in order: one, or each of a row of
    /// fields that take a character each, as a verification code's boxes.
    pub fields: Vec<TypedField>,
    /// The keys left something else than the text in the field, so the
    /// field's text was set directly.
    pub set: bool,
}

/// A field `type` typed into, and what it holds afterwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypedField {
    pub element: ElementSummary,
    /// The field's text once the text went in; for a field of a row, the
    /// characters it took, which the app may have acted on since, as by
    /// clearing a code it submitted. None for a password field, whose text
    /// cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub holds: Option<String>,
}

device_input! {
    /// `key`: a hardware button or a key.
    pub struct KeyInput {
        /// home, back (Android), lock, volume-up, volume-down, enter, delete, tab or escape
        #[garde(skip)]
        pub key: Key,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyResult {
    pub device: DeviceId,
    pub key: Key,
}

device_input! {
    /// `rotate`: turns the device.
    pub struct RotateInput {
        /// portrait or landscape, or left or right for a quarter turn
        #[garde(skip)]
        pub orientation: Rotation,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateResult {
    pub device: DeviceId,
    /// The orientation the screen shows after the turn: the one asked for,
    /// unless the app in front refused it.
    pub orientation: Orientation,
    /// The screenshot's size in that orientation.
    pub width: u32,
    pub height: u32,
}

device_input! {
    /// `fold`: folds or opens a foldable.
    pub struct FoldInput {
        /// closed, half-open or open, or the hinge's angle in degrees from 0 (closed) to 180 (flat); an Android foldable takes closed or open
        #[garde(length(chars, min = 1))]
        pub to: String,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FoldResult {
    pub device: DeviceId,
    pub posture: Posture,
    /// The hinge's angle; none on a device that only folds or opens.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_shared_types::Nullable<u16>")]
    pub degrees: Option<u16>,
    /// The display that is lit after the fold, which screenshots show.
    pub display: FoldDisplay,
    /// The screenshot's size on that display.
    pub width: u32,
    pub height: u32,
}

device_input! {
    /// `appearance`: light or dark.
    pub struct AppearanceInput {
        /// light or dark, or toggle for the other one
        #[garde(skip)]
        pub appearance: AppearanceChoice,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppearanceResult {
    pub device: DeviceId,
    pub appearance: Appearance,
}

device_input! {
    /// `wait`: until text or an element appears, or an element is gone.
    pub struct WaitInput {
        /// Wait until an element shows this text, as text= finds it
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1))]
        pub text: Option<String>,
        /// Wait until this element is on the screen: a ref, text=<text> or id=<identifier>
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1))]
        pub element: Option<String>,
        /// Wait until this element is gone from the screen: a ref, text=<text> or id=<identifier>
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1))]
        pub gone: Option<String>,
        /// Seconds to wait; default 30, at most 300
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "u64")]
        #[garde(range(min = 1, max = MAX_WAIT_SECONDS))]
        pub timeout: Option<u64>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitResult {
    pub device: DeviceId,
    /// The element that appeared, for --text and --element.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ElementSummary")]
    pub element: Option<ElementSummary>,
    #[schemars(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub waited_ms: u64,
}

device_input! {
    /// `install`: an app's build onto the device.
    pub struct InstallInput {
        /// The build on this Host: an .apk for an Android emulator, an .app for an iOS simulator
        #[garde(length(chars, min = 1, max = PATH_LENGTH))]
        pub file: String,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallResult {
    pub device: DeviceId,
    /// The app's id, its Android package or iOS bundle identifier, which
    /// `launch` and `logs` take.
    pub app: String,
    /// The version the build names, such as 2.3.0.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub version: Option<String>,
}

device_input! {
    /// `launch`: an installed app in front, as a tap on its icon brings it.
    pub struct LaunchInput {
        /// The app's id, such as com.example.app, as install prints it
        #[garde(length(chars, min = 1))]
        pub app: String,
        /// Stop the app first, so it starts from the beginning
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub restart: Option<bool>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LaunchResult {
    pub device: DeviceId,
    pub app: String,
    /// Whether the app started anew, rather than coming back in front.
    pub started: bool,
    /// Whether `logs` holds what the app prints to its standard output and
    /// error, which an iOS app shows only when Demi started it.
    pub console: bool,
    /// Whether the app's first screen showed before the launch returned.
    pub shown: bool,
}

device_input! {
    /// `logs`: the device's log entries after a cursor.
    pub struct LogsInput {
        /// Only the entries of this app, such as com.example.app, across its restarts
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1))]
        pub app: Option<String>,
        /// Only the entries whose text or tag holds this text
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1))]
        pub filter: Option<String>,
        /// Only the entries at this level or above: verbose, debug, info, notice, warning, error or fault
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "LogLevel")]
        #[garde(skip)]
        pub level: Option<LogLevel>,
        /// Cursor a previous read printed; without it, the last entries
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1))]
        pub after: Option<String>,
        /// The most entries to list; default 100
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1))]
        pub limit: Option<usize>,
    }
}

/// One entry of a device's log, numbered in the device's order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    #[schemars(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub sequence: u64,
    /// Milliseconds since the Unix epoch.
    pub timestamp: f64,
    pub level: LogLevel,
    pub pid: u32,
    /// The Android user id of the process, which each app has its own of.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "u32")]
    pub uid: Option<u32>,
    /// The iOS process's executable, such as SpringBoard.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub process: Option<String>,
    /// Android's tag, the unified log's category, or console for what an
    /// iOS app prints.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub tag: Option<String>,
    pub text: String,
}

impl demi_shared_types::buffer::Entry for LogEntry {
    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn set_sequence(&mut self, sequence: u64) {
        self.sequence = sequence;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogsResult {
    pub device: DeviceId,
    pub entries: Vec<LogEntry>,
    /// What the next read takes as --after.
    pub cursor: String,
    /// More entries match than the limit listed.
    pub has_more: bool,
    /// How many entries after the cursor are gone: the log kept the last
    /// [`LOG_BYTES`] of them.
    #[schemars(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub dropped: u64,
}

device_input! {
    /// `erase`: the device back to its factory state, started again.
    pub struct EraseInput {}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EraseResult {
    pub device: DeviceId,
}

recording_target! {
    /// What `record`'s first operand names: the device `start` records, or
    /// the recording every other action works on.
    pub enum RecordTarget { Device(DeviceId), "m", "a device handle such as m1, or a recording such as r1" }
}

record_input! {
    /// `record`: records every picture a device draws, and reads what it kept.
    pub struct RecordInput {
        target: RecordTarget, "For start, the device handle open returned, such as m1; for every other action, the recording's ID, such as r1";
        output_length: PATH_LENGTH;
    }
}

impl DeviceInput for RecordInput {
    fn device(&self) -> Option<&DeviceId> {
        match (&self.target, self.action) {
            (RecordTarget::Device(device), RecordAction::Start | RecordAction::Stop) => Some(device),
            _ => None,
        }
    }
}

closed_set! {
    /// Why an operation failed. Each cause keeps its own code.
    pub enum MobileErrorCode {
        InvalidInput = "invalid_input",
        DeviceNotFound = "device_not_found",
        /// More than one simulator has the name `open` was given.
        AmbiguousDevice = "ambiguous_device",
        /// Another conversation has the device open.
        DeviceBusy = "device_busy",
        /// The ref names no element of the device's last `inspect`.
        StaleRef = "stale_ref",
        Timeout = "timeout",
        /// This Host cannot run the platform's simulators, or not yet.
        Unavailable = "unavailable",
        /// The device already records.
        Recording = "recording",
        /// The recording is not on this Host, or was removed.
        RecordingNotFound = "recording_not_found",
        /// Five recordings run, so no other starts.
        TooManyRecordings = "too_many_recordings",
        OutputExists = "output_exists",
        IoError = "io_error",
        DriverError = "driver_error",
        /// The device stopped while it was open.
        DeviceLost = "device_lost",
        Cancelled = "cancelled",
    }
}

/// A failed operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MobileFailure {
    pub code: MobileErrorCode,
    pub message: String,
}

/// What a failed operation writes to stderr under `--json`, and what a
/// package call's failure carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailureDocument {
    pub error: MobileFailure,
}

closed_set! {
    /// What changed in a conversation's devices.
    pub enum DevicesChange {
        Changed = "changed",
    }
}

/// The message the package sends its plugin when a conversation's open
/// devices change (`crates/plugin-interface/src/request.rs`):
/// `{"devices":"changed"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct MobileMessage {
    #[garde(skip)]
    pub devices: DevicesChange,
}
