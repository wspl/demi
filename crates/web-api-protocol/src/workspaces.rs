//! Workspaces: a directory on one of the user's devices, which conversations
//! can target and the page calls a project, named by the directory. A
//! workspace is a pointer: deleting it touches no file.

use demi_shared_types::{MAX_SAFE_INTEGER, Timestamp, WorkflowChoice};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::conversations::ModelSettings;
use crate::files::AbsolutePath;
use crate::ids::{ConversationId, DeviceId, WorkspaceId};
use crate::text::Trimmed;


/// A workspace as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub id: WorkspaceId,
    pub device_id: DeviceId,
    /// The directory on the device, absolute as the device names it.
    pub path: String,
    /// The directory's last component, or the directory when it is a root.
    pub name: String,
    pub created_at: Timestamp,
    /// The model settings the project's next conversation starts with, as
    /// last chosen in it; absent before the first choice.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ModelSettings")]
    pub last_model: Option<ModelSettings>,
    /// The Workflow menu choice the project's next conversation starts
    /// with; absent before the first choice.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "WorkflowChoice")]
    pub last_workflow: Option<WorkflowChoice>,
    /// How its conversations keep their messages saved for later.
    pub saved_scope: SavedScope,
    /// How many messages the project's own list keeps; 0 while it keeps
    /// them per conversation.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub saved_count: u64,
}

/// How a project's conversations keep their messages saved for later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SavedScope {
    /// Each conversation keeps a list of its own.
    Conversation,
    /// The project keeps one list, which all its conversations show.
    Project,
}

/// `PUT /workspaces/:id/saved-scope`: the way the project's conversations
/// keep their saved messages from now on. To per conversation, `to` names
/// the conversation of the project, not archived, that takes the project's
/// list, which a list with messages needs.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedScopeSwitch {
    #[garde(skip)]
    pub scope: SavedScope,
    #[serde(default)]
    #[garde(skip)]
    pub to: Option<ConversationId>,
}

/// `GET /workspaces`: the user's workspaces, in their sidebar order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Workspaces {
    pub workspaces: Vec<WorkspaceDto>,
}

/// `{ workspace }`: the answer of a creation and a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceAnswer {
    pub workspace: WorkspaceDto,
}

/// `POST /workspaces`: a directory on one of the user's devices, or a new
/// project directory on the user's Cloud, which can wake it.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreateWorkspace {
    Device {
        #[garde(skip)]
        device_id: DeviceId,
        /// An `AbsolutePath` is valid once it is decoded.
        #[garde(skip)]
        path: AbsolutePath,
    },
    /// The directory `~/projects/<name>` on the Cloud, which the creation
    /// makes.
    Cloud {
        #[garde(length(chars, min = 1))]
        name: Trimmed,
    },
}

/// `PATCH /workspaces/:id`: the model settings or workflow last chosen in
/// the project; each field replaces its value.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeWorkspace {
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "ModelSettings")]
    #[garde(dive)]
    pub last_model: Option<ModelSettings>,
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "WorkflowChoice")]
    #[garde(dive)]
    pub last_workflow: Option<WorkflowChoice>,
}
