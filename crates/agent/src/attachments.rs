//! What an uploaded file becomes (`backend.md` § Media by reference,
//! `web-api.md` § Uploads and media): the media type and, for a text file,
//! the opening the upload is recorded with, and the blocks it becomes in a
//! message once the backend has written it to the conversation's Host. The
//! backend's upload route and its content resolution both use these rules;
//! the agent never reads a file's bytes itself.

use demi_core::{
    Attachment, B64Bytes, BlobRef, DocumentSource, MediaSource, ModelMediaKind, UserContentBlock,
    sniff_model_media_type,
};

use crate::{
    images,
    store::{
        StoreError,
        media::{BlobStore, HeldMedia},
    },
    transcript::char_offset,
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
    let medium = match sniff_model_media_type(upload.bytes) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MemoryBlobs;

    const PNG: [u8; 12] = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01,
    ];

    fn upload<'a>(
        name: &'a str,
        media_type: &'a str,
        bytes: &'a B64Bytes,
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

    #[tokio::test]
    async fn an_upload_becomes_its_native_medium_then_its_record() {
        let blobs = MemoryBlobs::new();
        let png = crate::testing::png(4, 3, 1);
        let sha256 = BlobRef::of(&png);
        let (image, image_bytes) =
            upload_blocks(upload("tiny.png", "image/png", &png, &sha256), &*blobs)
                .await
                .unwrap();
        let pdf_bytes = B64Bytes::from(b"%PDF-1.7".to_vec());
        let (pdf, _) = upload_blocks(
            upload("paper.pdf", "application/pdf", &pdf_bytes, &sha256),
            &*blobs,
        )
        .await
        .unwrap();
        let notes = B64Bytes::from(b"\n hello".to_vec());
        let (text, text_bytes) =
            upload_blocks(upload("notes.txt", "text/plain", &notes, &sha256), &*blobs)
                .await
                .unwrap();

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
        // The medium is the upload's own blob, and its bytes come along for
        // the session to hold; a file with no native medium brings none.
        assert_eq!(
            image[0],
            UserContentBlock::Image {
                source: MediaSource::Ref {
                    r#ref: sha256.clone(),
                    media_type: "image/png".into()
                }
            }
        );
        let mut held = HeldMedia::default();
        held.hold(sha256.clone(), png);
        assert_eq!(image_bytes, held);
        assert_eq!(text_bytes, HeldMedia::default());
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

    #[tokio::test]
    async fn an_uploaded_image_enters_fitted_and_one_that_does_not_decode_stays_its_record() {
        let blobs = MemoryBlobs::new();
        // Wider than 2,000 px: the message's image is the fitted one, stored
        // under its own blob; the record keeps the original.
        let wide = crate::testing::png(2_400, 10, 1);
        let wide_blob = BlobRef::of(&wide);
        let (blocks, held) =
            upload_blocks(upload("wide.png", "image/png", &wide, &wide_blob), &*blobs)
                .await
                .unwrap();
        let UserContentBlock::Image {
            source: MediaSource::Ref { r#ref, media_type },
        } = &blocks[0]
        else {
            panic!("{blocks:?}");
        };
        assert_ne!(r#ref, &wide_blob);
        assert_eq!(media_type, "image/png");
        let fitted = blobs.get(r#ref).await.unwrap().expect("the fitted image is stored");
        let mut expected = HeldMedia::default();
        expected.hold(r#ref.clone(), fitted);
        assert_eq!(held, expected);
        let UserContentBlock::Attachment(record) = &blocks[1] else {
            panic!("{blocks:?}");
        };
        assert_eq!(
            (record.size_bytes, &record.sha256),
            (wide.len() as u64, &wide_blob)
        );
        // Bytes that only start like a PNG: the model reads the file by path.
        let broken = B64Bytes::from(PNG.to_vec());
        let broken_blob = BlobRef::of(&broken);
        let (blocks, held) =
            upload_blocks(upload("shot.png", "image/png", &broken, &broken_blob), &*blobs)
                .await
                .unwrap();
        assert!(matches!(blocks.as_slice(), [UserContentBlock::Attachment(_)]));
        assert_eq!(held, HeldMedia::default());
    }
}
