//! Each `browser.*` operation's input and result (`browser.md` § Results).
//! Inputs are the flat argument objects the command's flags and positionals
//! make; results are what `--json` prints.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::rust::unwrap_or_skip;

use super::{
    AssetKind, BrowserFailure, BrowserNode, BrowserTab, BrowserTarget, BrowserTreeNode,
    BrowserViewport, ClipboardFormat, ClipboardMime, ContentFormat, Dialog, DialogOutcome,
    DialogType, ElementState, FETCH_URLS, InspectView, LOCATOR_LENGTH, Load, LogLevel,
    MAX_NODES, MAX_TIMEOUT_MS, Modifier, MouseButton, NodeRef, ReadProperty, STDIN_BYTES,
    TIMEOUT_MS, TabId, TextCursor,
};
use crate::DecodeError;

/// What every operation's input answers about itself.
pub trait BrowserInput {
    /// The deadline when the input names none, in milliseconds.
    const DEFAULT_TIMEOUT_MS: u64 = TIMEOUT_MS;

    /// The deadline the input names, in milliseconds.
    fn timeout_ms(&self) -> Option<u64>;

    /// The whole operation's deadline.
    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms().unwrap_or(Self::DEFAULT_TIMEOUT_MS))
    }

    /// The tab the operation acts on; `open`, `tabs` and `content.fetch` act
    /// on none.
    fn tab(&self) -> Option<&TabId> {
        None
    }

    /// The element target the input carries.
    fn target(&self) -> Option<BrowserTarget> {
        None
    }
}

/// Declares an operation's input: its own fields, then the deadline every
/// input takes, with its [`BrowserInput`] implementation. `tab` puts the tab
/// the operation acts on first; `targeted` puts the tab and an element target
/// first.
macro_rules! input {
    (@struct $(#[$meta:meta])* $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
        #[serde(rename_all = "kebab-case", deny_unknown_fields)]
        pub struct $name {
            $($body)*
            /// Whole operation deadline in milliseconds
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "u64")]
            #[garde(range(min = 1, max = MAX_TIMEOUT_MS))]
            pub timeout: Option<u64>,
        }
    };
    (
        tab
        $(#[$meta:meta])*
        pub struct $name:ident { $($body:tt)* }
    ) => {
        input!(@struct $(#[$meta])* $name {
            /// Browser tab ID returned by open or tabs
            #[garde(skip)]
            pub tab: TabId,
            $($body)*
        });

        impl BrowserInput for $name {
            fn timeout_ms(&self) -> Option<u64> {
                self.timeout
            }

            fn tab(&self) -> Option<&TabId> {
                Some(&self.tab)
            }
        }
    };
    (
        targeted
        $(#[$meta:meta])*
        pub struct $name:ident { $($body:tt)* }
    ) => {
        locator_struct! {
            target
            $(#[$meta])*
            pub struct $name {
                /// Browser tab ID returned by open or tabs
                #[garde(skip)]
                pub tab: TabId,
            } {
                $($body)*
                /// Whole operation deadline in milliseconds
                #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
                #[schemars(with = "u64")]
                #[garde(range(min = 1, max = MAX_TIMEOUT_MS))]
                pub timeout: Option<u64>,
            }
        }

        impl BrowserInput for $name {
            fn timeout_ms(&self) -> Option<u64> {
                self.timeout
            }

            fn tab(&self) -> Option<&TabId> {
                Some(&self.tab)
            }

            fn target(&self) -> Option<BrowserTarget> {
                Some(BrowserTarget {
                    r#ref: self.r#ref.clone(),
                    role: self.role.clone(),
                    name: self.name.clone(),
                    name_pattern: self.name_pattern.clone(),
                    text_pattern: self.text_pattern.clone(),
                    label: self.label.clone(),
                    placeholder: self.placeholder.clone(),
                    text_match: self.text_match.clone(),
                    test_id: self.test_id.clone(),
                    css: self.css.clone(),
                    exact: self.exact,
                    frame: self.frame.clone(),
                    nth: self.nth,
                    within: self.within.clone(),
                })
            }
        }
    };
    (
        $(#[$meta:meta])*
        pub struct $name:ident { $($body:tt)* }
    ) => {
        input!(@struct $(#[$meta])* $name { $($body)* });

        impl BrowserInput for $name {
            fn timeout_ms(&self) -> Option<u64> {
                self.timeout
            }
        }
    };
}

input!(@struct
    /// `open`: opens a tab at a URL, starting the browser when it does not run.
    OpenInput {
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub url: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Load")]
        #[garde(skip)]
        pub load: Option<Load>,
    }
);

impl BrowserInput for OpenInput {
    /// A cold start installs and launches Chrome before the page loads.
    const DEFAULT_TIMEOUT_MS: u64 = MAX_TIMEOUT_MS;

    fn timeout_ms(&self) -> Option<u64> {
        self.timeout
    }
}

/// What `open` answers: the new tab, with its title and viewport when the
/// page reported them in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenResult {
    pub tab: TabId,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "BrowserViewport")]
    pub viewport: Option<BrowserViewport>,
}

input! {
    /// `tabs`: lists the browser's tabs.
    pub struct TabsInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(skip)]
        pub offset: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1, max = MAX_NODES))]
        pub limit: Option<usize>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabsResult {
    pub tabs: Vec<BrowserTab>,
    pub truncated: bool,
}

