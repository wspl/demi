//! `GET /api/search?q=` (`web-api.md` § Search): the caller's conversations
//! that match, read from the caller's search index alone. A query outside
//! its bounds answers 400 `invalid_query`.

use axum::Json;
use axum::extract::State;
use demi_backend_user_shard::conversation::search;
use demi_web_api_protocol::search::{SearchQuery, SearchResults};

use super::AppState;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;

pub(super) async fn search(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    QueryParams(SearchQuery { q }): QueryParams<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let results = search::search(&state.services, &user.id, &q).await?;
    Ok(Json(SearchResults { results }))
}
