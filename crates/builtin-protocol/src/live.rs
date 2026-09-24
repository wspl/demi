//! The live view's protocol (`live-view.md` § The stream): what the page and
//! the live view module say to each other over a user stream. The stream
//! carries bytes without boundaries, so each message is one frame: a
//! four-byte big-endian length of the rest, a one-byte kind, then its
//! payload. The page and the module ship in the same release; there is no
//! version.

use std::sync::LazyLock;

use demi_core::Nullable;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{
    DecodeError,
    browser::{BrowserCreatedBy, BrowserViewport, DialogType, TabId},
};

/// The declared operation that serves a view (`native-runtime.md` § User
/// streams).
pub const OPERATION: &str = "browser.live";

/// The view's arguments: none; the page and the module speak over the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveInput {}

/// A frame's kind, the byte after its length.
pub const CONTROL_FRAME: u8 = 1;
/// A video frame: [`VideoHeader`], then H.264 Annex B data.
pub const VIDEO_FRAME: u8 = 2;
/// A chosen file's bytes: [`FileHeader`], then the data.
pub const FILE_FRAME: u8 = 3;
/// The largest frame after its length: a paste's text and HTML, or a key frame.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// A file frame's largest data.
pub const FILE_CHUNK_BYTES: usize = 64 * 1024;
/// How often the module speaks at least, so a still page is told from a stall.
pub const HEARTBEAT_MS: u64 = 250;
/// Silence after which the page shows the stream as stalled and stops sending input.
pub const STALL_MS: u64 = 1000;

/// A notice's code when this Host cannot capture the watched tab.
pub const CAPTURE_UNAVAILABLE: &str = "capture_unavailable";
/// A notice's code when the watched tab's capture failed; the next picture ends it.
pub const CAPTURE_FAILED: &str = "capture_failed";

const TEXT_BYTES: usize = 1_000_000;
const HTML_BYTES: usize = 4_000_000;

/// A video frame's header, big-endian: the tab ID (24 ASCII bytes), the
/// stream generation (u32), the sequence number (u32), flags (u8, 1 = key
/// frame), three reserved bytes, the timestamp in microseconds (f64), and
/// the picture's width and height in pixels (u16 each).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoHeader {
    pub tab: TabId,
    pub generation: u32,
    pub sequence: u32,
    pub key: bool,
    pub timestamp: f64,
    pub width: u16,
    pub height: u16,
}

impl VideoHeader {
    pub const BYTES: usize = 48;

    /// Appends the header to `bytes`.
    pub fn write(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(self.tab.as_str().as_bytes());
        bytes.extend_from_slice(&self.generation.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.push(u8::from(self.key));
        bytes.extend_from_slice(&[0; 3]);
        bytes.extend_from_slice(&self.timestamp.to_be_bytes());
        bytes.extend_from_slice(&self.width.to_be_bytes());
        bytes.extend_from_slice(&self.height.to_be_bytes());
    }
}

/// A file frame's header, big-endian: the upload (u32) and the file's index
/// in it (u32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub upload: u32,
    pub file: u32,
}

impl FileHeader {
    pub const BYTES: usize = 8;

    /// Splits a file frame's payload into its header and data.
    pub fn split(payload: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let Some((header, data)) = payload.split_first_chunk::<{ Self::BYTES }>() else {
            return Err(DecodeError::Invalid("a file frame is shorter than its header".into()));
        };
        let (upload, file) = header.split_at(4);
        let header = Self {
            upload: u32::from_be_bytes(upload.try_into().expect("four bytes")),
            file: u32::from_be_bytes(file.try_into().expect("four bytes")),
        };
        Ok((header, data))
    }
}

/// What a control token looks like: a UUID in lowercase, as
/// `crypto.randomUUID` makes it. It is both the check and the schema.
pub const CONTROL_TOKEN_PATTERN: &str = "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$";

/// A control's identity in its page, as `crypto.randomUUID` makes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String")]
#[schemars(extend("pattern" = CONTROL_TOKEN_PATTERN))]
pub struct ControlToken(String);

impl ControlToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ControlToken {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        static PATTERN: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(CONTROL_TOKEN_PATTERN).expect("the control token pattern compiles"));
        if PATTERN.is_match(&value) {
            Ok(Self(value))
        } else {
            Err(format!("{value:?} is not a control token"))
        }
    }
}

closed_set! {
    /// The viewer's platform, which decides how its keys map on the Host.
    pub enum Platform {
        Mac = "mac",
        Windows = "windows",
        Linux = "linux",
        Other = "other",
    }
}

closed_set! {
    /// A viewport mode the page may choose; `custom` is the agent's.
    pub enum ViewerMode {
        Web = "web",
        Mobile = "mobile",
    }
}

closed_set! {
    pub enum PointerAction {
        Move = "move",
        Down = "down",
        Up = "up",
    }
}

