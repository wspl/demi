//! `POST /api/attachments?name=&sha256=`: a file's bytes, at most 25 MiB, into
//! the caller's blobs with one attachment record.
//!
//! - With a SHA-256 and no body, the blob the caller holds already gains the
//!   record, and no bytes travel.
//! - A blob the caller does not hold answers 200 with a null attachment.
//!
//! The answer carries the media type the backend reads from the bytes and, for
//! a text file, its opening, which the composer shows on the file's capsule as
//! the message will carry it.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use demi_agent_store::attachments::{is_text, snippet, upload_media_type};
use demi_shared_types::BlobRef;
use demi_web_api_protocol::attachments::{
    ATTACHMENT_MAX_BYTES, AttachmentAnswer, AttachmentCheckAnswer, AttachmentDto, UploadQuery,
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;
use garde::Validate as _;

use super::body::read_within;
use super::error::ApiError;
use super::gate::AuthUser;
use super::query::QueryParams;
use demi_backend_user_shard::services::Services;

fn too_large() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::TooLarge,
        format!("An upload is at most {ATTACHMENT_MAX_BYTES} bytes"),
    )
    .with_param("limitBytes", ATTACHMENT_MAX_BYTES)
}

/// The media type `headers` send the upload's bytes as: one `type/subtype`,
/// with its parameters. A `multipart/*` form is an envelope, not a file's
/// bytes.
fn sent_media_type(headers: &HeaderMap) -> Result<String, ApiError> {
    let refused =
        || ApiError::invalid_body("Send the file's bytes with its media type as Content-Type");
    let value = headers.get(CONTENT_TYPE).ok_or_else(refused)?;
    let media_type: mime::Mime = value
        .to_str()
        .map_err(|_| refused())?
        .parse()
        .map_err(|_| refused())?;
    if media_type.type_() == mime::MULTIPART {
        return Err(refused());
    }
    Ok(media_type.to_string())
}

/// The route's body limit, which the router gives this route instead of
/// the JSON bodies'.
pub(super) const BODY_LIMIT: usize = ATTACHMENT_MAX_BYTES;

pub(super) async fn upload(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    QueryParams(query): QueryParams<UploadQuery>,
    request: Request,
) -> Result<Response, ApiError> {
    query
        .validate()
        .map_err(|report| ApiError::invalid_query(format!("name: {report}")))?;
    let sent = sent_media_type(request.headers())?;
    // The body is read only after the checks above, so a refusal of the
    // query or the media type, or of a declared length over the limit,
    // comes before a byte of it.
    let bytes = read_within(request, &services, BODY_LIMIT, too_large).await?;
    let blobs = services.blobs.for_user(&user.id);
    let uploaded = async |opening: Bytes, size_bytes: u64, sha256| {
        record(&services, &user.id, &query.name, &sent, &opening, size_bytes, sha256).await
    };
    match (bytes.is_empty(), query.sha256.clone()) {
        (true, None) => Err(ApiError::invalid_body("An upload holds at least one byte")),
        // No bytes travel for a blob the caller holds already; only the
        // caller's own namespace is asked, and only its opening is read.
        // A blob it does not hold is an answer too, with no upload.
        (true, Some(sha256)) => Ok(match blobs.opening(&sha256).await? {
            Some(held) => {
                let attachment = uploaded(held.bytes, held.size, sha256).await?;
                let answer = AttachmentCheckAnswer { attachment: Some(attachment) };
                (StatusCode::CREATED, Json(answer)).into_response()
            }
            None => {
                let answer = AttachmentCheckAnswer { attachment: None };
                (StatusCode::OK, Json(answer)).into_response()
            }
        }),
        (false, Some(sha256)) => {
            if !blobs.put_named(bytes.clone(), &sha256).await? {
                return Err(ApiError::invalid_body(
                    "The bytes do not have the SHA-256 the request named",
                ));
            }
            let size = bytes.len() as u64;
            let attachment = uploaded(bytes, size, sha256).await?;
            Ok((StatusCode::CREATED, Json(AttachmentAnswer { attachment })).into_response())
        }
        (false, None) => {
            let sha256 = blobs.put(bytes.clone()).await?;
            let size = bytes.len() as u64;
            let attachment = uploaded(bytes, size, sha256).await?;
            Ok((StatusCode::CREATED, Json(AttachmentAnswer { attachment })).into_response())
        }
    }
}

/// Records an upload of the blob `sha256` names, read from `opening`, the
/// file's opening or all of it: the media type the bytes say, else the one
/// they were sent as, and a text file's snippet.
async fn record(
    services: &Services,
    user: &UserId,
    name: &str,
    sent: &str,
    opening: &[u8],
    size_bytes: u64,
    sha256: BlobRef,
) -> Result<AttachmentDto, ApiError> {
    let media_type = upload_media_type(sent, opening);
    // The record keeps the snippet, so a draft that names the upload shows
    // it on every page.
    let snippet = is_text(name, &media_type).then(|| snippet(opening));
    let record = services
        .control
        .create_attachment(user.clone(), media_type, size_bytes, sha256, snippet)
        .await?;
    Ok(AttachmentDto {
        id: record.id,
        media_type: record.media_type,
        size_bytes: record.size_bytes,
        sha256: record.sha256,
        created_at: record.created_at,
        snippet: record.snippet,
    })
}
