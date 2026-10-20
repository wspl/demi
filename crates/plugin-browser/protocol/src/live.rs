//! The browser's live view protocol: what the
//! page and the live view module say to each other over a user stream about
//! the browser, beside the live view's own frames, pictures and input
//! (`live-view-protocol`). The page and the module ship in the same release;
//! there is no version.

use std::sync::LazyLock;

use demi_shared_types::{MAX_SAFE_INTEGER, Nullable};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use demi_shared_types::DecodeError;

use demi_live_view_protocol::{
    LiveHeartbeat, LiveHello, LiveKeyframe, LiveNotice, LiveRelease, ModuleMessage,
    ViewerInput,
};

use crate::browser::{BrowserCreatedBy, BrowserViewport, DialogType, TabId};

/// The declared operation that serves a view (`crates/runner-jobs/src/commands/streams.rs`).
pub const OPERATION: &str = "browser.live";

/// The view's arguments: none; the page and the module speak over the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LiveInput {}

/// A chosen file's bytes: [`FileHeader`], then the data. The live view's
/// own frames are `live-view-protocol`'s, kinds 1 to 3.
pub const FILE_FRAME: u8 = 4;
/// A file frame's largest data.
pub const FILE_CHUNK_BYTES: usize = 64 * 1024;

/// A notice's code when this Host cannot capture the watched tab.
pub const CAPTURE_UNAVAILABLE: &str = "capture_unavailable";
/// A notice's code when the watched tab's capture failed; the next picture ends it.
pub const CAPTURE_FAILED: &str = "capture_failed";
/// A notice's code when even recreating the capture extension failed: the
/// module stops trying until the viewer asks again (`Recapture`).
pub const CAPTURE_STOPPED: &str = "capture_stopped";

/// A paste's largest text and HTML, which together stay within the
/// live view's largest frame.
const TEXT_BYTES: usize = 1_000_000;
const HTML_BYTES: usize = 4_000_000;

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
            return Err(DecodeError::Invalid(
                "a file frame is shorter than its header".into(),
            ));
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
pub const CONTROL_TOKEN_PATTERN: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$";

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
        static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(CONTROL_TOKEN_PATTERN).expect("the control token pattern compiles")
        });
        if PATTERN.is_match(&value) {
            Ok(Self(value))
        } else {
            Err(format!("{value:?} is not a control token"))
        }
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
    /// A native form control an observer reports.
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