input! {
    tab
    /// `info`: reads a tab's URL, title, viewport and dialog.
    pub struct InfoInput {}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InfoResult {
    pub tab: TabId,
    pub url: String,
    pub title: String,
    pub viewport: BrowserViewport,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Dialog")]
    pub dialog: Option<Dialog>,
}

input! {
    tab
    /// `goto`: navigates a tab to a URL.
    pub struct GotoInput {
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub url: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Load")]
        #[garde(skip)]
        pub load: Option<Load>,
    }
}

input! {
    tab
    /// `back`: goes one entry back in a tab's history.
    pub struct BackInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Load")]
        #[garde(skip)]
        pub load: Option<Load>,
    }
}

input! {
    tab
    /// `forward`: goes one entry forward in a tab's history.
    pub struct ForwardInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Load")]
        #[garde(skip)]
        pub load: Option<Load>,
    }
}

input! {
    tab
    /// `reload`: reloads a tab's document.
    pub struct ReloadInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Load")]
        #[garde(skip)]
        pub load: Option<Load>,
    }
}

/// What a navigation answers: the URL it observed, with the title when the
/// same document reported it in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavigationResult {
    pub tab: TabId,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub title: Option<String>,
}

input! {
    tab
    /// `history`: lists a tab's navigation entries.
    pub struct HistoryInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(skip)]
        pub offset: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1, max = MAX_NODES))]
        pub limit: Option<usize>,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    pub index: usize,
    pub url: String,
    pub title: String,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryResult {
    pub entries: Vec<HistoryEntry>,
    pub truncated: bool,
}

input! {
    tab
    /// `close`: closes a tab.
    pub struct CloseInput {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseResult {
    pub closed: TabId,
}

input! {
    tab
    /// `inspect`: reads a tab's accessibility or DOM tree.
    pub struct InspectInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "InspectView")]
        #[garde(skip)]
        pub view: Option<InspectView>,
        /// Container node reference
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "NodeRef")]
        #[garde(skip)]
        pub within: Option<NodeRef>,
        /// Frame references, outermost to innermost
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<NodeRef>")]
        #[garde(skip)]
        pub frame: Option<Vec<NodeRef>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1, max = MAX_NODES))]
        pub limit: Option<usize>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectResult {
    pub tab: TabId,
    pub url: String,
    pub title: String,
    pub view: InspectView,
    pub tree: Vec<BrowserTreeNode>,
    pub truncated: bool,
}

