//! What a page follows on a socket, and the version of it the page holds.

use demi_shared_types::NodeId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Version;

/// A value of a conversation that its socket can follow: it names the value
/// to `follow`, `unfollow` and `stale`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Followed {
    /// The conversation's metadata, whole.
    Metadata,
    /// The conversation's agents.
    Agents,
    /// A node's transcript: the root's, a subagent's or a side question's.
    Transcript {
        #[garde(skip)]
        node: NodeId,
    },
    /// The messages saved for later, whole.
    Saved,
    /// The workflows of the conversation's working directory, whole.
    Workflows,
    /// The conversation's shares, whole.
    Shares,
}

/// The version of a value that a page holds: the entity tag of a whole value,
/// or the version of a transcript or of the agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Held {
    /// The `ETag` the value's read or its last push carried; empty when the
    /// page holds none, which no value has, so the value is pushed.
    Tag {
        #[garde(skip)]
        tag: String,
    },
    /// The `Version` the transcript's or the agents' read or last change
    /// carried.
    Version {
        #[garde(dive)]
        version: Version,
        /// Whether the page holds the node's request output and cache chart,
        /// as its read or a push brought them: a follow of a node whose page
        /// holds none, as one shown from a copy kept on the disk, is sent them
        /// as they stand. Absent for the agents.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[garde(skip)]
        readings: bool,
    },
}

/// A value a page follows, and the version of it the page holds. The server
/// registers the follow, then compares the value's current version with
/// `holds` where the value changes, so no change falls between the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Follow {
    #[garde(dive)]
    pub value: Followed,
    #[garde(dive)]
    pub holds: Held,
}

impl Follow {
    /// A follow of the whole value `value` the page holds as `tag`.
    pub fn tagged(value: Followed, tag: impl Into<String>) -> Self {
        Self {
            value,
            holds: Held::Tag { tag: tag.into() },
        }
    }

    /// A follow of the transcript or the agents `value` the page holds at
    /// `version`.
    pub fn versioned(value: Followed, version: Version) -> Self {
        Self {
            value,
            holds: Held::Version { version, readings: false },
        }
    }
}
