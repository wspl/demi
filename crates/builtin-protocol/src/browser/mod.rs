//! The `browser.*` operations (`browser.md`): their limits, the values they
//! share, their failures, and each operation's input and result.

pub use failure::{ActionProgress, BrowserErrorCode, BrowserFailure, ErrorDetails, FailureDocument};
pub use operations::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::DecodeError;

/// The prefix of every browser operation's name, as a literal for `concat!`.
macro_rules! prefix {
    () => {
        "browser."
    };
}
pub(crate) use prefix;

/// The prefix of every browser operation's name, such as `browser.open`.
pub const PREFIX: &str = prefix!();

/// The deadline of an operation whose input names none, in milliseconds.
pub const TIMEOUT_MS: u64 = 30_000;
/// The longest deadline an input may name, and `open`'s default: a cold start
/// installs and launches Chrome first.
pub const MAX_TIMEOUT_MS: u64 = 300_000;
/// The most nodes, entries or matches one result lists.
pub const MAX_NODES: usize = 1_000;
/// How many a result lists when the input names no limit.
pub const DEFAULT_NODES: usize = 100;
/// The largest result written to stdout; a larger one is shortened or fails.
pub const INLINE_BYTES: usize = 64 * 1024;
/// The longest text an input carries, such as typed text or an expression,
/// in UTF-16 code units.
pub const STDIN_BYTES: usize = 1024 * 1024;
/// The largest PNG a clipboard holds, and its most pixels.
pub const CLIPBOARD_PNG_BYTES: usize = 16 * 1024 * 1024;
pub const CLIPBOARD_PNG_PIXELS: usize = 16_000_000;
/// The most URLs one `content.fetch` reads.
pub const FETCH_URLS: usize = 10;
/// The console entries a tab retains, and their most bytes.
pub const CONSOLE_ENTRIES: usize = 1000;
pub const CONSOLE_BYTES: usize = 1024 * 1024;
/// The CDP events a debugging connection retains, and their most bytes.
pub const CDP_EVENTS: usize = 10000;
pub const CDP_BYTES: usize = 8 * 1024 * 1024;
/// The longest locator, URL, path or name an input carries, in UTF-16 code
/// units.
pub const LOCATOR_LENGTH: usize = 4096;

