//! How served file bytes stay inert (`file-previews.md` § Keeping file
//! content inert): a media type the page shows in place is served as itself,
//! an image under a policy that keeps an SVG inert, and anything else, or
//! anything asked for as a download, as `application/octet-stream` with
//! `Content-Disposition: attachment` and the file's name. Every answer says
//! `nosniff`.

use axum::http::header::{CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderMap, HeaderValue};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// An image served in place runs no script and fetches nothing, in an opaque
/// origin.
const IMAGE_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/// What RFC 8187 encodes of a file name: all but the unreserved characters
/// `-`, `.`, `_`, `~` and `!`.
const FILE_NAME: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~').remove(b'!');

/// The headers that serve bytes as `media_type`, the type a request asks to
/// see them as: that type when the page shows it in place and no download
/// is asked for, a download named `file_name` otherwise.
pub(super) fn content_headers(media_type: Option<&str>, download: bool, file_name: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    let in_place = media_type.filter(|media_type| !download && demi_core::shows_in_place(media_type));
    match in_place {
        Some(media_type) => {
            if media_type.starts_with("image/") {
                headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(IMAGE_POLICY));
            }
            let value = HeaderValue::from_str(media_type).expect("the file-type table's media types are header values");
            headers.insert(CONTENT_TYPE, value);
        }
        None => {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
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
    HeaderValue::from_str(&format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"))
        .expect("printable ASCII is a header value")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(media_type: Option<&str>, download: bool, file_name: Option<&str>) -> Vec<(String, String)> {
        let mut headers: Vec<(String, String)> = content_headers(media_type, download, file_name)
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().unwrap().to_owned()))
            .collect();
        headers.sort();
        headers
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = expected
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        pairs.sort();
        pairs
    }

    #[test]
    fn media_shows_in_place_images_inert_and_the_rest_downloads_under_its_name() {
        assert_eq!(
            headers(Some("image/svg+xml"), false, Some("logo.svg")),
            pairs(&[
                ("content-type", "image/svg+xml"),
                ("content-security-policy", IMAGE_POLICY),
                ("x-content-type-options", "nosniff"),
            ])
        );
        // Chrome refuses audio opened directly under a policy; PDF viewers
        // refuse a sandbox.
        for media_type in ["audio/mpeg", "application/pdf"] {
            assert_eq!(
                headers(Some(media_type), false, None),
                pairs(&[("content-type", media_type), ("x-content-type-options", "nosniff")])
            );
        }
        let download = |disposition: &str| {
            pairs(&[
                ("content-type", "application/octet-stream"),
                ("content-disposition", disposition),
                ("x-content-type-options", "nosniff"),
            ])
        };
        assert_eq!(headers(Some("text/html"), false, None), download("attachment"));
        assert_eq!(headers(None, false, None), download("attachment"));
        assert_eq!(
            headers(Some("text/markdown"), false, Some("README.md")),
            download("attachment; filename=\"README.md\"; filename*=UTF-8''README.md")
        );
        assert_eq!(
            headers(Some("image/png"), true, Some("图 (1)'s.png")),
            download("attachment; filename=\"_ (1)'s.png\"; filename*=UTF-8''%E5%9B%BE%20%281%29%27s.png")
        );
    }
}
