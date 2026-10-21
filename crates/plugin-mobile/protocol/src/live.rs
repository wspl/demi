//! The simulators' live view protocol: what the page and `demi-mobile` say to
//! each other over a `mobile` user stream about its one device, beside the
//! live view's own frames, pictures and input (`live-view-protocol`).
//!
//! Each open device a page shows has a stream of its own, whose only state is
//! whether it is looked at
//! (`docs/decisions/0017-each-device-streams-on-its-own.md`).
//!
//! The page and the service ship in the same release; there is no version.

use demi_shared_types::Nullable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_live_view_protocol::{
    LiveActivity, LiveHeartbeat, LiveInputs, LiveKeyframe, LiveNotice, LiveRelease,
    ModuleMessage, Platform, ViewerInput,
};

use crate::mobile::{DeviceId, Orientation, MobilePlatform};

/// The declared operation that serves a view (`crates/runner-jobs/src/commands/streams.rs`).
pub const OPERATION: &str = "mobile.live";

/// A notice's code when the device's capture failed; the next picture ends
/// it.
pub const CAPTURE_FAILED: &str = "capture_failed";
/// A notice's code when this Host cannot capture the device.
pub const CAPTURE_UNAVAILABLE: &str = "capture_unavailable";
/// A notice's code when the viewer's input did not reach the device.
pub const INPUT_FAILED: &str = "input_failed";
/// A notice's code when the viewer's text, which a US keyboard cannot type,
/// found no focused field to add it to; the notice's message is that text,
/// the viewer's own, which the page quotes.
pub const NO_FOCUSED_FIELD: &str = "no_focused_field";

/// The view's arguments: none; the page names the stream's device in its
/// hello.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveInput {}

/// What the page sends on one device's stream: the live view's own
/// messages, and whether the stream is looked at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveViewerMessage {
    /// First, and once: the viewer's platform, and the device the stream
    /// carries, its only one.
    Hello {
        #[garde(skip)]
        platform: Platform,
        #[garde(skip)]
        device: DeviceId,
    },
    /// Whether the stream is looked at: whether a view of it shows in a
    /// visible page. A stream starts not looked at; while it is not, the
    /// service sends nothing.
    Looked {
        #[garde(skip)]
        looked: bool,
    },
    /// The viewer's input, in the points of the screen as the picture shows
    /// it upright.
    Input {
        #[garde(dive)]
        input: ViewerInput,
    },
    Keyframe(#[garde(dive)] LiveKeyframe),
    Release(#[garde(dive)] LiveRelease),
    /// The view lays the device out in `width` × `height` device pixels, at
    /// `scale` device pixels per CSS pixel, and asks for its frame at that
    /// size, as the device is turned now and again as it turns; asked only
    /// for a frame the page does not keep.
    Frame {
        #[garde(range(min = 1, max = 16384))]
        width: u32,
        #[garde(range(min = 1, max = 16384))]
        height: u32,
        #[garde(range(min = 0.25, max = 8.0))]
        scale: f64,
    },
    /// The viewer's Wake: the screen turns on, and stays on if it is.
    Wake {},
    /// The view shows the screen at `width` × `height` device pixels, which
    /// its pictures need no more than; sent again when that changes.
    Shown {
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

/// A button the drawn frame shows: where it starts along its edge and how
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

/// A point of a corner's shape, from the corner of the screen's box: `x`
/// across from its upright edge, `y` down from its level one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveCornerPoint {
    #[garde(range(min = -4096.0, max = 4096.0))]
    pub x: f64,
    #[garde(range(min = -4096.0, max = 4096.0))]
    pub y: f64,
}

/// One rounded corner of the screen, drawn
/// as if it were the top left one: the view mirrors it into its place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveCorner {
    /// A quarter circle of `radius`; none at 0, a square corner.
    Round {
        #[garde(range(min = 0.0, max = 4096.0))]
        radius: f64,
    },
    /// The screen's edge across the corner, as the device's system UI
    /// shapes it: a line through `points`, from where it leaves the
    /// upright edge to where it meets the level one.
    Curve {
        #[garde(length(min = 2, max = 512), dive)]
        points: Vec<LiveCornerPoint>,
    },
}

/// The screen's four corners, as the picture shows them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveCorners {
    #[garde(dive)]
    pub top_left: LiveCorner,
    #[garde(dive)]
    pub top_right: LiveCorner,
    #[garde(dive)]
    pub bottom_right: LiveCorner,
    #[garde(dive)]
    pub bottom_left: LiveCorner,
}