/// An opaque handle: `prefix`, an underscore and the URL-safe base64 of 16
/// random bytes, 22 characters.
pub fn handle(prefix: &str, random: [u8; 16]) -> String {
    use base64::Engine;
    format!("{prefix}_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
}

/// Declares a [`handle`] with a fixed prefix as a checked newtype. Its
/// pattern is both its check and its schema, which the browser reads.
macro_rules! handle {
    ($(#[$meta:meta])* $name:ident, $prefix:literal, $what:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
        #[serde(try_from = "String")]
        #[schemars(extend("pattern" = $name::PATTERN))]
        pub struct $name(String);

        impl $name {
            /// What a handle with this prefix looks like: the prefix, an
            /// underscore and 22 characters of URL-safe base64.
            pub const PATTERN: &str = concat!("^", $prefix, "_[A-Za-z0-9_-]{22}$");

            /// The handle of 16 random bytes.
            pub fn from_random(random: [u8; 16]) -> Self {
                Self(handle($prefix, random))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                    regex::Regex::new($name::PATTERN).expect("the handle pattern compiles")
                });
                if PATTERN.is_match(&value) {
                    Ok(Self(value))
                } else {
                    Err(format!(concat!("{:?} is not a ", $what), value))
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_from(value.to_owned())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
    };
}

handle!(
    /// A tab's public identity, which `open` and `tabs` return.
    TabId,
    "t",
    "tab ID"
);
handle!(
    /// A node reference, which `inspect`, `find` and `probe` return; it
    /// stays valid while its document does.
    NodeRef,
    "e",
    "node reference"
);

closed_set! {
    /// How far a navigation loads before it answers.
    #[derive(Default)]
    pub enum Load {
        Commit = "commit",
        #[default]
        DomContentLoaded = "domcontentloaded",
        Load = "load",
    }
}

closed_set! {
    /// Which tree `inspect` returns.
    #[derive(Default)]
    pub enum InspectView {
        #[default]
        Accessibility = "accessibility",
        Dom = "dom",
    }
}

closed_set! {
    /// What `read` reads from each element; `--attribute` reads an attribute instead.
    pub enum ReadProperty {
        Text = "text",
        TextContent = "text-content",
        Html = "html",
        Value = "value",
        Visible = "visible",
        Enabled = "enabled",
        Checked = "checked",
    }
}

closed_set! {
    /// A modifier key held during pointer input. `ControlOrMeta` is Meta on
    /// macOS and Control elsewhere; the variant has no doc of its own, which
    /// would make the set's JSON Schema a union.
    pub enum Modifier {
        Alt = "Alt",
        Control = "Control",
        ControlOrMeta = "ControlOrMeta",
        Meta = "Meta",
        Shift = "Shift",
    }
}

closed_set! {
    #[derive(Default)]
    pub enum MouseButton {
        #[default]
        Left = "left",
        Middle = "middle",
        Right = "right",
    }
}

closed_set! {
    /// Where `select-text` leaves the cursor instead of selecting the text.
    pub enum TextCursor {
        Before = "before",
        After = "after",
    }
}

closed_set! {
    /// The element condition `wait` waits for.
    #[derive(Default)]
    pub enum ElementState {
        #[default]
        Visible = "visible",
        Hidden = "hidden",
        Attached = "attached",
        Detached = "detached",
        Enabled = "enabled",
    }
}

closed_set! {
    /// A clipboard item's media type.
    #[derive(Default)]
    pub enum ClipboardMime {
        #[default]
        TextPlain = "text/plain",
        TextHtml = "text/html",
        ImagePng = "image/png",
    }
}

closed_set! {
    /// What `clipboard.read` returns inline: the text; without it, every item
    /// is written to files.
    pub enum ClipboardFormat {
        Text = "text",
    }
}

closed_set! {
    /// A console entry's level.
    pub enum LogLevel {
        Debug = "debug",
        Info = "info",
        Log = "log",
        Warning = "warning",
        Error = "error",
    }
}

closed_set! {
    /// The form a page's content takes: rendered text, HTML, or the DOM tree
    /// serialized.
    #[derive(Default)]
    pub enum ContentFormat {
        #[default]
        Text = "text",
        Html = "html",
        Dom = "dom",
    }
}

closed_set! {
    pub enum AssetKind {
        Font = "font",
        Image = "image",
        Stylesheet = "stylesheet",
        Video = "video",
    }
}

closed_set! {
    pub enum DialogType {
        Alert = "alert",
        Confirm = "confirm",
        Prompt = "prompt",
        BeforeUnload = "beforeunload",
    }
}

closed_set! {
    pub enum DialogOutcome {
        Accepted = "accepted",
        Dismissed = "dismissed",
    }
}

closed_set! {
    /// Who decides a tab's viewport (`live-view.md` § Modes): the user's
    /// panel, a phone, or the agent.
    #[derive(Default)]
    pub enum ViewportMode {
        #[default]
        Web = "web",
        Mobile = "mobile",
        Custom = "custom",
    }
}

/// Declares a struct with an element target's locator fields between its own
/// leading and trailing fields. Serde's `flatten` cannot be combined with
/// `deny_unknown_fields`, so every struct that carries a target has the
/// fields written into it; `target` adds the scope fields.
macro_rules! locator_struct {
    (
        target
        $(#[$meta:meta])*
        pub struct $name:ident { $($before:tt)* } { $($after:tt)* }
    ) => {
        locator_struct! {
            locator
            $(#[$meta])*
            pub struct $name { $($before)* } {
                /// Frame references, outermost to innermost
                #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
                #[schemars(with = "Vec<NodeRef>")]
                #[garde(skip)]
                pub frame: Option<Vec<NodeRef>>,
                /// Explicit zero-based match index
                #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
                #[schemars(with = "usize")]
                #[garde(range(max = MAX_NODES - 1))]
                pub nth: Option<usize>,
                /// Container node reference
                #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
                #[schemars(with = "NodeRef")]
                #[garde(skip)]
                pub within: Option<NodeRef>,
                $($after)*
            }
        }
    };
    (
        locator
        $(#[$meta:meta])*
        pub struct $name:ident { $($before:tt)* } { $($after:tt)* }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
        #[serde(rename_all = "kebab-case", deny_unknown_fields)]
        pub struct $name {
            $($before)*
            /// A node reference returned by inspect or find
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "NodeRef")]
            #[garde(skip)]
            pub r#ref: Option<NodeRef>,
            /// Accessible role, ASCII case-insensitive, such as button, textbox, or date
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub role: Option<String>,
            /// Accessible name, with --role
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub name: Option<String>,
            /// Accessible-name regular expression, with --role
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub name_pattern: Option<String>,
            /// Rendered-text regular expression
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub text_pattern: Option<String>,
            /// Associated label text (label for, wrapping label, or aria-labelledby)
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub label: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub placeholder: Option<String>,
            /// Visible text to match
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub text_match: Option<String>,
            /// data-testid attribute
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub test_id: Option<String>,
            /// CSS selector
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "String")]
            #[garde(length(utf16, min = 1, max = LOCATOR_LENGTH))]
            pub css: Option<String>,
            /// Match the complete name or text
            #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
            #[schemars(with = "bool")]
            #[garde(skip)]
            pub exact: Option<bool>,
            $($after)*
        }
    };
}

locator_struct! {
    target
    /// An element target: a node reference, or a role, text, label,
    /// placeholder, test ID or CSS selector, narrowed by frames, a container
    /// and an index. Admission checks which fields may combine
    /// (`browser.md` § Element targets).
    #[derive(Default)]
    pub struct BrowserTarget {} {}
}

locator_struct! {
    locator
    /// A query's base locator: a target without its frames, container and
    /// index, which the query's own `frame`, `within` and `nth` express.
    #[derive(Default)]
    pub struct BrowserQueryMatch {} {}
}

impl From<BrowserQueryMatch> for BrowserTarget {
    fn from(locator: BrowserQueryMatch) -> Self {
        let BrowserQueryMatch {
            r#ref,
            role,
            name,
            name_pattern,
            text_pattern,
            label,
            placeholder,
            text_match,
            test_id,
            css,
            exact,
        } = locator;
        Self {
            r#ref,
            role,
            name,
            name_pattern,
            text_pattern,
            label,
            placeholder,
            text_match,
            test_id,
            css,
            exact,
            frame: None,
            nth: None,
            within: None,
        }
    }
}

/// A declarative query tree (`browser.md` § Queries): one base, `match`,
/// `and` or `or`, narrowed by a container, a frame, what the element has or
/// lacks, its text, its visibility and an index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserQuery {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "BrowserQueryMatch")]
    #[garde(dive)]
    pub r#match: Option<BrowserQueryMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Box<BrowserQuery>")]
    #[garde(dive)]
    pub within: Option<Box<BrowserQuery>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Box<BrowserQuery>")]
    #[garde(dive)]
    pub frame: Option<Box<BrowserQuery>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<BrowserQuery>")]
    #[garde(length(min = 1), dive)]
    pub and: Option<Vec<BrowserQuery>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<BrowserQuery>")]
    #[garde(length(min = 1), dive)]
    pub or: Option<Vec<BrowserQuery>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Box<BrowserQuery>")]
    #[garde(dive)]
    pub has: Option<Box<BrowserQuery>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Box<BrowserQuery>")]
    #[garde(dive)]
    pub has_not: Option<Box<BrowserQuery>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(utf16, max = STDIN_BYTES))]
    pub has_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(utf16, max = STDIN_BYTES))]
    pub has_not_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "usize")]
    #[garde(skip)]
    pub nth: Option<usize>,
}