input! {
    targeted
    /// `find`: lists the elements a target or a query tree matches.
    pub struct FindInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(skip)]
        pub offset: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1, max = MAX_NODES))]
        pub limit: Option<usize>,
        /// Read a declarative query tree from stdin
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub query: Option<bool>,
        /// JSON query tree when --query is supplied
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub body: Option<String>,
    }
}

/// What `find` answers. `count` is every current match, even when `offset`
/// and `limit` return fewer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindResult {
    pub matches: Vec<BrowserNode>,
    pub count: usize,
    pub truncated: bool,
}

input! {
    targeted
    /// `read`: reads a property or an attribute of the target's element, or
    /// of every match with `--all`.
    pub struct ReadInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ReadProperty")]
        #[garde(skip)]
        pub property: Option<ReadProperty>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub attribute: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub all: Option<bool>,
    }
}

/// What `read` answers: one value, or every match's with `--all`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ReadResult {
    One { value: Value },
    All { values: Vec<Value>, truncated: bool },
}

input! {
    tab
    /// `screenshot`: captures a tab's viewport, whole page or a rectangle as PNG.
    pub struct ScreenshotInput {
        /// New output file on this Host
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub overwrite: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub full_page: Option<bool>,
        /// CSS rectangle: x,y,width,height
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub clip: Option<String>,
    }
}

closed_set! {
    /// The media type of a screenshot.
    pub enum ImageMime {
        Png = "image/png",
    }
}

/// What `screenshot` answers when it writes a file; `width` and `height` are
/// in CSS pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScreenshotResult {
    pub path: String,
    pub mime_type: ImageMime,
    pub width: u32,
    pub height: u32,
    pub viewport: BrowserViewport,
}

input! {
    tab
    /// `probe`: lists the nodes under a viewport point.
    pub struct ProbeInput {
        /// Viewport CSS coordinates: x,y
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub xy: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub include_non_interactable: Option<bool>,
        /// New output file on this Host
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub overwrite: Option<bool>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeResult {
    pub matches: Vec<BrowserNode>,
    pub viewport: BrowserViewport,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub path: Option<String>,
    pub truncated: bool,
}

input! {
    targeted
    /// `click`: clicks the target's element or a viewport point.
    pub struct ClickInput {
        /// Viewport CSS coordinates: x,y
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub xy: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<Modifier>")]
        #[garde(skip)]
        pub modifier: Option<Vec<Modifier>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "u8")]
        #[garde(range(min = 1, max = 2))]
        pub count: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "MouseButton")]
        #[garde(skip)]
        pub button: Option<MouseButton>,
        /// Expected URL glob after the action
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub wait_url: Option<String>,
    }
}

input! {
    targeted
    /// `move`: moves the pointer over the target's element or to a point.
    pub struct MoveInput {
        /// Viewport CSS coordinates: x,y
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub xy: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<Modifier>")]
        #[garde(skip)]
        pub modifier: Option<Vec<Modifier>>,
    }
}

input! {
    tab
    /// `drag`: drags the pointer through viewport points.
    pub struct DragInput {
        /// Viewport CSS coordinates x,y, at least two
        #[garde(length(min = 2), inner(length(utf16, min = 1, max = LOCATOR_LENGTH)))]
        pub point: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<Modifier>")]
        #[garde(skip)]
        pub modifier: Option<Vec<Modifier>>,
    }
}

input! {
    targeted
    /// `scroll`: scrolls at the target's element or a point.
    pub struct ScrollInput {
        /// Viewport CSS coordinates: x,y
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub xy: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<Modifier>")]
        #[garde(skip)]
        pub modifier: Option<Vec<Modifier>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "f64")]
        #[garde(skip)]
        pub dx: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "f64")]
        #[garde(skip)]
        pub dy: Option<f64>,
    }
}

input! {
    targeted
    /// `fill`: replaces the value of the target's field.
    pub struct FillInput {
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub text: String,
        /// Expected URL glob after the action
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub wait_url: Option<String>,
    }
}

