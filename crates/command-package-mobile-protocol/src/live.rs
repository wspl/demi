//! The simulators' live view protocol (`mobile.md` § The live view): what
//! the page and `demi-mobile` say to each other over the `mobile` user
//! stream about the conversation's devices, beside the live view's own
//! frames, pictures and input (`live-view-protocol`). The page and the
//! service ship in the same release; there is no version.

use demi_shared_types::Nullable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_live_view_protocol::{
    LiveActivity, LiveHeartbeat, LiveHello, LiveInputs, LiveKeyframe, LiveNotice,
    LiveRelease, ModuleMessage, ViewerInput,
};

use crate::mobile::{DeviceId, Orientation, MobilePlatform};

/// The declared operation that serves a view (`native-runtime.md` § User
/// streams).
pub const OPERATION: &str = "mobile.live";

/// A notice's code when the watched device's capture failed; the next
/// picture ends it.
pub const CAPTURE_FAILED: &str = "capture_failed";
/// A notice's code when this Host cannot capture the watched device.
pub const CAPTURE_UNAVAILABLE: &str = "capture_unavailable";
/// A notice's code when the viewer's input did not reach the device.
pub const INPUT_FAILED: &str = "input_failed";
/// A notice's code when the viewer's text, which a US keyboard cannot type,
/// found no focused field to add it to; the notice's message is that text,
/// the viewer's own, which the page quotes.
pub const NO_FOCUSED_FIELD: &str = "no_focused_field";

/// The view's arguments: none; the page and the service speak over the
/// stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveInput {}

/// What the page sends: the live view's own messages and the device it
/// watches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveViewerMessage {
    /// First: the viewer's platform.
    Hello(#[garde(dive)] LiveHello),
    /// The device the view shows, or none.
    Watch {
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<DeviceId>")]
        #[garde(skip)]
        device: Option<DeviceId>,
    },
    /// The viewer's input to `device`, in the points of its screen as the
    /// picture shows it upright.
    Input {
        #[garde(skip)]
        device: DeviceId,
        #[garde(dive)]
        input: ViewerInput,
    },
    Keyframe(#[garde(dive)] LiveKeyframe),
    Release(#[garde(dive)] LiveRelease),
    /// The view lays `device` out in `width` × `height` device pixels, at
    /// `scale` device pixels per CSS pixel, and asks for its frame at that
    /// size; asked again when its laid-out size changes.
    Frame {
        #[garde(skip)]
        device: DeviceId,
        #[garde(range(min = 1, max = 16384))]
        width: u32,
        #[garde(range(min = 1, max = 16384))]
        height: u32,
        #[garde(range(min = 0.25, max = 8.0))]
        scale: f64,
    },
    /// The viewer's Wake: `device`'s screen turns on, and stays on if it
    /// is (`mobile.md` § The live view).
    Wake {
        #[garde(skip)]
        device: DeviceId,
    },
    /// The view shows `device`'s screen at `width` × `height` device
    /// pixels, which its pictures need no more than; sent again when that
    /// changes.
    Shown {
        #[garde(skip)]
        device: DeviceId,
        #[garde(range(min = 1, max = 16384))]
        width: u32,
        #[garde(range(min = 1, max = 16384))]
        height: u32,
    },
}

/// A rectangle of a drawn frame, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveRect {
    #[garde(range(min = 0.0))]
    pub x: f64,
    #[garde(range(min = 0.0))]
    pub y: f64,
    #[garde(range(min = 1.0))]
    pub width: f64,
    #[garde(range(min = 1.0))]
    pub height: f64,
}

/// The edge of the device a button stands on, as the picture shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LiveSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// A button the outline marks: where it starts along its edge and how
/// long it is, as fractions of the edge's length, from its top or its left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveButtonMark {
    #[garde(skip)]
    pub side: LiveSide,
    #[garde(range(min = 0.0, max = 1.0))]
    pub start: f64,
    #[garde(range(min = 0.0, max = 1.0))]
    pub length: f64,
}

