//! The sidebar's explicit order (`web-api.md` § Sidebar mutations, read
//! state and page synchronization): a move of one row before another, or to
//! the end.

use demi_core::Nullable;
use garde::Validate;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::ids::{ConversationId, WorkspaceId};

/// `POST /sidebar/reorder`: the row `id` moves before `beforeId`, or to the
/// end of its partition when that is null. A conversation moves within its
/// project and pin partition; a workspace among the user's workspaces.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SidebarReorder {
    Conversation {
        #[garde(skip)]
        id: ConversationId,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<ConversationId>")]
        #[garde(skip)]
        before_id: Option<ConversationId>,
    },
    Workspace {
        #[garde(skip)]
        id: WorkspaceId,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<WorkspaceId>")]
        #[garde(skip)]
        before_id: Option<WorkspaceId>,
    },
}