closed_set! {
    /// Where a download the user started in the page stands.
    pub enum DownloadState {
        InProgress = "inProgress",
        Complete = "complete",
        Canceled = "canceled",
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
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub size: u64,
}

/// What the page sends: the live view's own messages
/// (`live-view-protocol`) and the browser's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum LiveViewerMessage {
    /// First: the viewer's platform.
    Hello(#[garde(dive)] LiveHello),
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
    /// The viewer's input to `tab`, in its CSS pixels.
    Input {
        #[garde(skip)]
        tab: TabId,
        #[garde(dive)]
        input: ViewerInput,
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
        #[garde(range(max = MAX_SAFE_INTEGER))]
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
        #[garde(range(max = MAX_SAFE_INTEGER))]
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
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        #[garde(length(chars, max = 2000))]
        text: Option<String>,
    },
    Keyframe(#[garde(dive)] LiveKeyframe),
    Release(#[garde(dive)] LiveRelease),
    /// Try capturing again after [`CAPTURE_STOPPED`]: the module recreates
    /// the capture extension.
    Recapture {},
}

impl LiveViewerMessage {
    /// Decodes a control frame's payload from the page.
    pub fn decode(payload: &[u8]) -> Result<Self, DecodeError> {
        demi_shared_types::decode_slice(payload)
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
    #[garde(range(max = MAX_SAFE_INTEGER))]
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

/// The most cursor regions a view hears of for a tab, from all its frames.
pub const MAX_CURSOR_REGIONS: usize = 4000;

/// Where on the visible page a cursor applies, in tab CSS pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct CursorRegion {
    #[garde(skip)]
    pub x: f64,
    #[garde(skip)]
    pub y: f64,
    #[garde(range(min = f64::MIN_POSITIVE))]
    pub width: f64,
    #[garde(range(min = f64::MIN_POSITIVE))]
    pub height: f64,
    /// A CSS cursor keyword; `auto` leaves it to the browser, which the
    /// observer resolves for the pointer's point.
    #[garde(length(chars, max = 200))]
    pub cursor: String,
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
    /// Whether the browser loads the tab's top-level page.
    #[garde(skip)]
    pub loading: bool,
    /// Whether the tab's history has a page before the current one.
    #[garde(skip)]
    pub can_go_back: bool,
    /// Whether the tab's history has a page after the current one.
    #[garde(skip)]
    pub can_go_forward: bool,
    /// The page's icon, as the tab list carries it.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(max = crate::browser::FAVICON_LENGTH))]
    pub favicon: Option<String>,
}

/// What lies under the viewer's right click in the watched tab, which the page
/// builds the browser's menu from. The page's own `contextmenu` handler that
/// cancels the event shows its own menu instead, and none of this is sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveMenu {
    /// The click in tab CSS pixels.
    #[garde(skip)]
    pub x: f64,
    #[garde(skip)]
    pub y: f64,
    /// The address of the link under the click; empty when none.
    #[garde(length(chars, max = 8192))]
    pub link: String,
    /// Whether the page has selected text to copy.
    #[garde(skip)]
    pub selection: bool,
    /// Whether the click is in a field that takes text.
    #[garde(skip)]
    pub editable: bool,
}

/// A download the user started in the watched tab, as a browser's download
/// bubble shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveDownload {
    /// The browser's identity of the download.
    #[garde(length(chars, min = 1, max = 100))]
    pub id: String,
    /// The file's name, as the page suggested it.
    #[garde(length(chars, min = 1, max = 255))]
    pub name: String,
    #[garde(skip)]
    pub state: DownloadState,
    /// The bytes received so far.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub received: u64,
    /// The file's size; 0 while the server has not said it.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub total: u64,
    /// The file on the Host once complete; empty before.
    #[garde(length(chars, max = 4096))]
    pub path: String,
}

/// The most downloads a tab's bubble keeps, the newest.
pub const MAX_DOWNLOADS: usize = 20;

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
        /// The list's number in the Host's sequence: a list numbered higher
        /// than a request's answer names describes the browser after the
        /// request started.
        #[garde(range(max = MAX_SAFE_INTEGER))]
        list: u64,
        #[garde(dive)]
        tabs: Vec<LiveTab>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<TabId>")]
        #[garde(skip)]
        watched: Option<TabId>,
    },
    /// Video frames of this generation follow, starting with a key frame:
    /// `width` × `height` pixels of `viewport`, at `scale` of its device
    /// pixels. The page places a picture, and maps input on it, by its own
    /// generation's viewport. At scale 1 a picture
    /// is the page's own pixels, less the last row or column of an odd side,
    /// which H.264 cannot encode; below 1 congestion lowered its resolution.
    Stream {
        #[garde(skip)]
        tab: TabId,
        #[garde(skip)]
        generation: u32,
        #[garde(range(min = 1))]
        width: u32,
        #[garde(range(min = 1))]
        height: u32,
        #[garde(dive)]
        viewport: BrowserViewport,
        #[garde(range(min = f64::MIN_POSITIVE, max = 1.0))]
        scale: f64,
    },
    Heartbeat(#[garde(dive)] LiveHeartbeat),
    /// Where on the watched tab's visible page each cursor applies, in
    /// document order: a later region over an earlier one wins. The view
    /// resolves the cursor under its pointer from them; where none applies,
    /// or one leaves the cursor to the browser, the last `cursor` holds.
    Cursors {
        #[garde(skip)]
        tab: TabId,
        #[garde(length(max = MAX_CURSOR_REGIONS), dive)]
        regions: Vec<CursorRegion>,
    },
    /// The cursor the watched tab shows at the viewer's pointer, as its
    /// observer resolved it there.
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
    /// The viewer's right click reached the page, which left it to the
    /// browser: the page shows the browser's menu.
    Menu {
        #[garde(skip)]
        tab: TabId,
        #[garde(dive)]
        menu: LiveMenu,
    },
    /// The downloads the user started in the tab, the newest last.
    Downloads {
        #[garde(skip)]
        tab: TabId,
        #[garde(length(max = MAX_DOWNLOADS), dive)]
        downloads: Vec<LiveDownload>,
    },
    /// Something the viewer asked for failed; the stream goes on: a
    /// browser failure's code, [`CAPTURE_UNAVAILABLE`], [`CAPTURE_FAILED`]
    /// or [`CAPTURE_STOPPED`].
    Notice(#[garde(dive)] LiveNotice),
    /// The browser ended; the stream ends after this.
    Ended {
        #[garde(skip)]
        reason: EndReason,
    },
}

impl ModuleMessage for LiveModuleMessage {
    fn heartbeat(heartbeat: LiveHeartbeat) -> Self {
        Self::Heartbeat(heartbeat)
    }

    fn notice(notice: LiveNotice) -> Self {
        Self::Notice(notice)
    }
}
