//! What an uploaded file becomes (`web-api.md` § Uploads and media).

use demi_agent_store::{
    attachments::{Upload, is_text, snippet, unavailable, upload_blocks, upload_media_type},
    media::{BlobStore, HeldMedia},
    testing::{MemoryBlobs, png},
};
use demi_core::{B64Bytes, BlobRef, MediaSource, UserContentBlock};

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
        "application/pdf"
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
    let png = png(4, 3, 1);
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
    let wide = png(2_400, 10, 1);
    let wide_blob = BlobRef::of(&wide);
    let (blocks, held) = upload_blocks(upload("wide.png", "image/png", &wide, &wide_blob), &*blobs)
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
    let fitted = blobs
        .get(r#ref)
        .await
        .unwrap()
        .expect("the fitted image is stored");
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
    let (blocks, held) = upload_blocks(
        upload("shot.png", "image/png", &broken, &broken_blob),
        &*blobs,
    )
    .await
    .unwrap();
    assert!(matches!(
        blocks.as_slice(),
        [UserContentBlock::Attachment(_)]
    ));
    assert_eq!(held, HeldMedia::default());
}
