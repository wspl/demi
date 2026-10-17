//! The `mobile.*` operations (`mobile.md` § Commands): their limits, the
//! values they share, their failures, and each operation's input and
//! result. Inputs are the flat argument objects a command's flags and
//! positionals make; results are what `--json` prints. Every coordinate an
//! input takes or a result gives is a pixel of the device's screenshot,
//! whose long side is at most [`SCREENSHOT_LONG_SIDE`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

/// The prefix of every operation's name, such as `mobile.open`.
pub const PREFIX: &str = "mobile.";

/// The longest side of a screenshot, the most a model reads at full detail;
/// every coordinate is in this image's pixels.
pub const SCREENSHOT_LONG_SIDE: u32 = 1568;
/// The most devices a conversation has open on one Host.
pub const MAX_DEVICES: usize = 4;
/// How long `wait` waits when it names no timeout, and the longest it may
/// name, in seconds.
pub const WAIT_SECONDS: u64 = 30;
pub const MAX_WAIT_SECONDS: u64 = 300;
/// The longest text `type` types, in characters.
pub const TEXT_LENGTH: usize = 10_000;
/// The longest device name, path or text to wait for an input carries.
pub const NAME_LENGTH: usize = 4096;
/// The largest result written to stdout; a larger tree is cut.
pub const INLINE_BYTES: usize = 64 * 1024;

numbered!(
    /// A device's handle in the conversation, which `open` answers: `m` and
    /// the device's number, given once across the conversation's Hosts
    /// (`mobile.md` § Commands).
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
    /// Which way the device is turned, as `demi-mobile` last set it.
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
    /// What `record` does.
    pub enum RecordAction {
        Start = "start",
        Stop = "stop",
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

/// One element of the screen's tree (`mobile.md` § Commands): its
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
    "appearance" => Appearance(AppearanceInput) -> AppearanceResult;
    "wait" => Wait(WaitInput) -> WaitResult;
    "record" => Record(RecordInput) -> RecordResult;
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
    #[garde(length(chars, min = 1, max = NAME_LENGTH))]
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
        #[garde(length(chars, min = 1, max = NAME_LENGTH))]
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
    /// Whether elements were left out to fit the result.
    pub truncated: bool,
}

device_input! {
    /// `tap`: a tap at a point or on an element.
    pub struct TapInput {
        /// An element ref from the last inspect, or x and y in screenshot pixels
        #[garde(length(min = 1, max = 2), inner(length(chars, min = 1, max = 32)))]
        pub at: Vec<String>,
        /// Hold for a second, a long press
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub long: Option<bool>,
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
        /// x1 y1 x2 y2 in screenshot pixels, or an element ref and up, down, left or right
        #[garde(length(min = 2, max = 4), inner(length(chars, min = 1, max = 32)))]
        pub at: Vec<String>,
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
        #[garde(length(chars, min = 1, max = TEXT_LENGTH))]
        pub text: String,
        /// Tap this element ref first, then type into it
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ElementRef")]
        #[garde(skip)]
        pub into: Option<ElementRef>,
        /// Set the field's text at once instead of adding to it; needs --into
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub replace: Option<bool>,
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
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ElementSummary")]
    pub element: Option<ElementSummary>,
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
    pub orientation: Orientation,
    /// The screenshot's size in the new orientation.
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
        /// Wait until an element's label or value contains this text
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1, max = NAME_LENGTH))]
        pub text: Option<String>,
        /// Wait until this element ref is on the screen
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ElementRef")]
        #[garde(skip)]
        pub element: Option<ElementRef>,
        /// Wait until this element ref is gone from the screen
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ElementRef")]
        #[garde(skip)]
        pub gone: Option<ElementRef>,
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
    /// `record`: starts recording what the device shows, or stops it and
    /// writes the MP4.
    pub struct RecordInput {
        /// start or stop
        #[garde(skip)]
        pub action: RecordAction,
        /// For stop: a new output file on this Host; without it the MP4 goes to stdout
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1, max = NAME_LENGTH))]
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
pub struct RecordResult {
    pub device: DeviceId,
    /// Whether it records now.
    pub recording: bool,
    /// The MP4 `stop --output` saved.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub path: Option<String>,
    /// How long the recording is, once stopped.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "u64", range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub duration_ms: Option<u64>,
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
        TooManyDevices = "too_many_devices",
        /// The ref names no element of the device's last `inspect`.
        StaleRef = "stale_ref",
        Timeout = "timeout",
        /// This Host cannot run the platform's simulators, or not yet.
        Unavailable = "unavailable",
        NotRecording = "not_recording",
        Recording = "recording",
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
/// devices change (`plugins.md` § Messages from its command package):
/// `{"devices":"changed"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct MobileMessage {
    #[garde(skip)]
    pub devices: DevicesChange,
}