/// A device's frame (`mobile.md` § The live view), turned as the picture
/// is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveFrame {
    /// The frame Xcode installs for the model, drawn: a PNG of `width` ×
    /// `height` pixels, transparent where `screen` shows the picture.
    #[serde(rename_all = "camelCase")]
    Image {
        /// The PNG, base64.
        #[garde(length(min = 1))]
        png: String,
        #[garde(range(min = 1))]
        width: u32,
        #[garde(range(min = 1))]
        height: u32,
        #[garde(dive)]
        screen: LiveRect,
        /// The radius of the screen's corners, in the frame's pixels, which
        /// the picture is rounded to.
        #[garde(range(min = 0.0, max = 4096.0))]
        corner_radius: f64,
    },
    /// No frame is installed: the view draws an outline around the screen,
    /// whose corners have this radius in the screen's points, with its
    /// buttons as marks.
    #[serde(rename_all = "camelCase")]
    Outline {
        #[garde(range(min = 0.0, max = 4096.0))]
        corner_radius: f64,
        #[garde(dive)]
        buttons: Vec<LiveButtonMark>,
    },
}

/// An open device as the view lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveDevice {
    #[garde(skip)]
    pub id: DeviceId,
    #[garde(skip)]
    pub name: String,
    #[garde(skip)]
    pub platform: MobilePlatform,
    #[garde(skip)]
    pub os: String,
    /// Whether a recording of it runs, whoever started it.
    #[garde(skip)]
    pub recording: bool,
}

/// What the service sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveModuleMessage {
    /// The conversation's open devices on this Host and the one this
    /// viewer watches.
    State {
        #[garde(dive)]
        devices: Vec<LiveDevice>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<DeviceId>")]
        #[garde(skip)]
        watched: Option<DeviceId>,
    },
    /// Video frames of this generation follow, starting with a key frame:
    /// `width` × `height` pixels of the screen turned upright, whose
    /// points are `points_width` × `points_height`, at `scale` of its
    /// pixels.
    #[serde(rename_all = "camelCase")]
    Stream {
        #[garde(skip)]
        device: DeviceId,
        #[garde(skip)]
        generation: u32,
        #[garde(range(min = 1))]
        width: u32,
        #[garde(range(min = 1))]
        height: u32,
        #[garde(range(min = 1.0, max = 4096.0))]
        points_width: f64,
        #[garde(range(min = 1.0, max = 4096.0))]
        points_height: f64,
        #[garde(skip)]
        orientation: Orientation,
        #[garde(range(min = f64::MIN_POSITIVE, max = 1.0))]
        scale: f64,
    },
    /// What input the watched device takes: touches.
    Inputs {
        #[garde(skip)]
        device: DeviceId,
        #[garde(dive)]
        inputs: LiveInputs,
    },
    /// Whether the agent uses the watched device, and where it last tapped,
    /// in its points.
    Activity {
        #[garde(skip)]
        device: DeviceId,
        #[garde(dive)]
        activity: LiveActivity,
    },
    /// The watched device's frame for the size the view asked, sent again
    /// when the device turns, with the size of its screen in points as the
    /// frame shows it, so the view lays the screen out, and says the size
    /// it shows it at, before the first picture.
    #[serde(rename_all = "camelCase")]
    Frame {
        #[garde(skip)]
        device: DeviceId,
        #[garde(skip)]
        orientation: Orientation,
        #[garde(range(min = 1.0, max = 4096.0))]
        points_width: f64,
        #[garde(range(min = 1.0, max = 4096.0))]
        points_height: f64,
        #[garde(dive)]
        frame: LiveFrame,
    },
    /// Whether the watched device's screen is on, as it watches it and
    /// again each time that changes.
    Screen {
        #[garde(skip)]
        device: DeviceId,
        #[garde(skip)]
        on: bool,
    },
    Heartbeat(#[garde(dive)] LiveHeartbeat),
    /// Something the viewer asked for failed; the stream goes on:
    /// [`CAPTURE_FAILED`], [`CAPTURE_UNAVAILABLE`], [`INPUT_FAILED`] or
    /// [`NO_FOCUSED_FIELD`].
    Notice(#[garde(dive)] LiveNotice),
}

impl ModuleMessage for LiveModuleMessage {
    fn heartbeat(heartbeat: LiveHeartbeat) -> Self {
        Self::Heartbeat(heartbeat)
    }

    fn notice(notice: LiveNotice) -> Self {
        Self::Notice(notice)
    }
}
