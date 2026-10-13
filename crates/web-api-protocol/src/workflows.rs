//! A conversation's Workflow menu (`subagents.md` § Workflows of a
//! project, `product.md` § A conversation's workflow): `GET
//! /conversations/:id/workflows` lists the workflows of its working
//! directory with what the page watches to hear of a change, and `GET
//! /devices/:id/workflows?directory=` those of a new conversation's
//! directory before its first message.

use demi_runner_protocol::files::SubscriptionKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::Nullable;

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
    /// The page's file watch subscriptions whose change means the list may
    /// have changed: the workflows directory's details, and the names of
    /// each nearer directory and `.demi` that could gain one.
    pub watch: Vec<WatchSubject>,
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
}

/// A subject the page subscribes to on the file watch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WatchSubject {
    pub kind: SubscriptionKind,
    pub path: String,
}

/// `?directory=` of `GET /devices/:id/workflows`: the absolute directory a
/// new conversation works in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowsQuery {
    pub directory: AbsolutePath,
}
