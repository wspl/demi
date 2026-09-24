//! What an uploaded file becomes (`backend.md` § Media by reference,
//! `web-api.md` § Uploads and media): the media type and, for a text file,
//! the opening the upload is recorded with, and the blocks it becomes in a
//! message once the backend has written it to the conversation's Host. The
//! backend's upload route and its content resolution both use these rules;
//! the agent never reads a file's bytes itself.

use demi_core::{Attachment, B64Bytes, BlobRef, DocumentSource, MediaSource, UserContentBlock};

use crate::transcript::char_offset;

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
    if let Some(media) = demi_core::sniff_model_media_type(bytes) {
        return media.media_type.to_owned();
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
    pub bytes: &'a [u8],
}

/// The blocks an upload becomes in a message: the medium a model reads
/// natively when it is an image, a video or a PDF, then the file's record,
/// with its opening when it is text.
pub fn upload_blocks(upload: Upload<'_>) -> Vec<UserContentBlock> {
    let record = UserContentBlock::Attachment(Attachment {
        name: upload.name.to_owned(),
        path: upload.path.to_owned(),
        media_type: upload.media_type.to_owned(),
        size_bytes: upload.bytes.len() as u64,
        sha256: upload.sha256.clone(),
        snippet: is_text(upload.name, upload.media_type).then(|| snippet(upload.bytes)),
    });
    let media = match demi_core::sniff_model_media_type(upload.bytes) {
        Some(media) => {
            let source = MediaSource::Binary {
                data: B64Bytes::new(upload.bytes.to_vec()),
                media_type: media.media_type.to_owned(),
            };
            Some(match media.kind {
                demi_core::ModelMediaKind::Image => UserContentBlock::Image { source },
                demi_core::ModelMediaKind::Video => UserContentBlock::Video { source },
            })
        }
        None if is_pdf(upload.media_type, upload.bytes) => Some(UserContentBlock::Document {
            source: DocumentSource::Binary {
                data: B64Bytes::new(upload.bytes.to_vec()),
                media_type: PDF.to_owned(),
                file_name: upload.name.to_owned(),
            },
        }),
        None => None,
    };
    media.into_iter().chain([record]).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: [u8; 12] = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01,
    ];

    fn blob() -> BlobRef {
        BlobRef::try_from("b".repeat(64)).unwrap()
    }

    fn upload<'a>(
        name: &'a str,
        media_type: &'a str,
        bytes: &'a [u8],
        sha256: &'a BlobRef,
    ) -> Upload<'a> {
        Upload {
            name,
            path: "/home/demi/.demi/attachments/c1/file",
            media_type,
            sha256,
            bytes,
        }
    }

    #[test]
    fn an_upload_is_recorded_with_the_type_its_bytes_show_and_a_text_files_opening() {
        assert_eq!(
            upload_media_type("application/octet-stream", &PNG),
            "image/png"
        );
        assert_eq!(
            upload_media_type("application/octet-stream", b"%PDF-1.7 ..."),
            PDF
        );
        assert_eq!(upload_media_type("text/csv", b"a,b\n1,2"), "text/csv");
        assert!(is_text("notes.MD", "application/octet-stream"));
        assert!(is_text("data", "text/plain"));
        assert!(!is_text("photo.png", "image/png"));
        assert_eq!(
            snippet(b"\r\n  \n first\r\nsecond\rthird"),
            "first\nsecond\nthird"
        );
        let long = "字".repeat(300);
        assert_eq!(snippet(long.as_bytes()), "字".repeat(160));
    }

    #[test]
    fn an_upload_becomes_its_native_medium_then_its_record() {
        let sha256 = blob();
        let image = upload_blocks(upload("tiny.png", "image/png", &PNG, &sha256));
        let pdf = upload_blocks(upload("paper.pdf", "application/pdf", b"%PDF-1.7", &sha256));
        let text = upload_blocks(upload("notes.txt", "text/plain", b"\n hello", &sha256));

        let kinds = |blocks: &[UserContentBlock]| -> Vec<String> {
            blocks
                .iter()
                .map(|block| {
                    serde_json::to_value(block).unwrap()["type"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect()
        };
        assert_eq!(kinds(&image), ["image", "attachment"]);
        assert_eq!(
            image[0],
            UserContentBlock::Image {
                source: MediaSource::Binary {
                    data: B64Bytes::new(PNG.to_vec()),
                    media_type: "image/png".into()
                }
            }
        );
        assert_eq!(kinds(&pdf), ["document", "attachment"]);
        assert_eq!(kinds(&text), ["attachment"]);
        let UserContentBlock::Attachment(record) = &text[0] else {
            unreachable!()
        };
        assert_eq!(
            (
                record.name.as_str(),
                record.size_bytes,
                record.snippet.as_deref()
            ),
            ("notes.txt", 7, Some("hello"))
        );
        let UserContentBlock::Attachment(record) = &image[1] else {
            unreachable!()
        };
        assert_eq!(record.snippet, None);
        assert_eq!(
            unavailable("upload-9"),
            UserContentBlock::Text {
                text: "[attachment upload-9 is not available]".into()
            }
        );
    }
}
