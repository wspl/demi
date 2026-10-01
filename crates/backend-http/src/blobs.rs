//! `GET /api/blobs/:sha256` (`backend.md` § Media by reference): the bytes a
//! media reference names, from the caller's own namespace. A name is a
//! content hash, so the answer never changes and the user's browser keeps
//! it for a year; the cache is private and varies by cookie, since the bytes
//! are the caller's. A byte range is answered as `fs/raw` answers one
//! (`web-api.md` § Uploads and media): Safari plays a video only from a
//! server that does.

use std::sync::Arc;

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{CACHE_CONTROL, RANGE, VARY};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_shared_types::BlobRef;
use demi_web_api_protocol::error::ErrorCode;

use super::content::content_headers;
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_host_access::transfer::RangeAnswer;
use demi_backend_user_shard::services::Services;

pub(super) async fn blob(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    name: Result<Path<String>, PathRejection>,
    RawQuery(query): RawQuery,
    request: HeaderMap,
) -> Result<Response, ApiError> {
    // A name that is not a lowercase SHA-256 names no blob.
    let blob = name
        .ok()
        .and_then(|Path(name)| BlobRef::try_from(name).ok())
        .ok_or_else(no_such_blob)?;
    let bytes = services
        .blobs
        .for_user(&user.id)
        .get(&blob)
        .await?
        .ok_or_else(no_such_blob)?;
    // `type` asks to see the bytes as that media type, and is not validated:
    // one the page does not show in place leaves the blob a download.
    let requested = query.as_deref().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "type")
            .map(|(_, media_type)| media_type.into_owned())
    });
    let size = u64::try_from(bytes.len()).expect("a blob held in memory fits u64");
    let range = request.get(RANGE).and_then(|value| value.to_str().ok());
    let part = RangeAnswer::of(range, size);
    let mut headers = content_headers(requested.as_deref(), false, None);
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    headers.insert(VARY, HeaderValue::from_static("Cookie"));
    headers.extend(part.headers());
    Ok(match part.part_of(&bytes) {
        Some(body) => (part.status(), headers, body).into_response(),
        None => (part.status(), headers).into_response(),
    })
}

fn no_such_blob() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "No such blob")
}
