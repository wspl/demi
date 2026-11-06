//! `GET /api/search?q=`: the caller's conversations that match, and
//! `GET /api/conversations/{id}/search?q=`: the messages of one of them that
//! match, each read from the caller's search index alone. A query outside its
//! bounds answers 400 `invalid_query`.

use axum::Json;
use axum::extract::{Path, State};
use demi_core::conversation::search;
use demi_web_api_protocol::search::{MessageResults, SearchQuery, SearchResults};

use crate::api::Api;
use crate::conversations::owned;
use crate::error::ApiError;
use crate::caller::Principal;
use crate::query::QueryParams;

pub(crate) async fn search(
    State(state): State<Api>,
    Principal(user): Principal,
    QueryParams(SearchQuery { q, workspace }): QueryParams<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let results = search::search(&state.store(&user).await?, &q, workspace.as_ref()).await?;
    Ok(Json(SearchResults { results }))
}

/// `GET /api/conversations/{id}/search?q=`; a conversation not the caller's
/// answers 404.
pub(crate) async fn search_in(
    State(state): State<Api>,
    Principal(user): Principal,
    Path(id): Path<String>,
    QueryParams(SearchQuery { q, .. }): QueryParams<SearchQuery>,
) -> Result<Json<MessageResults>, ApiError> {
    let record = owned(&state.services, &user, &id).await?;
    Ok(Json(search::search_in(&state.store(&user).await?, &record.id, &q).await?))
}
