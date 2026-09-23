//! What a message and a tool result hold (`runtime.md` § Transcript): text,
//! media the model reads natively, references, and the records of files that
//! came with a message.

use std::{borrow::Cow, fmt, str::FromStr};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize, Serializer};
use serde_with::rust::unwrap_or_skip;

use crate::B64Bytes;

/// A blob's name: the SHA-256 of its bytes in lowercase hexadecimal
/// (`storage.md` § Encodings and digests). A blob belongs to its owner's
/// namespace, so a name grants no access across users.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct BlobRef(String);

const BLOB_REF_PATTERN: &str = "^[0-9a-f]{64}$";

impl BlobRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BlobRef {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(format!(
                "{value:?} is not a blob reference (64 lowercase hexadecimal digits)"
            ));
        }
        Ok(Self(value))
    }
}

impl FromStr for BlobRef {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        Self::try_from(value.to_owned())
    }
}

impl fmt::Display for BlobRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for BlobRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl JsonSchema for BlobRef {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "BlobRef".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": BLOB_REF_PATTERN })
    }
}

/// One part of a message or a steer, in the transcript and in the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum UserContentBlock {
    Text {
        #[garde(skip)]
        text: String,
    },
    /// An image the model reads natively.
    Image {
        #[garde(dive)]
        source: MediaSource,
    },
    /// A video the model reads natively; only a model whose catalog marks
    /// video support receives one.
    Video {
        #[garde(dive)]
        source: MediaSource,
    },
    /// A PDF the model reads natively.
    Document {
        #[garde(dive)]
        source: DocumentSource,
    },
    /// Text that names something, such as a file on a paired device and the
    /// command that reads it; providers render it as text.
    Reference {
        #[garde(skip)]
        reference: String,
    },
    /// A file that came with the message.
    Attachment(#[garde(dive)] Attachment),
}

/// The record of a file that came with a message: on the conversation's Host
/// at `path`, never inlined. Providers render it as a tag that names the file
/// ([`attachment_tag`]); the page draws the file's tile from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attachment {
    #[garde(length(utf16, min = 1))]
    pub name: String,
    /// Absolute, on the conversation's Host.
    #[garde(length(utf16, min = 1))]
    pub path: String,
    #[garde(skip)]
    pub media_type: String,
    #[garde(range(max = crate::MAX_SAFE_INTEGER))]
    pub size_bytes: u64,
    /// The uploaded bytes in the blob store, from which the page fetches
    /// them.
    #[garde(skip)]
    pub sha256: BlobRef,
    /// The opening of a text file, for the tile that shows it as a page.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub snippet: Option<String>,
}

/// The model-facing form of an attachment: one self-closing tag that names
/// the file, its media type, its size and its path, never its content. For
/// example `<attachment name="notes.md" type="text/markdown" size="82"
/// path="/home/demi/.demi/attachments/c1/notes.md"/>`.
pub fn attachment_tag(attachment: &Attachment) -> String {
    let size = attachment.size_bytes.to_string();
    let attributes = [
        ("name", attachment.name.as_str()),
        ("type", attachment.media_type.as_str()),
        ("size", size.as_str()),
        ("path", attachment.path.as_str()),
    ];
    let mut tag = String::from("<attachment");
    for (key, value) in attributes {
        tag.push(' ');
        tag.push_str(key);
        tag.push_str("=\"");
        push_escaped(&mut tag, value);
        tag.push('"');
    }
    tag.push_str("/>");
    tag
}

/// Appends `value` with the characters that could end or open markup
/// escaped.
fn push_escaped(out: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

/// Whether `text` holds nothing but white space as JavaScript's `trim` counts
/// it, which includes U+FEFF and excludes U+0085: the one test of an empty
/// text, wherever emptiness decides what happens, as in message validation.
pub fn is_blank(text: &str) -> bool {
    text.chars().all(is_trimmed_space)
}

/// `text` without the white space around it, as JavaScript's `trim` removes
/// it: the characters [`is_blank`] counts as white space, so a text is blank
/// exactly when its trim is empty.
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_trimmed_space)
}

/// JavaScript's white space and line terminators.
fn is_trimmed_space(character: char) -> bool {
    character == '\u{feff}' || (character.is_whitespace() && character != '\u{85}')
}

/// Where an image's or a video's bytes are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MediaSource {
    /// The bytes themselves, as a session holds them for inference.
    Binary {
        #[garde(skip)]
        data: B64Bytes,
        #[garde(skip)]
        media_type: String,
    },
    /// A URL the provider fetches.
    Url {
        #[garde(skip)]
        url: String,
    },
    /// The bytes in the conversation owner's blob namespace, as a store keeps
    /// them and as the page receives them.
    Ref {
        #[garde(skip)]
        r#ref: BlobRef,
        #[garde(skip)]
        media_type: String,
    },
}

/// Where a document's bytes are, with the name the file came with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DocumentSource {
    Binary {
        #[garde(skip)]
        data: B64Bytes,
        #[garde(skip)]
        media_type: String,
        #[garde(skip)]
        file_name: String,
    },
    Ref {
        #[garde(skip)]
        r#ref: BlobRef,
        #[garde(skip)]
        media_type: String,
        #[garde(skip)]
        file_name: String,
    },
}

/// One part of a tool's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ToolResultContentBlock {
    Text {
        #[garde(skip)]
        text: String,
    },
    Image {
        #[garde(dive)]
        source: ToolMediaSource,
    },
    Video {
        #[garde(dive)]
        source: ToolMediaSource,
    },
}

/// Where the bytes of a tool result's image or video are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ToolMediaSource {
    Binary {
        #[garde(skip)]
        data: B64Bytes,
        #[garde(skip)]
        media_type: String,
    },
    Ref {
        #[garde(skip)]
        r#ref: BlobRef,
        #[garde(skip)]
        media_type: String,
    },
}
