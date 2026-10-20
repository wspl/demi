//! A conversation's Workflow menu
//! (`crates/backend-user-shard/src/conversation/workflows.rs`):
//!
//! - `GET /conversations/:id/workflows` lists the workflows of its working
//!   directory as the backend last read them, with when; the conversation's
//!   metadata carries their revision, so a page hears of a change.
//! - `GET /devices/:id/workflows?directory=` lists those of a new
//!   conversation's directory before its first message.

use demi_runner_protocol::files::SubscriptionKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::{Nullable, Timestamp};

use crate::files::AbsolutePath;

/// The project's workflows as the menu lists them, by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationWorkflows {
    /// The `.demi/workflows` directory they are in; null when no directory
    /// from the working directory up to the repository's root has one.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub directory: Option<String>,
    /// Why that directory cannot be used: it is a symbolic link, which is
    /// never followed; null while it can.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub unavailable: Option<String>,
    pub workflows: Vec<MenuWorkflow>,
}

/// The workflows of a directory as the backend holds them, which it answers
/// at once, never waiting on the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum KeptWorkflowList {
    /// The backend has not read them yet: their first read is announced.
    Unknown,
    /// As the backend last read them, at `read_at`; a Host offline since
    /// leaves them as they were.
    Read {
        list: ConversationWorkflows,
        read_at: Timestamp,
    },
}

/// One workflow file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MenuWorkflow {
    /// The file's name without `.yaml`.
    pub id: String,
    /// What the menu shows; null for a file that cannot be used.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub name: Option<String>,
    /// The file on the Host.
    pub path: String,
    /// Why it cannot be used: it is a link, or it does not read, with its
    /// path and the line and field that are wrong; null while it can.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub unavailable: Option<String>,
    /// The ids of the profiles it declares, their `--profile` values, which
    /// the context card's cost part follows; none for a file that does not
    /// read.
    pub profiles: Vec<String>,
}

/// A subject a project search watches on the Host's file watch: what makes
/// the backend read it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WatchSubject {
    pub kind: SubscriptionKind,
    pub path: String,
    /// See [`demi_runner_protocol::files::Subject::suffix`].
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_with::rust::unwrap_or_skip")]
    #[schemars(with = "String")]
    pub suffix: Option<String>,
}

/// `?directory=` of `GET /devices/:id/workflows`: the absolute directory a
/// new conversation works in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowsQuery {
    pub directory: AbsolutePath,
}
