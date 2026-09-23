//! Transcript media by reference (`backend.md` § Media by reference): the
//! blob route serves the caller's own blobs, inert and cached for good.

use demi_web_api::error::ErrorCode;
use reqwest::StatusCode;
use sha2::{Digest, Sha256};

use crate::support::Harness;

/// Stores `bytes` in `user`'s namespace where the object store of the data
/// directory keeps them (`storage.md` § Ownership and layout), and answers
/// their name.
fn store_blob(harness: &Harness, user: &str, bytes: &[u8]) -> String {
    let name = hex::encode(Sha256::digest(bytes));
    let directory = harness.data_dir().join("blobs").join(user);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(&name), bytes).unwrap();
    name
}

#[tokio::test]
async fn a_blob_is_served_from_the_callers_namespace_inert_and_immutable() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let bytes = b"\x89PNG\r\n\x1a\n not really an image";
    let name = store_blob(&harness, master.user.id.as_str(), bytes);

    let download = backend.get(&format!("/api/blobs/{name}"), Some(&master)).await;
    assert_eq!(download.status, StatusCode::OK);
    assert_eq!(download.body, bytes);
    assert_eq!(download.headers["cache-control"], "private, max-age=31536000, immutable");
    assert_eq!(download.headers["vary"], "Cookie");
    assert_eq!(download.headers["x-content-type-options"], "nosniff");
    assert_eq!(download.headers["content-type"], "application/octet-stream");
    assert_eq!(download.headers["content-disposition"], "attachment");

    let image = backend.get(&format!("/api/blobs/{name}?type=image%2Fpng"), Some(&master)).await;
    assert_eq!(image.headers["content-type"], "image/png");
    assert_eq!(
        image.headers["content-security-policy"],
        "default-src 'none'; style-src 'unsafe-inline'; sandbox"
    );
    assert!(image.headers.get("content-disposition").is_none());
    let video = backend.get(&format!("/api/blobs/{name}?type=video/mp4"), Some(&master)).await;
    assert_eq!(video.headers["content-type"], "video/mp4");
    assert!(video.headers.get("content-security-policy").is_none());
    // A type the page does not show in place leaves the blob a download.
    let page = backend.get(&format!("/api/blobs/{name}?type=text/html"), Some(&master)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.headers["content-type"], "application/octet-stream");
    assert_eq!(page.headers["content-disposition"], "attachment");

    // Another user's name reaches nothing, and neither does a malformed one.
    let theirs = store_blob(&harness, "someone-else", b"their bytes");
    for missing in [theirs, name.to_uppercase(), "not-a-hash".to_owned(), "0".repeat(64)] {
        let refused = backend.get(&format!("/api/blobs/{missing}"), Some(&master)).await;
        assert_eq!(refused.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound), "{missing}");
    }
    let anonymous = backend.get(&format!("/api/blobs/{name}"), None).await;
    assert_eq!(anonymous.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));
    backend.close().await;
}
