//! How served file bytes stay inert.
//!
//! - A media type the page shows in place is served as itself, an image under a
//!   policy that keeps an SVG inert.
//! - Anything else, or anything asked for as a download, is served as
//!   `application/octet-stream` with `Content-Disposition: attachment` and the
//!   file's name.
//! - Every answer says `nosniff`.

use axum::http::header::{
    CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// What RFC 8187 encodes of a file name: all but the unreserved characters
/// `-`, `.`, `_`, `~` and `!`.
const FILE_NAME: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'!');

/// The headers that serve bytes as `media_type`, the type a request asks to
/// see them as: that type when the page shows it in place and no download
/// is asked for, a download named `file_name` otherwise.
pub(super) fn content_headers(
    media_type: Option<&str>,
    download: bool,
    file_name: Option<&str>,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    let in_place =
        media_type.filter(|media_type| !download && demi_shared_types::shows_in_place(media_type));
    match in_place {
        Some(media_type) => {
            if media_type.starts_with("image/") {
                headers.insert(
                    CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(demi_shared_types::IMAGE_CONTENT_POLICY),
                );
            }
            let value = HeaderValue::from_str(media_type)
                .expect("the file-type table's media types are header values");
            headers.insert(CONTENT_TYPE, value);
        }
        None => {
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            headers.insert(CONTENT_DISPOSITION, attachment(file_name));
        }
    }
    headers
}

/// `attachment` naming the file (RFC 6266): an ASCII fallback, in which
/// anything but printable ASCII and the quote and backslash become `_`, and
/// the exact name in RFC 8187 form.
fn attachment(file_name: Option<&str>) -> HeaderValue {
    let Some(name) = file_name.filter(|name| !name.is_empty()) else {
        return HeaderValue::from_static("attachment");
    };
    let fallback: String = name
        .chars()
        .map(|char| match char {
            '"' | '\\' => '_',
            ' '..='~' => char,
            _ => '_',
        })
        .collect();
    let encoded = utf8_percent_encode(name, FILE_NAME);
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .expect("printable ASCII is a header value")
}
