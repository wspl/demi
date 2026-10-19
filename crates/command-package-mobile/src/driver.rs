//! One driver per platform, behind one interface (`mobile.md` § Drivers):
//! the Host's simulators, boot and shutdown, and for an open device its
//! frames as H.264, its input, its UI tree, its screen's size and
//! orientation. Every coordinate here is a point of the screen turned
//! upright, as the picture shows it; the commands map them to screenshot
//! pixels.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::future::BoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use demi_command_package_mobile_protocol::{
    live::LiveCorners,
    mobile::{Appearance, Orientation, MobilePlatform, PowerState},
};
use demi_live_view::stream::CaptureEvent;

/// Why a driver could not do what it was asked.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DriverError {
    /// The device is gone: shut down, or its helper ended.
    #[error("{0}")]
    Lost(String),
    #[error("{0}")]
    Failed(String),
    /// Text a US keyboard cannot type found no focused field to go into,
    /// which an iPhone needs (`mobile.md` § Commands): the text.
    #[error("{0:?} has no keys on a US keyboard, and there is no focused text field to add it to")]
    NoFocusedField(String),
}

pub type Result<T> = std::result::Result<T, DriverError>;

/// A simulator of the Host, as its platform's tools name it.
#[derive(Debug, Clone, PartialEq)]
pub struct Simulator {
    /// The `simctl` id or AVD name.
    pub native_id: String,
    pub name: String,
    pub model: String,
    pub os: String,
    pub state: PowerState,
    /// The mark of its current boot, which a later boot changes, such as
    /// when the boot began in seconds; none while it is shut down
    /// (`mobile.md` § Lifetime).
    pub boot: Option<u64>,
}

/// A point or a size of the upright screen, in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Points {
    pub x: f64,
    pub y: f64,
}

/// A rectangle of the upright screen, in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn centre(&self) -> Points {
        Points {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }

    /// The smallest rectangle around both.
    pub fn union(&self, other: &Rect) -> Rect {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
}

/// The device's screen as it shows now, turned upright: its size in points
/// and in pixels, and the size its screenshots are drawn at, which every
/// coordinate a command takes or prints is in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub orientation: Orientation,
    pub points: Points,
    pub pixels: (u32, u32),
    /// The screenshot's size: the pixels fitted into
    /// [`SCREENSHOT_LONG_SIDE`](demi_command_package_mobile_protocol::mobile::SCREENSHOT_LONG_SIDE)
    /// by the rule of the platform that draws it.
    pub screenshot: (u32, u32),
}

/// One element of the screen's tree as the platform's accessibility
/// service reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawElement {
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    pub identifier: Option<String>,
    pub frame: Rect,
    pub enabled: Option<bool>,
    pub selected: Option<bool>,
    pub focused: Option<bool>,
    /// A secure field: its value is never read.
    pub secure: bool,
    /// A key of the software keyboard, which `inspect` shows as one
    /// Keyboard element unless asked for its keys.
    pub keyboard_key: bool,
    pub children: Vec<RawElement>,
}

/// A finger's phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
}

/// A hardware button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Home,
    Lock,
    VolumeUp,
    VolumeDown,
    /// Android's Back.
    Back,
}

/// A key of a hardware keyboard, by its USB HID usage, with the modifiers
/// held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    pub usage: u32,
    pub down: bool,
}

/// A PNG of the upright screen.
#[derive(Debug, Clone)]
pub struct Png {
    pub bytes: Bytes,
    pub width: u32,
    pub height: u32,
}

/// A recording that stopped: its MP4 at the path its start named.
#[derive(Debug, Clone)]
pub struct Recording {
    pub duration_ms: u64,
}

/// A rectangle of a picture, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PixelRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where the view lays a device's frame out: `width` × `height` device
/// pixels at `scale` of them per point, the page's pixel ratio, which the
/// frame is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameArea {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// The device's frame (`mobile.md` § The live view), turned as its picture
/// is turned upright.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// The frame the platform installs for the model, drawn as a PNG that is
    /// transparent where `screen` shows the picture.
    Image {
        png: Bytes,
        width: u32,
        height: u32,
        screen: PixelRect,
        /// The screen's corners, in the frame's pixels.
        corners: LiveCorners,
    },
    /// The platform installs none: the view draws the frame itself,
    /// around a screen with these corners, in the screen's points.
    Drawn { corners: LiveCorners },
}

/// A running capture of the device's screen for the live view; dropping it
/// stops it.
pub trait DeviceCapture: Send + 'static {
    fn next(&mut self) -> BoxFuture<'_, Option<CaptureEvent>>;
    /// The viewers' windows have room for `window` more frames past
    /// `sequence` (`demi_live_view::stream::Capture::ack`): the newest
    /// picture waits until then.
    fn ack(&self, sequence: u32, window: u32);
    fn key_frame(&self);
    fn encoding(&self, bitrate: u32, fps: u32);
}

/// What a driver tells of a boot it starts, which the device table records
/// (`mobile.md` § Lifetime): the simulator is Demi's from the moment its
/// boot begins, so one whose boot fails halfway is still Demi's to shut
/// down.
pub trait Booting: Send + Sync {
    /// The boot began, with this mark (`Simulator::boot`).
    fn began(&self, boot: u64);
    /// A boot that failed was undone: the simulator is shut down again.
    fn undone(&self);
}

