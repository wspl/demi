//! `POST /sidebar/reorder` (`web-api.md` § Sidebar mutations, read state and
//! page synchronization): the user's explicit order, one move at a time,
//! within the moved row's partition.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::sidebar::SidebarReorder;

use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::services::Services;

pub(super) async fn reorder(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(request): JsonBody<SidebarReorder>,
) -> Result<StatusCode, ApiError> {
    let control = &services.control;
    let (moved, changed) = match request {
        SidebarReorder::Conversation { id, before_id } => (
            control
                .reorder_conversations(user.id.clone(), id, before_id)
                .await?,
            Part::ConversationOrder,
        ),
        SidebarReorder::Workspace { id, before_id } => (
            control
                .reorder_workspaces(user.id.clone(), id, before_id)
                .await?,
            Part::Workspaces,
        ),
    };
    if !moved {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::InvalidOrder,
            "Rows must belong to the same project and pin partition",
        ));
    }
    services.sync.mark(&user.id, changed);
    Ok(StatusCode::NO_CONTENT)
}
