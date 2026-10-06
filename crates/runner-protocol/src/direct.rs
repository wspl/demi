//! A direct channel between a page and its device's runner
//! (`direct-channel.md`): what the backend's introduction tells the runner,
//! and each operation's channel as the page and the runner speak it, its
//! header, answers and limits. The page receives these types too.

use std::collections::BTreeMap;
use std::time::Duration;

use demi_command_protocol::{CommandLocale, PackageDescriptor, conversation_name, without_nul};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::files::{DirectoryEntry, FileWatchRequest};

/// The most bytes of one binary message on a channel.
pub const MESSAGE_BYTES: usize = 64 * 1024;
/// The most bytes either end keeps queued on one channel; it writes more as
/// the queue drains. Larger queues measured slower, not faster.
pub const QUEUE_BYTES: usize = 256 * 1024;
/// The most peers a runner keeps; it refuses another offer with `busy`.
pub const MAX_PEERS: usize = 8;
/// The most channels one peer keeps open; another is refused with `busy`.
pub const MAX_CHANNELS: usize = 64;
/// How long the page waits for a peer to connect, and the runner keeps one
/// that has not.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a watch channel stays silent before it says `heartbeat`, as the
/// relay's watch does.
pub const WATCH_HEARTBEAT: Duration = Duration::from_secs(30);

/// The user stream a stream name opens: an operation of a published package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceBinding {
    #[garde(dive)]
    pub package: PackageDescriptor,
    #[garde(length(min = 1))]
    pub operation: String,
}

/// What the runner needs of the introducing user to act as the relay
/// would, which only the backend knows: the user streams the user's
/// plugins declare, by name, and the locale a command of the user's
/// receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Introduction {
    #[garde(dive)]
    pub streams: BTreeMap<String, ServiceBinding>,
    #[garde(dive)]
    pub locale: CommandLocale,
}

/// Why a runner did not answer an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfferRefusal {
    /// The runner keeps as many peers as it may.
    Busy,
    /// The offer is not one the runner can answer.
    InvalidOffer,
}

/// The page's first message on a channel: the operation, and the
/// conversation it acts for with the directory its work starts in, as the
/// conversation's summary names them (`direct-channel.md` § Who may
/// connect).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "op",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChannelHeader {
    /// Opens the user stream `stream`; its input bytes follow.
    Stream {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
        #[garde(length(min = 1))]
        stream: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        #[schemars(with = "Option<BTreeMap<String, serde_json::Value>>")]
        args: Option<serde_json::Map<String, serde_json::Value>>,
    },
    /// A file's bytes from `offset`, `length` of them or to the end; a
    /// `version` the file no longer has answers `file_changed`.
    Read {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
        offset: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
        length: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(inner(length(min = 1)))]
        version: Option<String>,
    },
    /// Writes the bytes that follow to `path`, then `{ end: true }`.
    Write {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
        #[garde(skip)]
        replace: bool,
    },
    /// A file's text, unless the file still has the `version` named.
    Text {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(inner(length(min = 1)))]
        version: Option<String>,
    },
    /// A directory's entries; the conversation's directory without a path.
    List {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(inner(length(min = 1), custom(without_nul)))]
        path: Option<String>,
    },
    /// Makes a directory with its parents.
    Mkdir {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
    },
    /// Deletes a file, or a directory with everything in it.
    Delete {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
    },
    /// The file watch; `paths` messages follow, as on the relay.
    Watch {
        #[garde(custom(conversation_name))]
        conversation: String,
        #[garde(length(min = 1), custom(without_nul))]
        cwd: String,
    },
}

impl ChannelHeader {
    /// The conversation the operation acts for, and the directory its work
    /// starts in on the device, as the conversation's summary names them.
    pub fn scope(&self) -> Scope<'_> {
        let (Self::Stream { conversation, cwd, .. }
        | Self::Read { conversation, cwd, .. }
        | Self::Write { conversation, cwd, .. }
        | Self::Text { conversation, cwd, .. }
        | Self::List { conversation, cwd, .. }
        | Self::Mkdir { conversation, cwd, .. }
        | Self::Delete { conversation, cwd, .. }
        | Self::Watch { conversation, cwd }) = self;
        Scope { conversation, cwd }
    }
}

/// The conversation an operation acts for and its directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope<'a> {
    pub conversation: &'a str,
    pub cwd: &'a str,
}

/// The page's last message on a `write` channel: the bytes are complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct WriteEnd {
    pub end: bool,
}

/// A text message the page sends on a channel after its header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageMessage {
    WriteEnd(WriteEnd),
    Watch(FileWatchRequest),
}

/// The runner's first message when it refuses an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelRefusal {
    pub error: ChannelError,
}

/// Why an operation failed: the code and HTTP status the relay's route
/// answers for the same failure, and the runner's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelError {
    pub code: ChannelErrorCode,
    pub status: u16,
    pub message: String,
}

impl ChannelError {
    pub fn new(code: ChannelErrorCode, status: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            status,
            message: message.into(),
        }
    }
}

/// The codes a channel's refusal carries, spelled as the relay's
/// `ErrorCode` spells them, and `busy` for a peer at its limit of channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelErrorCode {
    FsError,
    FileChanged,
    IsDirectory,
    FileExists,
    ProtectedPath,
    FileTooLarge,
    NotText,
    DirectoryTooLarge,
    HostOperationFailed,
    UnknownStream,
    StreamFailed,
    InvalidMessage,
    /// The answer is larger than one message on the channel carries; the
    /// relay, whose messages are larger, answers it.
    TooLarge,
    Busy,
}

/// The runner's first message when it carries out an operation that
/// answers nothing more: `stream`, `write`, `mkdir`, `delete` and `watch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Opened {
    pub ok: bool,
}

impl Opened {
    pub fn new() -> Self {
        Self { ok: true }
    }
}

impl Default for Opened {
    fn default() -> Self {
        Self::new()
    }
}

/// A `read`'s answer: the file's size and version; its bytes follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct ReadOpened {
    pub ok: bool,
    #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub size: u64,
    pub version: String,
}

/// A `text`'s answer: the file's version, and whether it is the one the
/// header named, in which case no text follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextOpened {
    pub ok: bool,
    pub version: String,
    pub unchanged: bool,
}

/// A `list`'s answer, the relay's `{ path, home, entries }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Listed {
    pub ok: bool,
    pub path: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_shared_types::Nullable<String>")]
    pub home: Option<String>,
    pub entries: Vec<DirectoryEntry>,
}

serde_plain::derive_display_from_serialize!(OfferRefusal);
serde_plain::derive_display_from_serialize!(ChannelErrorCode);
