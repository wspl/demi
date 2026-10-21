//! `GET /conversations/:id/cost/last-hour`: what the conversation's calls of
//! the last 60 minutes cost, summed from the usage ledger when asked
//! (`docs/decisions/0041-the-cost-shows-the-last-hour.md`).

use axum::Json;
use axum::extract::{Path, State};
use demi_web_api_protocol::conversations::LastHourCost;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /conversations/:id/cost/last-hour`.
pub(super) async fn last_hour(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<LastHourCost>, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let agents = state.services.control.conversation_last_hour(record.id).await?;
    Ok(Json(LastHourCost { agents }))
}
