//! `GET /conversations/:id/cost/recent?window=hour|day`: what the
//! conversation's calls of the last 60 minutes or 24 hours cost, summed from
//! the usage ledger when asked.

use axum::Json;
use axum::extract::{Path, Query, State};
use demi_web_api_protocol::conversations::{CostWindow, RecentCost};
use serde::Deserialize;

use crate::api::Api;
use crate::conversations::owned;
use crate::error::ApiError;
use crate::caller::Principal;

/// The query of `GET /conversations/:id/cost/recent`.
#[derive(Debug, Deserialize)]
pub(crate) struct RecentQuery {
    window: CostWindow,
}

/// `GET /conversations/:id/cost/recent`.
pub(crate) async fn recent(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
    Query(query): Query<RecentQuery>,
) -> Result<Json<RecentCost>, ApiError> {
    let record = owned(&state.services, &user, &id).await?;
    let agents = state.store(&user).await?.conversation_recent(record.id, query.window).await?;
    Ok(Json(RecentCost { agents }))
}