/// A platform's simulators.
pub trait Driver: Send + Sync + 'static {
    fn platform(&self) -> MobilePlatform;
    /// Why this Host cannot run the platform's simulators, or none.
    fn unavailable(&self) -> Option<String>;
    fn list(&self) -> BoxFuture<'_, Result<Vec<Simulator>>>;
    /// Boots a shut-down simulator without a window, telling `booting` as
    /// the boot begins; one that fails is shut down again, and `booting`
    /// hears that it was.
    fn boot<'a>(&'a self, native_id: &'a str, booting: &'a dyn Booting) -> BoxFuture<'a, Result<()>>;
    fn shutdown<'a>(&'a self, native_id: &'a str) -> BoxFuture<'a, Result<()>>;
    /// Attaches a booted simulator for `conversation`: turned portrait
    /// where the platform cannot say how it is turned, as iOS cannot;
    /// as it is where it can, as Android's emulator can.
    fn attach<'a>(
        &'a self,
        conversation: &'a str,
        native_id: &'a str,
    ) -> BoxFuture<'a, Result<Arc<dyn Device>>>;
}

/// An open device: what the commands and the live view do with it.
pub trait Device: Send + Sync + 'static {
    /// The screen as it shows now; it changes with each rotation.
    fn screen(&self) -> watch::Receiver<Screen>;
    /// Counts the pictures the device drew: a still screen draws none, so
    /// a count that stays tells a settled picture.
    fn drawn(&self) -> watch::Receiver<u64>;
    /// Cancelled once the device is gone.
    fn ended(&self) -> CancellationToken;
    /// Whether the screen is on (`mobile.md` § The live view): as an
    /// emulator reports its display, or, for a simulator, which reports
    /// none, off from a Lock pressed while it was on until the next input.
    fn awake(&self) -> watch::Receiver<bool>;
    /// The screen upright, at its screen's screenshot size.
    fn screenshot(&self) -> BoxFuture<'_, Result<Png>>;
    /// The screen's tree, every window, alerts and the keyboard included.
    fn tree(&self) -> BoxFuture<'_, Result<Vec<RawElement>>>;
    fn touch(&self, phase: TouchPhase, at: Points) -> BoxFuture<'_, Result<()>>;
    fn tap(&self, at: Points, seconds: f64) -> BoxFuture<'_, Result<()>>;
    fn swipe(&self, from: Points, to: Points) -> BoxFuture<'_, Result<()>>;
    fn press(&self, button: Button) -> BoxFuture<'_, Result<()>>;
    fn key(&self, key: KeyPress) -> BoxFuture<'_, Result<()>>;
    /// Turns the screen on, and never off.
    fn wake(&self) -> BoxFuture<'_, Result<()>>;
    /// Types `keys` as a US hardware keyboard would, each released again,
    /// Shift included, whatever becomes of the typing.
    fn type_keys<'a>(&'a self, keys: &'a [crate::keys::Typed]) -> BoxFuture<'a, Result<()>>;
    /// Enters `text`, which a US keyboard cannot type, into the focused
    /// field as the platform allows (`mobile.md` § Commands); returns once
    /// the app took it.
    fn insert<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>>;
    /// Sets the text of the field at `at` at once.
    fn set_value<'a>(&'a self, at: Points, text: &'a str) -> BoxFuture<'a, Result<()>>;
    fn orient(&self, orientation: Orientation) -> BoxFuture<'_, Result<()>>;
    fn appearance(&self) -> BoxFuture<'_, Result<Appearance>>;
    fn set_appearance(&self, appearance: Appearance) -> BoxFuture<'_, Result<()>>;
    /// Records the screen into `file`, an MP4 that `record_stop` finishes.
    fn record_start<'a>(&'a self, file: &'a std::path::Path) -> BoxFuture<'a, Result<()>>;
    fn record_stop(&self) -> BoxFuture<'_, Result<Recording>>;
    /// The device's frame, fitted into `area`; none while what it needs is
    /// still being read from the device, which [`Self::frame_changes`]
    /// announces once known.
    fn frame(&self, area: FrameArea) -> BoxFuture<'_, Result<Option<Frame>>>;
    /// Changes once each time the frame [`Self::frame`] gives changes other
    /// than by a turn of the screen; a device whose frame is known at once
    /// never announces one, its channel closed.
    fn frame_changes(&self) -> watch::Receiver<()> {
        watch::channel(()).1
    }
    /// Starts encoding the screen at `size` pixels.
    fn capture(
        &self,
        size: (u32, u32),
        fps: u32,
        bitrate: u32,
    ) -> BoxFuture<'_, Result<Box<dyn DeviceCapture>>>;
    /// Lets go of the device, restoring what attaching changed.
    fn detach(&self) -> BoxFuture<'_, Result<()>>;
    /// The `adb` serial of an open Android emulator, which the agent's own
    /// `adb` commands take; none for a platform without one.
    fn serial(&self) -> Option<String> {
        None
    }
}

/// A platform this Host cannot run, which says why (`mobile.md` § Scope).
pub struct Absent {
    pub platform: MobilePlatform,
    pub reason: String,
}

impl Driver for Absent {
    fn platform(&self) -> MobilePlatform {
        self.platform
    }

    fn unavailable(&self) -> Option<String> {
        Some(self.reason.clone())
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Simulator>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn boot<'a>(&'a self, _native_id: &'a str, _booting: &'a dyn Booting) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Err(DriverError::Failed(self.reason.clone())) })
    }

    fn shutdown<'a>(&'a self, _native_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Err(DriverError::Failed(self.reason.clone())) })
    }

    fn attach<'a>(
        &'a self,
        _conversation: &'a str,
        _native_id: &'a str,
    ) -> BoxFuture<'a, Result<Arc<dyn Device>>> {
        Box::pin(async { Err(DriverError::Failed(self.reason.clone())) })
    }
}