closed_set! {
    pub enum PointerButton {
        None = "none",
        Left = "left",
        Middle = "middle",
        Right = "right",
    }
}

closed_set! {
    pub enum KeyAction {
        Down = "down",
        Up = "up",
    }
}

closed_set! {
    /// A native form control an observer reports (`live-view.md` § Input).
    pub enum ControlKind {
        Select = "select",
        Date = "date",
        Month = "month",
        Week = "week",
        Time = "time",
        DatetimeLocal = "datetime-local",
        Color = "color",
        Suggestions = "suggestions",
        File = "file",
    }
}

closed_set! {
    /// Why the browser ended: it stopped, or the conversation released it.
    pub enum EndReason {
        BrowserEnded = "browser_ended",
        Released = "released",
    }
}

/// A file the viewer chose for an upload; its bytes follow as file frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadFile {
    #[garde(length(chars, min = 1, max = 255), pattern(r"^[^/\\\x00]+$"))]
    pub name: String,
    #[garde(length(chars, max = 200))]
    pub mime_type: String,
    #[garde(skip)]
    pub size: u64,
}

/// What the page sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveViewerMessage {
    /// First: the viewer's platform.
    Hello {
        #[garde(skip)]
        platform: Platform,
    },
    /// The panel's size in CSS pixels and the viewer's screen.
    #[serde(rename_all = "camelCase")]
    Panel {
        #[garde(range(min = 1, max = 4096))]
        width: u32,
        #[garde(range(min = 1, max = 4096))]
        height: u32,
        #[garde(range(min = 0.5, max = 4.0))]
        device_pixel_ratio: f64,
        #[garde(range(min = 1, max = 4096))]
        screen_width: u32,
        #[garde(range(min = 1, max = 4096))]
        screen_height: u32,
    },
    /// The tab the view shows, or none.
    Watch {
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<TabId>")]
        #[garde(skip)]
        tab: Option<TabId>,
    },
    Mode {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        mode: ViewerMode,
    },
    #[serde(rename_all = "camelCase")]
    Pointer {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        action: PointerAction,
        #[garde(range(min = 0.0, max = 4096.0))]
        x: f64,
        #[garde(range(min = 0.0, max = 4096.0))]
        y: f64,
        #[garde(skip)]
        button: PointerButton,
        #[garde(range(max = 31))]
        buttons: u8,
        #[garde(range(max = 3))]
        click_count: u8,
        /// The modifier keys held, as CDP numbers them: Alt 1, Control 2,
        /// Meta 4, Shift 8.
        #[garde(range(max = 15))]
        modifiers: u8,
    },
    #[serde(rename_all = "camelCase")]
    Wheel {
        #[garde(skip)]
        tab: TabId,
        #[garde(range(min = 0.0, max = 4096.0))]
        x: f64,
        #[garde(range(min = 0.0, max = 4096.0))]
        y: f64,
        #[garde(range(min = -10_000.0, max = 10_000.0))]
        delta_x: f64,
        #[garde(range(min = -10_000.0, max = 10_000.0))]
        delta_y: f64,
        #[garde(range(max = 15))]
        modifiers: u8,
    },
    #[serde(rename_all = "camelCase")]
    Key {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        action: KeyAction,
        #[garde(length(chars, max = 64))]
        key: String,
        #[garde(length(chars, max = 64))]
        code: String,
        #[garde(skip)]
        key_code: u8,
        #[garde(range(max = 15))]
        modifiers: u8,
        #[garde(skip)]
        repeat: bool,
        #[garde(range(max = 3))]
        location: u8,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, min = 1, max = 16))]
        text: Option<String>,
        #[garde(skip)]
        alt_graph: bool,
    },
    /// Committed text: an input method's result.
    Text {
        #[garde(skip)]
        tab: TabId,
        #[garde(length(chars, max = 20_000))]
        text: String,
    },
    /// An input method's text being composed.
    Composition {
        #[garde(skip)]
        tab: TabId,
        #[garde(length(chars, max = 20_000))]
        text: String,
    },
    Paste {
        #[garde(skip)]
        tab: TabId,
        #[garde(length(chars, max = TEXT_BYTES))]
        text: String,
        #[garde(length(chars, max = HTML_BYTES))]
        html: String,
    },
    /// A choice in a native control, for the revision the viewer saw.
    Choice {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        token: ControlToken,
        #[garde(skip)]
        revision: u64,
        #[garde(length(chars, max = 10_000))]
        value: String,
        #[garde(length(max = 1000))]
        indices: Vec<u32>,
    },
    /// Files chosen for a file input; their bytes follow as file frames.
    Upload {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        token: ControlToken,
        #[garde(skip)]
        revision: u64,
        #[garde(skip)]
        upload: u32,
        #[garde(length(max = 100), dive)]
        files: Vec<UploadFile>,
    },
    Dialog {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        accept: bool,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(chars, max = 2000))]
        text: Option<String>,
    },
    /// The page showed this frame; the module paces itself by these.
    #[serde(rename_all = "camelCase")]
    Ack {
        #[garde(skip)]
        generation: u32,
        #[garde(skip)]
        sequence: u32,
        #[garde(skip)]
        decode_queue: u32,
    },
    /// The decoder lost the stream; the next frame must be a key frame.
    Keyframe {
        #[garde(skip)]
        generation: u32,
    },
    /// Release every key and button this viewer holds.
    Release {},
}

