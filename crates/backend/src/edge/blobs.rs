//! `GET /api/blobs/:sha256` (`backend.md` § Media by reference): the bytes a
//! media reference names, from the caller's own namespace. A name is a
//! content hash, so the answer never changes and the browser keeps it for a
//! year; the cache is private and varies by cookie, since the bytes are the
//! caller's.

use std::sync::Arc;

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{CACHE_CONTROL, VARY};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_core::BlobRef;
use demi_web_api::error::ErrorCode;

use super::content::content_headers;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::backend::Services;

pub(super) async fn blob(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    name: Result<Path<String>, PathRejection>,
    RawQuery(query): RawQuery,
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
    let mut headers = content_headers(requested.as_deref(), false, None);
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    headers.insert(VARY, HeaderValue::from_static("Cookie"));
    Ok((headers, bytes).into_response())
}

fn no_such_blob() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "No such blob")
}
