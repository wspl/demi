//! `GET /conversations/:id/cost/recent?window=hour|day`: what the
//! conversation's calls of the last 60 minutes or 24 hours cost, summed from
//! the usage ledger when asked.

use axum::Json;
use axum::extract::{Path, Query, State};
use demi_web_api_protocol::conversations::{CostWindow, RecentCost};
use serde::Deserialize;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;

/// The query of `GET /conversations/:id/cost/recent`.
#[derive(Debug, Deserialize)]
pub(super) struct RecentQuery {
    window: CostWindow,
}

/// `GET /conversations/:id/cost/recent`.
pub(super) async fn recent(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    Query(query): Query<RecentQuery>,
) -> Result<Json<RecentCost>, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let agents = state.services.control.conversation_recent(record.id, query.window).await?;
    Ok(Json(RecentCost { agents }))
}