impl BrowserQuery {
    /// Decodes the query tree `find --query` reads from stdin: every branch
    /// has exactly one base.
    pub fn parse(body: &str) -> Result<Self, DecodeError> {
        let query: Self = crate::decode_slice(body.as_bytes())?;
        for branch in query.branches() {
            let bases = usize::from(branch.r#match.is_some())
                + usize::from(branch.and.is_some())
                + usize::from(branch.or.is_some());
            if bases != 1 {
                return Err(DecodeError::Invalid(
                    "a query requires exactly one base: match, and, or".into(),
                ));
            }
        }
        Ok(query)
    }

    /// This query and every query nested in it.
    pub fn branches(&self) -> impl Iterator<Item = &Self> {
        let mut pending = vec![self];
        std::iter::from_fn(move || {
            let branch = pending.pop()?;
            pending.extend(branch.within.as_deref());
            pending.extend(branch.frame.as_deref());
            pending.extend(branch.has.as_deref());
            pending.extend(branch.has_not.as_deref());
            pending.extend(branch.and.iter().chain(branch.or.iter()).flatten());
            Some(branch)
        })
    }
}

/// A node's value: an input's text, or a range or progress number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NodeValue {
    Text(String),
    Number(f64),
}

/// A node's box in viewport CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One accessibility node that `find` matched or `probe` found under a point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserNode {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "NodeRef")]
    pub r#ref: Option<NodeRef>,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "NodeValue")]
    pub value: Option<NodeValue>,
    pub depth: usize,
    pub states: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Bounds")]
    pub bounds: Option<Bounds>,
}

/// One node of the tree `inspect` returns: an accessibility node, or a DOM
/// element with its tag.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserTreeNode {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "NodeRef")]
    pub r#ref: Option<NodeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "NodeValue")]
    pub value: Option<NodeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<String>")]
    pub states: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<BrowserTreeNode>")]
    pub children: Option<Vec<BrowserTreeNode>>,
}

/// Who opened a tab: an agent node, a page's `window.open`, a temporary
/// command such as `content.fetch`, or the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum BrowserCreatedBy {
    Agent {
        #[serde(rename = "nodeId")]
        #[garde(skip)]
        node_id: String,
    },
    Page {
        #[garde(skip)]
        opener: TabId,
    },
    Temporary {
        #[serde(rename = "nodeId")]
        #[garde(skip)]
        node_id: String,
    },
    User {},
}

/// A tab as `tabs` lists it and the conversation browser tab routes return it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserTab {
    pub id: TabId,
    pub title: String,
    pub url: String,
    pub created_by: BrowserCreatedBy,
}

/// A tab's viewport (`live-view.md` § Modes): its CSS size, the pixel ratio
/// it renders at, and who decides them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserViewport {
    #[garde(range(min = 1))]
    pub width: u32,
    #[garde(range(min = 1))]
    pub height: u32,
    #[garde(range(min = f64::MIN_POSITIVE))]
    pub device_pixel_ratio: f64,
    #[garde(skip)]
    pub mode: ViewportMode,
}

/// A JavaScript dialog a tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dialog {
    pub r#type: DialogType,
    pub message: String,
}

// Declared after `locator_struct!`, which they use.
mod failure;
mod operations;
