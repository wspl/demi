//! Shares: what was said in a conversation or in one answer of it, read-only
//! behind a random link. The owner makes, takes again and removes one; anyone with
//! its link reads it.

use std::borrow::Cow;

use demi_shared_types::{Block, BlockId, InstructionEntry, MAX_SAFE_INTEGER, Nullable, Timestamp, client_block};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::conversations::CompactionSummary;

use crate::ids::ShareToken;

/// What a share holds: the conversation, or one answer of it, named by the
/// text that ends the agent's reply.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum ShareTarget {
    Conversation,
    Answer {
        #[garde(skip)]
        block_id: BlockId,
    },
}

/// `POST /conversations/:id/shares` and `PUT /conversations/:id/shares/:token`:
/// the share of the conversation, or of the answer its block ends, its
/// link, and when its snapshot was taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ShareAnswer {
    Conversation {
        token: ShareToken,
        /// The public URL followed by `/s/<token>`.
        url: String,
        taken_at: Timestamp,
    },
    Answer {
        token: ShareToken,
        url: String,
        block_id: BlockId,
        taken_at: Timestamp,
    },
}

impl ShareAnswer {
    /// The answer of the share `token` of `target`.
    pub fn new(token: ShareToken, url: String, target: ShareTarget, taken_at: Timestamp) -> Self {
        match target {
            ShareTarget::Conversation => Self::Conversation { token, url, taken_at },
            ShareTarget::Answer { block_id } => Self::Answer {
                token,
                url,
                block_id,
                taken_at,
            },
        }
    }
}

/// One share of a conversation, as its summary in the product state lists
/// it: the header and an answer read from it whether a share exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ConversationShare {
    Conversation { token: ShareToken },
    Answer { token: ShareToken, block_id: BlockId },
}

impl ConversationShare {
    pub fn new(token: ShareToken, target: ShareTarget) -> Self {
        match target {
            ShareTarget::Conversation => Self::Conversation { token },
            ShareTarget::Answer { block_id } => Self::Answer { token, block_id },
        }
    }

    pub fn token(&self) -> &ShareToken {
        match self {
            Self::Conversation { token } | Self::Answer { token, .. } => token,
        }
    }
}

/// The kind of a share, as its public page reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShareKind {
    Conversation,
    Answer,
}

impl ShareTarget {
    pub fn kind(&self) -> ShareKind {
        match self {
            Self::Conversation => ShareKind::Conversation,
            Self::Answer { .. } => ShareKind::Answer,
        }
    }
}

/// `GET /shares/:token`: what the shared page shows above what the share
/// holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SharedConversation {
    pub kind: ShareKind,
    /// The conversation's title when it was shared; null while it was the
    /// placeholder, which the page shows as New conversation in the reader's
    /// language.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub title: Option<String>,
    pub taken_at: Timestamp,
}

/// `GET /shares/:token/transcript`: a page of what the share holds, as a
/// conversation's page is answered, with no failure facts and its `instructions` always
/// empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SharedTranscriptPage {
    /// The index of the page's first entry.
    pub start: u32,
    /// How many entries the share holds.
    pub length: u32,
    pub blocks: Vec<SharedEntry>,
    pub instructions: Vec<InstructionEntry>,
    pub summaries: Vec<CompactionSummary>,
}

/// What a share's page holds at an index: a message or a text, as a page sends
/// a block, or in place of a run of the agent's steps, `{ "type":
/// "steps_not_shared", "count": 12 }`.
#[derive(Debug, Clone, PartialEq)]
pub enum SharedEntry {
    Block(Block),
    StepsNotShared(u64),
}

/// The tag of a run of steps not shared, which no block type takes.
const STEPS_NOT_SHARED: &str = "steps_not_shared";

/// A run of steps not shared as it is sent.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Steps {
    #[serde(rename = "type")]
    tag: String,
    count: u64,
}

impl Serialize for SharedEntry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Block(block) => client_block::serialize(block, serializer),
            Self::StepsNotShared(count) => Steps {
                tag: STEPS_NOT_SHARED.to_owned(),
                count: *count,
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for SharedEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("type").and_then(serde_json::Value::as_str) == Some(STEPS_NOT_SHARED) {
            let steps = Steps::deserialize(value).map_err(serde::de::Error::custom)?;
            return Ok(Self::StepsNotShared(steps.count));
        }
        Block::deserialize(value).map(Self::Block).map_err(serde::de::Error::custom)
    }
}

/// Written by hand because serde's untagged variant has no schema the
/// contracts take: a block's variants joined by the run of steps, tagged
/// the same way.
impl JsonSchema for SharedEntry {
    fn schema_name() -> Cow<'static, str> {
        "SharedEntry".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "oneOf": [
                generator.subschema_for::<Block>(),
                {
                    "type": "object",
                    "description": "A run of the agent's steps between two shown blocks, which the share does not hold.",
                    "properties": {
                        "type": { "type": "string", "const": STEPS_NOT_SHARED },
                        "count": { "type": "integer", "format": "uint64", "minimum": 1, "maximum": MAX_SAFE_INTEGER },
                    },
                    "required": ["type", "count"],
                    "additionalProperties": false,
                },
            ],
        })
    }
}

/// `?before=|after=|around=&edge=` of `GET /shares/:token/transcript`,
/// which reads the share's list as a conversation's page reads the
/// transcript. Queries are the backend's alone and are not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareTranscriptQuery {
    #[serde(default)]
    pub before: Option<u32>,
    #[serde(default)]
    pub after: Option<u32>,
    #[serde(default)]
    pub around: Option<BlockId>,
    #[serde(default)]
    pub edge: Option<BlockId>,
}
