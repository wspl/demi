//! `POST /sidebar/reorder`: the user's explicit order, one move at a time,
//! within the moved row's partition.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use demi_core_store::sidebar::Reorder;
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
    let store = services.stores.of(&user.id).await?;
    let (reorder, changed, missing) = match request {
        SidebarReorder::Conversation { id, before_id } => (
            store
                .reorder_conversations(id, before_id)
                .await?,
            Part::ConversationOrder,
            (ErrorCode::ConversationNotFound, "No such conversation"),
        ),
        SidebarReorder::Project { id, before_id } => (
            store
                .reorder_projects(id, before_id)
                .await?,
            Part::Projects,
            (ErrorCode::ProjectNotFound, "No such project"),
        ),
    };
    match reorder {
        Reorder::Moved => {}
        // A row another page deleted meanwhile: the page's move lost a race.
        Reorder::Missing => {
            let (code, message) = missing;
            return Err(ApiError::new(StatusCode::NOT_FOUND, code, message));
        }
        Reorder::OutOfPartition => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::InvalidOrder,
                "Rows must belong to the same project and pin partition",
            ));
        }
    }
    services.sync.mark(&user.id, changed);
    Ok(StatusCode::NO_CONTENT)
}
