//! Shares (`web-api.md` § Sharing, `sharing.md`): a read-only snapshot of a
//! conversation or of one answer of it, behind a random link. The owner
//! makes, takes again and removes one; anyone with its link reads it.

use demi_shared_types::{BlockId, Nullable, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

/// `GET /shares/:token`: what the shared page shows above the snapshot.
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

/// `?node=&before=|after=|around=&edge=` of `GET /shares/:token/transcript`,
/// which reads the snapshot's copy of an agent's transcript as a
/// conversation's page reads the transcript (`web-api.md` § Pages). Queries
/// are the backend's alone and are not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareTranscriptQuery {
    /// The number of the subagent whose transcript the page is of, as an
    /// `agent:` link names it; the conversation's own without it.
    #[serde(default)]
    pub node: Option<u64>,
    #[serde(default)]
    pub before: Option<u32>,
    #[serde(default)]
    pub after: Option<u32>,
    #[serde(default)]
    pub around: Option<BlockId>,
    #[serde(default)]
    pub edge: Option<BlockId>,
}

/// `?node=` of `GET /shares/:token/transcript/blocks/:blockId`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareNodeQuery {
    #[serde(default)]
    pub node: Option<u64>,
}
