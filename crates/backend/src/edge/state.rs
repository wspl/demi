//! `GET /api/state` (`web-api.md` § Sidebar mutations, read state and page
//! synchronization): the product snapshot under a private ETag, the quoted
//! SHA-256 of the exact body. The page revalidates with `If-None-Match`,
//! which answers 304 while the body is unchanged.

use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use super::error::ApiError;
use super::gate::AuthUser;
use crate::shard::Shards;

pub(super) async fn state(
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    request: HeaderMap,
) -> Result<Response, ApiError> {
    let owner = user.id.clone();
    let state = shards
        .of(&owner)
        .call(move |shard, _| async move { shard.product_state(user).await })
        .await??;
    // The snapshot's types serialize their fields as JSON strings, numbers,
    // arrays and objects with string keys, which serde_json never refuses.
    let body = serde_json::to_vec(&state).expect("the product state serializes to JSON");
    let etag = HeaderValue::from_str(&format!("\"{}\"", hex::encode(Sha256::digest(&body))))
        .expect("a quoted hexadecimal digest is a header value");
    let cache = (CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    if request.get(IF_NONE_MATCH) == Some(&etag) {
        return Ok((StatusCode::NOT_MODIFIED, [(ETAG, etag), cache]).into_response());
    }
    let json = (CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(([(ETAG, etag), cache, json], body).into_response())
}
