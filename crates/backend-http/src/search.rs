//! `GET /api/search?q=`: the caller's conversations that match, and
//! `GET /api/conversations/{id}/search?q=`: the messages of one of them that
//! match, each read from the caller's search index alone. A query outside its
//! bounds answers 400 `invalid_query`.

use axum::Json;
use axum::extract::{Path, State};
use demi_backend_user_shard::conversation::search;
use demi_web_api_protocol::search::{MessageResults, SearchQuery, SearchResults};

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;

pub(super) async fn search(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    QueryParams(SearchQuery { q, workspace }): QueryParams<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let results = search::search(&state.services, &user.id, &q, workspace.as_ref()).await?;
    Ok(Json(SearchResults { results }))
}

/// `GET /api/conversations/{id}/search?q=`; a conversation not the caller's
/// answers 404.
pub(super) async fn search_in(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    QueryParams(SearchQuery { q, .. }): QueryParams<SearchQuery>,
) -> Result<Json<MessageResults>, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    Ok(Json(search::search_in(&state.services, &user.id, &record.id, &q).await?))
}