input! {
    targeted
    /// `type`: types text key by key, into the target or the focused element.
    pub struct TypeInput {
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub text: String,
    }
}

input! {
    targeted
    /// `key`: presses a key or a chord.
    pub struct KeyInput {
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub key: String,
        /// Expected URL glob after the action
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub wait_url: Option<String>,
    }
}

input! {
    targeted
    /// `check`: checks or unchecks the target's checkbox or radio button.
    pub struct CheckInput {
        #[garde(skip)]
        pub value: bool,
    }
}

input! {
    targeted
    /// `select`: selects options of the target's select element by value,
    /// label or index.
    pub struct SelectInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<String>")]
        #[garde(inner(inner(length(utf16, max = STDIN_BYTES))))]
        pub value: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<String>")]
        #[garde(inner(inner(length(utf16, max = STDIN_BYTES))))]
        pub option_label: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<usize>")]
        #[garde(skip)]
        pub option_index: Option<Vec<usize>>,
    }
}

input! {
    targeted
    /// `select-text`: selects text inside the target, or places the cursor
    /// before or after it.
    pub struct SelectTextInput {
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub text: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "TextCursor")]
        #[garde(skip)]
        pub cursor: Option<TextCursor>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub prefix: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub suffix: Option<String>,
    }
}

/// What a pointer or form action answers: the operation and the target it
/// acted on, its own result, and what it observed after: the URL, tabs the
/// page opened, and a dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionResult {
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub target: Option<String>,
    pub result: Value,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<TabId>")]
    pub opened_tabs: Option<Vec<TabId>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Dialog")]
    pub dialog: Option<Dialog>,
}

impl ActionResult {
    /// An action's result with nothing observed after it.
    pub fn new(operation: impl Into<String>, result: Value) -> Self {
        Self {
            operation: operation.into(),
            target: None,
            result,
            url: None,
            opened_tabs: None,
            dialog: None,
        }
    }
}

input! {
    targeted
    /// `wait`: waits for a URL, the current document's load, or an element
    /// condition.
    pub struct WaitInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Load")]
        #[garde(skip)]
        pub load: Option<Load>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ElementState")]
        #[garde(skip)]
        pub state: Option<ElementState>,
    }
}

/// What `wait` answers; a load wait carries neither `url` nor `ref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitResult {
    pub condition: String,
    pub matched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "NodeRef")]
    pub r#ref: Option<NodeRef>,
}

