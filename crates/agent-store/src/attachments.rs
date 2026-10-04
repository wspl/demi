//! What an uploaded file becomes (`backend.md` § Media by reference,
//! `web-api.md` § Uploads and media): the media type and, for a text file,
//! the opening the upload is recorded with, and the blocks it becomes in a
//! message once the backend has written it to the conversation's Host. The
//! backend's upload route and its content resolution both use these rules;
//! the agent never reads a file's bytes itself.

use demi_command_protocol::sniff_media_type;
use demi_shared_types::{
    Attachment, B64Bytes, BlobRef, DocumentSource, MediaSource, ModelMediaKind, UserContentBlock,
    char_offset, model_media_type_for,
};

use crate::{
    StoreError, images,
    media::{BlobStore, HeldMedia},
};

/// How much of a text file's opening a record keeps, in Unicode scalar
/// values.
pub const SNIPPET_MAX_CHARS: usize = 160;

/// How much of a text file is read for its opening.
const SNIPPET_READ_BYTES: usize = 4_096;

/// The extensions of files that are text whatever their media type says.
const TEXT_EXTENSIONS: [&str; 11] = [
    "txt", "md", "markdown", "log", "csv", "json", "yaml", "yml", "xml", "ini", "toml",
];

const PDF: &str = "application/pdf";

/// Whether the file `name` of `media_type` is text, which its record shows
/// by its opening.
pub fn is_text(name: &str, media_type: &str) -> bool {
    if media_type.starts_with("text/") {
        return true;
    }
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        TEXT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
    })
}

/// The opening of a text file whose bytes begin with `bytes`: leading blank
/// space dropped, line endings normalized, cut to [`SNIPPET_MAX_CHARS`].
pub fn snippet(bytes: &[u8]) -> String {
    let read = &bytes[..bytes.len().min(SNIPPET_READ_BYTES)];
    let text = String::from_utf8_lossy(read)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let opening = text.trim_start();
    opening[..char_offset(opening, SNIPPET_MAX_CHARS)].to_owned()
}

/// The media type an upload is recorded with: the one its bytes show when a
/// model can read them natively, as an image, a video or a PDF, else the one
/// it was sent with.
pub fn upload_media_type(sent: &str, bytes: &[u8]) -> String {
    if let Some(media) = sniff_media_type(bytes) {
        return media.to_owned();
    }
    if bytes.starts_with(b"%PDF-") {
        return PDF.to_owned();
    }
    sent.to_owned()
}

/// An upload written to the conversation's Host, as its message receives it.
#[derive(Debug, Clone, Copy)]
pub struct Upload<'a> {
    /// The name it was written under.
    pub name: &'a str,
    /// Where it was written, absolute, on the conversation's Host.
    pub path: &'a str,
    /// The media type it was recorded with ([`upload_media_type`]).
    pub media_type: &'a str,
    pub sha256: &'a BlobRef,
    pub bytes: &'a B64Bytes,
}

/// The blocks an upload becomes in a message: the medium a model reads
/// natively when it is an image, a video or a PDF, then the file's record,
/// with its opening when it is text. The medium references the upload's own
/// blob, and its bytes come with the blocks, for the session to hold
/// (`runtime.md` § Media). An image enters fitted (`runtime.md` § Images in
/// the transcript): one that fitting changed references the fitted image's
/// blob, which is put into `blobs` first, and one no provider can take
/// stays the record alone, which the model reads by path.
pub async fn upload_blocks(
    upload: Upload<'_>,
    blobs: &dyn BlobStore,
) -> Result<(Vec<UserContentBlock>, HeldMedia), StoreError> {
    let record = UserContentBlock::Attachment(Attachment {
        name: upload.name.to_owned(),
        path: upload.path.to_owned(),
        media_type: upload.media_type.to_owned(),
        size_bytes: upload.bytes.len() as u64,
        sha256: upload.sha256.clone(),
        snippet: is_text(upload.name, upload.media_type).then(|| snippet(upload.bytes)),
    });
    let mut held = HeldMedia::default();
    let medium = match sniff_media_type(upload.bytes).and_then(model_media_type_for) {
        Some(media) if media.kind == ModelMediaKind::Image => {
            match images::fit(upload.bytes.clone(), media.media_type).await {
                Ok(fitted) => {
                    let blob = if fitted.reencoded {
                        blobs.put(fitted.data.clone()).await?
                    } else {
                        upload.sha256.clone()
                    };
                    held.hold(blob.clone(), fitted.data);
                    Some(UserContentBlock::Image {
                        source: MediaSource::Ref {
                            r#ref: blob,
                            media_type: fitted.media_type.to_owned(),
                        },
                    })
                }
                Err(_) => None,
            }
        }
        Some(media) => {
            held.hold(upload.sha256.clone(), upload.bytes.clone());
            Some(UserContentBlock::Video {
                source: MediaSource::Ref {
                    r#ref: upload.sha256.clone(),
                    media_type: media.media_type.to_owned(),
                },
            })
        }
        None if is_pdf(upload.media_type, upload.bytes) => {
            held.hold(upload.sha256.clone(), upload.bytes.clone());
            Some(UserContentBlock::Document {
                source: DocumentSource::Ref {
                    r#ref: upload.sha256.clone(),
                    media_type: PDF.to_owned(),
                    file_name: upload.name.to_owned(),
                },
            })
        }
        None => None,
    };
    Ok((medium.into_iter().chain([record]).collect(), held))
}

fn is_pdf(media_type: &str, bytes: &[u8]) -> bool {
    media_type.split(';').next().map(str::trim) == Some(PDF) || bytes.starts_with(b"%PDF-")
}

/// The text an upload that is gone, or not the sender's, becomes.
pub fn unavailable(reference: &str) -> UserContentBlock {
    UserContentBlock::Text {
        text: format!("[attachment {reference} is not available]"),
    }
}
