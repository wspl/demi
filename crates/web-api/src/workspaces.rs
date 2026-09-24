//! Workspaces (`web-api.md` § Workspaces, devices, and attached hosts): a
//! named directory on one of the user's devices, which conversations can
//! target and the page calls a project. A workspace is a pointer: renaming or
//! deleting it touches no file.

use demi_core::Timestamp;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::files::AbsolutePath;
use crate::ids::{DeviceId, WorkspaceId};
use crate::text::Trimmed;

/// The most characters of a workspace's name.
pub const WORKSPACE_NAME_MAX: usize = 256;

/// A workspace as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub id: WorkspaceId,
    pub device_id: DeviceId,
    /// The directory on the device, absolute as the device names it.
    pub path: String,
    pub name: String,
    pub created_at: Timestamp,
}

/// `GET /workspaces`: the user's workspaces, in their sidebar order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Workspaces {
    pub workspaces: Vec<WorkspaceDto>,
}

/// `{ workspace }`: the answer of a creation and a rename.
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
        #[garde(length(chars, min = 1, max = WORKSPACE_NAME_MAX))]
        name: Trimmed,
    },
    Cloud {
        #[garde(length(chars, min = 1, max = WORKSPACE_NAME_MAX))]
        name: Trimmed,
    },
}

/// `PATCH /workspaces/:id`: a new name.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct RenameWorkspace {
    #[garde(length(chars, min = 1, max = WORKSPACE_NAME_MAX))]
    pub name: Trimmed,
}
