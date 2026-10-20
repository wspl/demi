//! The `demi.fetch` package's contract: the arguments and the `--json` result
//! of its one operation, `fetch.get`, with their limits. It holds types and
//! their checks only; the fetch lives in `demi-fetch`.

use demi_shared_types::DecodeError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

/// The package's id, which its release descriptor names and `demi fetch`
/// binds to.
pub const PACKAGE: &str = "demi.fetch";

/// The one operation, `demi fetch`.
pub const GET: &str = "fetch.get";

/// The package's operations, as its descriptor lists them.
pub const OPERATIONS: &[&str] = &[GET];

/// How many characters a part holds when `--length` names none; it stays
/// inside the replay bound.
pub const DEFAULT_LENGTH: u32 = 15_000;
/// The most characters one part holds.
pub const MAX_LENGTH: u32 = 100_000;
/// The deadline of a fetch when `--timeout` names none, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// The longest deadline `--timeout` takes, in milliseconds.
pub const MAX_TIMEOUT_MS: u64 = 300_000;
/// The most redirects a fetch follows.
pub const MAX_REDIRECTS: usize = 10;
/// The largest body read as text: a page, JSON, XML or Markdown.
pub const MAX_TEXT_BYTES: u64 = 10 * 1024 * 1024;
/// The largest body saved as a file.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// How long a fetched page is kept on the Host, in seconds.
pub const CACHE_SECONDS: u64 = 15 * 60;

/// Why an invocation could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    /// The name is not one of the package's operations.
    #[error("unknown operation {0}")]
    Unknown(String),
    /// The operation's arguments are refused.
    #[error(transparent)]
    Invalid(#[from] DecodeError),
}

/// `fetch.get`: reads a URL from the Host and prints one part of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct FetchArgs {
    /// The http or https URL to read
    #[garde(length(min = 1))]
    pub url: String,
    /// The character the part starts at, 0 for the first
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "u64")]
    #[garde(skip)]
    pub offset: Option<u64>,
    /// How many characters the part holds
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "u32")]
    #[garde(range(min = 1, max = MAX_LENGTH))]
    pub length: Option<u32>,
    /// Print the body as received, HTML included, instead of Markdown
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub raw: Option<bool>,
    /// The whole fetch's deadline in milliseconds
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "u64")]
    #[garde(range(min = 1, max = MAX_TIMEOUT_MS))]
    pub timeout: Option<u64>,
}

impl FetchArgs {
    /// The part's first character.
    pub fn offset(&self) -> u64 {
        self.offset.unwrap_or(0)
    }

    /// How many characters the part holds.
    pub fn length(&self) -> u32 {
        self.length.unwrap_or(DEFAULT_LENGTH)
    }

    /// Whether the body is printed as received.
    pub fn raw(&self) -> bool {
        self.raw.unwrap_or(false)
    }

    /// The whole fetch's deadline in milliseconds.
    pub fn timeout_ms(&self) -> u64 {
        self.timeout.unwrap_or(DEFAULT_TIMEOUT_MS)
    }
}

/// What `demi fetch --json` prints: a page read as text, or a file saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Fetched {
    Page(FetchedPage),
    File(FetchedFile),
}

/// One part of a page read as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchedPage {
    /// The URL as given
    pub url: String,
    /// The URL the last redirect reached, the same as url without one
    pub final_url: String,
    /// The HTTP status
    pub status: u16,
    /// The media type the server named, such as text/html
    pub content_type: String,
    /// The characters of the whole page as printed
    pub characters: u64,
    /// The character the part starts at
    pub offset: u64,
    /// The part, as Markdown unless --raw
    pub content: String,
    /// The characters after the part
    pub remaining: u64,
    /// The call that reads on, or null at the end
    pub next: Option<String>,
}

/// A body that is no text, saved on the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchedFile {
    /// The URL as given
    pub url: String,
    /// The URL the last redirect reached, the same as url without one
    pub final_url: String,
    /// The HTTP status
    pub status: u16,
    /// The media type the server named, such as application/pdf
    pub content_type: String,
    /// Where the file is saved
    pub path: String,
    /// The file's size in bytes
    pub bytes: u64,
}

/// A decoded invocation: the operation and its checked arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Get(FetchArgs),
}

impl Operation {
    /// Decodes the arguments of the operation named `operation`.
    pub fn parse(operation: &str, args: serde_json::Value) -> Result<Self, OperationError> {
        match operation {
            GET => Ok(Self::Get(demi_shared_types::decode_value(args)?)),
            _ => Err(OperationError::Unknown(operation.to_owned())),
        }
    }
}
