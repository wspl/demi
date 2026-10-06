//! `POST /api/attachments?name=&sha256=` (`web-api.md` § Uploads and
//! media): a file's bytes, at most 25 MiB, into the caller's blobs with one
//! attachment record; with a SHA-256 and no body, the blob the caller holds
//! already gains the record, and no bytes travel. The answer carries the
//! media type the backend reads from the bytes and, for a text file, its
//! opening, which the composer shows on the file's capsule as the message
//! will carry it.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::{BytesRejection, FailedToBufferBody};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use demi_agent_store::attachments::{is_text, snippet, upload_media_type};
use demi_web_api_protocol::attachments::{
    ATTACHMENT_MAX_BYTES, AttachmentAnswer, AttachmentDto, UploadQuery,
};
use demi_web_api_protocol::error::ErrorCode;
use garde::Validate as _;

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
/// the JSON bodies' (`web-api.md` § Request bodies).
pub(super) const BODY_LIMIT: usize = ATTACHMENT_MAX_BYTES;

pub(super) async fn upload(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    QueryParams(query): QueryParams<UploadQuery>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<AttachmentAnswer>), ApiError> {
    query
        .validate()
        .map_err(|report| ApiError::invalid_query(format!("name: {report}")))?;
    let sent = sent_media_type(&headers)?;
    // A declared length over the limit is refused before a byte is read.
    let declared = headers
        .get(CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse::<u64>().ok());
    if declared.is_some_and(|length| length > ATTACHMENT_MAX_BYTES as u64) {
        return Err(too_large());
    }
    let bytes = body.map_err(|rejection| match rejection {
        BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => too_large(),
        other => ApiError::invalid_body(other.body_text()),
    })?;
    let blobs = services.blobs.for_user(&user.id);
    // What the record is read from: the file's opening, or all of it, and
    // its size.
    let (opening, size_bytes, sha256) = match (bytes.is_empty(), query.sha256) {
        (true, None) => return Err(ApiError::invalid_body("An upload holds at least one byte")),
        // No bytes travel for a blob the caller holds already; only the
        // caller's own namespace is asked, and only its opening is read.
        (true, Some(sha256)) => match blobs.opening(&sha256).await? {
            Some(held) => (held.bytes, held.size, sha256),
            None => {
                return Err(ApiError::new(
                    StatusCode::NOT_FOUND,
                    ErrorCode::BlobMissing,
                    "Your files hold no blob with that SHA-256; send its bytes",
                ));
            }
        },
        (false, Some(sha256)) => {
            if !blobs.put_named(bytes.clone(), &sha256).await? {
                return Err(ApiError::invalid_body(
                    "The bytes do not have the SHA-256 the request named",
                ));
            }
            let size = bytes.len() as u64;
            (bytes, size, sha256)
        }
        (false, None) => {
            let sha256 = blobs.put(bytes.clone()).await?;
            let size = bytes.len() as u64;
            (bytes, size, sha256)
        }
    };
    let media_type = upload_media_type(&sent, &opening);
    // The record keeps the snippet, so a draft that names the upload shows
    // it on every page (`web-api.md` § Conversation drafts).
    let snippet = is_text(&query.name, &media_type).then(|| snippet(&opening));
    let record = services
        .control
        .create_attachment(user.id.clone(), media_type, size_bytes, sha256, snippet)
        .await?;
    let attachment = AttachmentDto {
        id: record.id,
        media_type: record.media_type,
        size_bytes: record.size_bytes,
        sha256: record.sha256,
        created_at: record.created_at,
        snippet: record.snippet,
    };
    Ok((StatusCode::CREATED, Json(AttachmentAnswer { attachment })))
}
