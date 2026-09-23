//! How served file bytes stay inert (`file-previews.md` § Keeping file
//! content inert): a media type the page shows in place is served as itself,
//! an image under a policy that keeps an SVG inert, and anything else
//! downloads as `application/octet-stream`. Every answer says `nosniff`.

use axum::http::header::{CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderMap, HeaderValue};

/// An image served in place runs no script and fetches nothing, in an opaque
/// origin.
const IMAGE_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/// The headers that serve bytes as `media_type`, the type a request asks to
/// see them as: that type when the page shows it in place, a download
/// otherwise.
pub(super) fn content_headers(media_type: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    match media_type.filter(|media_type| demi_core::shows_in_place(media_type)) {
        Some(media_type) => {
            if media_type.starts_with("image/") {
                headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(IMAGE_POLICY));
            }
            let value = HeaderValue::from_str(media_type).expect("the file-type table's media types are header values");
            headers.insert(CONTENT_TYPE, value);
        }
        None => {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
            headers.insert(CONTENT_DISPOSITION, HeaderValue::from_static("attachment"));
        }
    }
    headers
}