impl LiveViewerMessage {
    /// Decodes a control frame's payload from the page.
    pub fn decode(payload: &[u8]) -> Result<Self, DecodeError> {
        crate::decode_slice(payload)
    }
}

/// An option of a native select or suggestion list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveControlOption {
    #[garde(length(chars, max = 2000))]
    pub label: String,
    #[garde(length(chars, max = 2000))]
    pub value: String,
    #[garde(length(chars, max = 2000))]
    pub group: String,
    #[garde(skip)]
    pub disabled: bool,
    #[garde(skip)]
    pub hidden: bool,
    #[garde(skip)]
    pub selected: bool,
}

/// A control's box in viewport CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ControlRect {
    #[garde(skip)]
    pub x: f64,
    #[garde(skip)]
    pub y: f64,
    #[garde(range(min = f64::MIN_POSITIVE))]
    pub width: f64,
    #[garde(range(min = f64::MIN_POSITIVE))]
    pub height: f64,
}

/// A native form control of the watched tab, which the page draws over the
/// picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveControl {
    #[garde(skip)]
    pub token: ControlToken,
    #[garde(skip)]
    pub revision: u64,
    #[garde(skip)]
    pub kind: ControlKind,
    #[garde(length(chars, max = 2000))]
    pub label: String,
    #[garde(length(chars, max = 10_000))]
    pub value: String,
    #[garde(length(chars, max = 100))]
    pub min: String,
    #[garde(length(chars, max = 100))]
    pub max: String,
    #[garde(length(chars, max = 100))]
    pub step: String,
    #[garde(length(chars, max = 1000))]
    pub accept: String,
    #[garde(skip)]
    pub multiple: bool,
    #[garde(skip)]
    pub disabled: bool,
    #[garde(skip)]
    pub required: bool,
    #[garde(skip)]
    pub size: u32,
    #[garde(length(max = 1000), dive)]
    pub options: Vec<LiveControlOption>,
    #[garde(dive)]
    pub rect: ControlRect,
}

/// A tab as the view lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveTab {
    #[garde(skip)]
    pub id: TabId,
    #[garde(skip)]
    pub title: String,
    #[garde(skip)]
    pub url: String,
    #[garde(dive)]
    pub created_by: BrowserCreatedBy,
    #[garde(dive)]
    pub viewport: BrowserViewport,
}

/// The watched tab's JavaScript dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveDialog {
    #[garde(skip)]
    pub r#type: DialogType,
    #[garde(skip)]
    pub message: String,
    #[garde(skip)]
    pub default_text: String,
}

/// What the module sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveModuleMessage {
    /// The browser's tabs and the one this viewer watches.
    State {
        #[garde(skip)]
        running: bool,
        #[garde(dive)]
        tabs: Vec<LiveTab>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<TabId>")]
        #[garde(skip)]
        watched: Option<TabId>,
    },
    /// Video frames of this generation follow, starting with a key frame.
    Stream {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        generation: u32,
        #[garde(range(min = 1))]
        width: u32,
        #[garde(range(min = 1))]
        height: u32,
    },
    Heartbeat {},
    Cursor {
        #[garde(skip)]
        tab: TabId,
        #[garde(length(chars, max = 200))]
        cursor: String,
        #[garde(skip)]
        editable: bool,
    },
    Controls {
        #[garde(skip)]
        tab: TabId,
        #[garde(length(max = 100), dive)]
        controls: Vec<LiveControl>,
    },
    /// Text the watched tab copied shortly after this viewer's input.
    Clipboard {
        #[garde(length(chars, max = TEXT_BYTES))]
        text: String,
    },
    Dialog {
        #[garde(skip)]
        tab: TabId,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<LiveDialog>")]
        #[garde(dive)]
        dialog: Option<LiveDialog>,
    },
    /// Whether a choice or an upload reached its control.
    Choice {
        #[garde(skip)]
        token: ControlToken,
        #[garde(skip)]
        accepted: bool,
    },
    /// Something the viewer asked for failed; the stream goes on.
    Notice {
        /// A browser failure's code, [`CAPTURE_UNAVAILABLE`] or [`CAPTURE_FAILED`].
        #[garde(skip)]
        code: String,
        #[garde(skip)]
        message: String,
    },
    /// The browser ended; the stream ends after this.
    Ended {
        #[garde(skip)]
        reason: EndReason,
    },
}