input! {
    targeted
    /// `upload`: sets the files of the target's file input.
    pub struct UploadInput {
        #[garde(length(min = 1), inner(length(utf16, min = 1, max = LOCATOR_LENGTH)))]
        pub file: Vec<String>,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadResult {
    pub files: Vec<String>,
    pub attached: usize,
}

input! {
    targeted
    /// `download`: clicks the target or a point and saves the download it starts.
    pub struct DownloadInput {
        /// Viewport CSS coordinates: x,y
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub xy: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<Modifier>")]
        #[garde(skip)]
        pub modifier: Option<Vec<Modifier>>,
        /// New output file on this Host
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub overwrite: Option<bool>,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadResult {
    pub path: String,
    pub suggested_filename: String,
    pub bytes: u64,
    pub mime_type: String,
}

input! {
    tab
    /// `clipboard.write`: writes stdin to the tab's clipboard.
    pub struct ClipboardWriteInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ClipboardMime")]
        #[garde(skip)]
        pub mime: Option<ClipboardMime>,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardWriteResult {
    pub mime_type: ClipboardMime,
    pub bytes: usize,
}

input! {
    tab
    /// `clipboard.read`: reads the tab's clipboard as text, or writes every
    /// item to a directory.
    pub struct ClipboardReadInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ClipboardFormat")]
        #[garde(skip)]
        pub format: Option<ClipboardFormat>,
        /// Output directory on the invoking Host
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub output_dir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub overwrite: Option<bool>,
    }
}

/// A clipboard item `clipboard.read` wrote to a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardItem {
    pub mime_type: ClipboardMime,
    pub path: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ClipboardReadResult {
    Text { text: String },
    Items { items: Vec<ClipboardItem> },
}

input! {
    targeted
    /// `eval`: evaluates a read-only expression in the page, or a function of
    /// the target's element.
    pub struct EvalInput {
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub expression: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub all: Option<bool>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalResult {
    pub value: Value,
}

input! {
    tab
    /// `logs`: reads a tab's console entries after a cursor.
    pub struct LogsInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<LogLevel>")]
        #[garde(skip)]
        pub level: Option<Vec<LogLevel>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub filter: Option<String>,
        /// Cursor returned by a previous read
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1, max = MAX_NODES))]
        pub limit: Option<usize>,
    }
}

/// One console entry, numbered in the tab's order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    pub sequence: u64,
    pub level: LogLevel,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub url: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogsResult {
    pub entries: Vec<LogEntry>,
    pub cursor: String,
    pub has_more: bool,
    pub truncated: bool,
}

input! {
    tab
    /// `viewport.set`: gives a tab the agent's viewport.
    pub struct ViewportSetInput {
        #[garde(range(min = 1, max = 4096))]
        pub width: u32,
        #[garde(range(min = 1, max = 4096))]
        pub height: u32,
        /// Device pixel ratio, 1 by default
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "f64")]
        #[garde(range(min = 0.5, max = 4.0))]
        pub scale: Option<f64>,
    }
}

input! {
    tab
    /// `viewport.reset`: returns a tab's viewport to the user's panel.
    pub struct ViewportResetInput {}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewportResult {
    pub viewport: BrowserViewport,
}

input! {
    tab
    /// `dialog.inspect`: reads the tab's JavaScript dialog.
    pub struct DialogInspectInput {}
}

/// What `dialog.inspect` answers: the dialog, or `null` when none is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialogInspectResult {
    #[serde(deserialize_with = "Option::deserialize")]
    pub dialog: Option<Dialog>,
}

input! {
    tab
    /// `dialog.accept`: accepts the tab's dialog, answering a prompt with text.
    pub struct DialogAcceptInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub text: Option<String>,
    }
}