impl LiveCorners {
    /// Four quarter circles of `radius`.
    pub fn round(radius: f64) -> Self {
        let corner = LiveCorner::Round { radius };
        Self {
            top_left: corner.clone(),
            top_right: corner.clone(),
            bottom_right: corner.clone(),
            bottom_left: corner,
        }
    }
}

/// A device's frame, turned as the picture
/// is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveFrame {
    /// The frame Xcode or Android Studio installs for the model, drawn: a
    /// PNG of `width` × `height` pixels, transparent where `screen` shows
    /// the picture.
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
        /// The device's silhouette, a grey PNG of the frame's size, white
        /// where the picture may lie: the picture lies under the frame,
        /// bounded by it, and shows only through the frame's screen. None
        /// where the frame covers the picture's corners itself.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        #[garde(skip)]
        silhouette: Option<String>,
    },
    /// No frame is installed: the view draws the device itself around the
    /// screen, concentric with its corners, in the screen's points, with
    /// its buttons.
    #[serde(rename_all = "camelCase")]
    Drawn {
        #[garde(dive)]
        corners: LiveCorners,
        #[garde(dive)]
        buttons: Vec<LiveButtonMark>,
    },
}

/// The stream's device as it is now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveDevice {
    #[garde(skip)]
    pub name: String,
    #[garde(skip)]
    pub platform: MobilePlatform,
    #[garde(skip)]
    pub os: String,
    /// The model it simulates, such as iPhone 17 Pro, as its driver reads
    /// it; none when the driver knows none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub model: Option<String>,
    /// Whether a recording of it runs, whoever started it.
    #[garde(skip)]
    pub recording: bool,
    /// Its orientation as its screen reports it when the state is sent.
    #[garde(skip)]
    pub orientation: Orientation,
}

/// What the service sends on one device's stream, only while the stream is
/// looked at, but for [`LiveModuleMessage::Gone`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveModuleMessage {
    /// The device as it is, once the stream is looked at and again as it
    /// changes.
    State {
        #[garde(dive)]
        device: LiveDevice,
    },
    /// The device is not open, or closed: the stream ends after this.
    Gone {},
    /// Video frames of this generation follow, starting with a key frame:
    /// `width` × `height` pixels of the screen turned upright, whose
    /// points are `points_width` × `points_height`, at `scale` of its
    /// pixels.
    #[serde(rename_all = "camelCase")]
    Stream {
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
    /// What input the device takes: touches.
    Inputs {
        #[garde(dive)]
        inputs: LiveInputs,
    },
    /// Whether the agent uses the device, and where it last tapped, in its
    /// points.
    Activity {
        #[garde(dive)]
        activity: LiveActivity,
    },
    /// The device's frame for the size the view asked, in `orientation`,
    /// sent again when the device turns, with the size of its screen in
    /// points as the frame shows it.
    ///
    /// `width` × `height` device pixels at `scale` are the area of the
    /// view's request it was drawn for, which the view keeps it by: a
    /// request the view made since may still be under way.
    #[serde(rename_all = "camelCase")]
    Frame {
        #[garde(range(min = 1, max = 16384))]
        width: u32,
        #[garde(range(min = 1, max = 16384))]
        height: u32,
        #[garde(range(min = 0.25, max = 8.0))]
        scale: f64,
        #[garde(skip)]
        orientation: Orientation,
        #[garde(range(min = 1.0, max = 4096.0))]
        points_width: f64,
        #[garde(range(min = 1.0, max = 4096.0))]
        points_height: f64,
        /// None while the device still reads what its frame needs, such as
        /// an Android screen's corners; the frame follows in another
        /// `Frame`, and the view shows the device once it has it.
        #[garde(dive)]
        frame: Option<LiveFrame>,
    },
    /// Whether the screen is on, once the stream is looked at and again
    /// each time that changes.
    Screen {
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
