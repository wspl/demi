//! What an uploaded file becomes.

use demi_agent_store::{
    attachments::{Upload, is_text, snippet, upload_blocks, upload_media_type},
    media::{BlobStore, HeldMedia},
    testing::{MemoryBlobs, png},
};
use demi_shared_types::{B64Bytes, BlobRef, MediaSource, UserContentBlock};

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
        id: "a1",
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
        source:
            MediaSource::Ref {
                r#ref,
                media_type,
                width,
                height,
            },
    } = &blocks[0]
    else {
        panic!("{blocks:?}");
    };
    assert_ne!(r#ref, &wide_blob);
    assert_eq!(media_type, "image/png");
    // Its reference carries the size it entered at.
    assert_eq!((*width, *height), (Some(2_000), Some(8)));
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
    // Bytes that only start like a PNG: the model fetches the file with demi attachment download.
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