input! {
    tab
    /// `dialog.dismiss`: dismisses the tab's dialog.
    pub struct DialogDismissInput {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialogResult {
    pub r#type: DialogType,
    pub outcome: DialogOutcome,
}

input! {
    tab
    /// `cdp.targets`: lists the CDP targets of a tab: its page, frames and workers.
    pub struct CdpTargetsInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(skip)]
        pub offset: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1, max = MAX_NODES))]
        pub limit: Option<usize>,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpTarget {
    pub id: String,
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpTargetsResult {
    pub targets: Vec<CdpTarget>,
    pub truncated: bool,
}

input! {
    tab
    /// `cdp.detach`: ends this caller's debugging connection to a tab.
    pub struct CdpDetachInput {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpDetachResult {
    pub detached: TabId,
}

input! {
    tab
    /// `cdp.send`: sends one CDP command with JSON parameters.
    pub struct CdpSendInput {
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub method: String,
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub params: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub target: Option<String>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpSendResult {
    pub method: String,
    pub result: Value,
}

input! {
    tab
    /// `cdp.events`: reads the CDP events this caller's connection received
    /// after a cursor.
    pub struct CdpEventsInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<String>")]
        #[garde(inner(inner(length(utf16, min = 1, max = LOCATOR_LENGTH))))]
        pub method: Option<Vec<String>>,
        /// Cursor returned by a previous read
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "usize")]
        #[garde(range(min = 1, max = MAX_NODES))]
        pub limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub target: Option<String>,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpEvent {
    pub sequence: u64,
    pub method: String,
    pub params: Value,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CdpEventsResult {
    pub events: Vec<CdpEvent>,
    pub cursor: String,
    pub has_more: bool,
    pub truncated: bool,
}

input! {
    tab
    /// `content.read`: reads a tab's content as text, HTML or DOM, inline or
    /// into a file.
    pub struct ContentReadInput {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ContentFormat")]
        #[garde(skip)]
        pub format: Option<ContentFormat>,
        /// New output file on this Host
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub overwrite: Option<bool>,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ContentReadResult {
    Inline {
        url: String,
        title: String,
        format: ContentFormat,
        content: String,
        truncated: bool,
    },
    File {
        url: String,
        title: String,
        format: ContentFormat,
        path: String,
    },
}

input! {
    /// `content.fetch`: loads URLs in temporary tabs and reads their content.
    pub struct ContentFetchInput {
        #[garde(length(min = 1, max = FETCH_URLS), inner(length(utf16, min = 1, max = LOCATOR_LENGTH)))]
        pub url: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "ContentFormat")]
        #[garde(skip)]
        pub format: Option<ContentFormat>,
    }
}

/// One URL `content.fetch` read, or its failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchedPage {
    pub requested_url: String,
    pub url: String,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "BrowserFailure")]
    pub error: Option<BrowserFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentFetchResult {
    pub pages: Vec<FetchedPage>,
    pub truncated: bool,
}

input! {
    tab
    /// `assets.list`: lists a tab's fonts, images, stylesheets, videos and
    /// inline SVGs as an inventory.
    pub struct AssetsListInput {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub kind: AssetKind,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineSvg {
    pub id: String,
    pub html: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetsListResult {
    pub inventory: String,
    pub assets: Vec<Asset>,
    pub inline_svgs: Vec<InlineSvg>,
    pub truncated: bool,
}

input! {
    tab
    /// `assets.export`: saves an inventory's assets to a directory with a manifest.
    pub struct AssetsExportInput {
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub inventory: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<String>")]
        #[garde(inner(inner(length(utf16, min = 1, max = LOCATOR_LENGTH))))]
        pub id: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "Vec<AssetKind>")]
        #[garde(skip)]
        pub kind: Option<Vec<AssetKind>>,
        /// Output directory on the invoking Host
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub output_dir: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "bool")]
        #[garde(skip)]
        pub overwrite: Option<bool>,
    }
}

/// An asset `assets.export` saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportedAsset {
    pub id: String,
    pub path: String,
    pub bytes: usize,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetsExportResult {
    pub directory: String,
    pub manifest: String,
    pub files: Vec<ExportedAsset>,
}

input! {
    tab
    /// `capabilities`: lists what the page supports, such as WebMCP.
    pub struct CapabilitiesInput {}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Value")]
    pub schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesResult {
    pub capabilities: Vec<Capability>,
}

input! {
    tab
    /// `webmcp.list`: lists the WebMCP tools the page declares.
    pub struct WebmcpListInput {}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebmcpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Value")]
    pub output_schema: Option<Value>,
}

/// What `webmcp.list` answers: the declarations' generation, which
/// `webmcp.call` names, and the tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WebmcpListResult {
    pub tools: String,
    pub entries: Vec<WebmcpTool>,
    pub truncated: bool,
}

input! {
    tab
    /// `webmcp.call`: calls one of the page's WebMCP tools with JSON arguments.
    pub struct WebmcpCallInput {
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub tool: String,
        /// The generation `webmcp.list` returned
        #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
        pub tools: String,
        #[garde(length(utf16, max = STDIN_BYTES))]
        pub arguments: String,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WebmcpCallResult {
    pub name: String,
    pub result: Value,
}

/// Declares the operations: the enum of decoded inputs, its accessors, and
/// the operation list.
macro_rules! operations {
    ($($name:literal => $variant:ident($input:ty),)*) => {
        /// A decoded `browser.*` invocation: the operation and its checked input.
        #[derive(Debug, Clone, PartialEq)]
        pub enum BrowserOperation {
            $($variant($input),)*
        }

        impl BrowserOperation {
            /// Decodes the arguments of the operation `browser.<name>`.
            pub fn parse(name: &str, args: Value) -> Result<Self, DecodeError> {
                match name {
                    $($name => crate::decode(args).map(Self::$variant),)*
                    _ => Err(DecodeError::UnknownOperation(format!("{}{name}", super::PREFIX))),
                }
            }

            /// The operation's name without the `browser.` prefix.
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $name,)*
                }
            }

            pub fn timeout(&self) -> Duration {
                match self {
                    $(Self::$variant(input) => input.timeout(),)*
                }
            }

            pub fn tab(&self) -> Option<&TabId> {
                match self {
                    $(Self::$variant(input) => input.tab(),)*
                }
            }

            pub fn target(&self) -> Option<BrowserTarget> {
                match self {
                    $(Self::$variant(input) => input.target(),)*
                }
            }
        }

        /// The browser operations, as the package descriptor lists them.
        pub const OPERATIONS: &[&str] = &[$(concat!(super::prefix!(), $name),)*];
    };
}

operations! {
    "open" => Open(OpenInput),
    "tabs" => Tabs(TabsInput),
    "info" => Info(InfoInput),
    "goto" => Goto(GotoInput),
    "back" => Back(BackInput),
    "forward" => Forward(ForwardInput),
    "reload" => Reload(ReloadInput),
    "history" => History(HistoryInput),
    "close" => Close(CloseInput),
    "inspect" => Inspect(InspectInput),
    "find" => Find(FindInput),
    "read" => Read(ReadInput),
    "screenshot" => Screenshot(ScreenshotInput),
    "probe" => Probe(ProbeInput),
    "click" => Click(ClickInput),
    "move" => Move(MoveInput),
    "drag" => Drag(DragInput),
    "scroll" => Scroll(ScrollInput),
    "fill" => Fill(FillInput),
    "type" => Type(TypeInput),
    "key" => Key(KeyInput),
    "check" => Check(CheckInput),
    "select" => Select(SelectInput),
    "select-text" => SelectText(SelectTextInput),
    "wait" => Wait(WaitInput),
    "upload" => Upload(UploadInput),
    "download" => Download(DownloadInput),
    "clipboard.write" => ClipboardWrite(ClipboardWriteInput),
    "clipboard.read" => ClipboardRead(ClipboardReadInput),
    "eval" => Eval(EvalInput),
    "logs" => Logs(LogsInput),
    "viewport.set" => ViewportSet(ViewportSetInput),
    "viewport.reset" => ViewportReset(ViewportResetInput),
    "dialog.inspect" => DialogInspect(DialogInspectInput),
    "dialog.accept" => DialogAccept(DialogAcceptInput),
    "dialog.dismiss" => DialogDismiss(DialogDismissInput),
    "cdp.targets" => CdpTargets(CdpTargetsInput),
    "cdp.detach" => CdpDetach(CdpDetachInput),
    "cdp.send" => CdpSend(CdpSendInput),
    "cdp.events" => CdpEvents(CdpEventsInput),
    "content.read" => ContentRead(ContentReadInput),
    "content.fetch" => ContentFetch(ContentFetchInput),
    "assets.list" => AssetsList(AssetsListInput),
    "assets.export" => AssetsExport(AssetsExportInput),
    "capabilities" => Capabilities(CapabilitiesInput),
    "webmcp.list" => WebmcpList(WebmcpListInput),
    "webmcp.call" => WebmcpCall(WebmcpCallInput),
}

impl BrowserOperation {
    /// The URL glob an action waits for after its input.
    pub fn wait_url(&self) -> Option<&str> {
        match self {
            Self::Click(input) => input.wait_url.as_deref(),
            Self::Fill(input) => input.wait_url.as_deref(),
            Self::Key(input) => input.wait_url.as_deref(),
            _ => None,
        }
    }
}
