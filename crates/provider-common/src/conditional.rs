//! The conditional download of a document a backend keeps a copy of
//! (`models.md` § The models.dev document, `media-models.md` § Versions):
//! a `GET` that sends the copy's `ETag` and `Last-Modified`, so an
//! unchanged document answers 304 without its body. The models.dev client
//! and the media catalog both download this way; what each keeps and when
//! it asks is its own.

use bytes::Bytes;
use http::header::{ACCEPT, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use http::{HeaderValue, StatusCode};
use reqwest::Url;

/// What revalidates a downloaded copy.
#[derive(Debug, Clone, Default)]
pub(crate) struct Validators {
    etag: Option<HeaderValue>,
    last_modified: Option<HeaderValue>,
}

/// A download's answer.
pub(crate) enum Answer {
    /// The copy the request was sent with is still the document.
    NotModified,
    /// The document, with what revalidates it next time.
    Fetched { body: Bytes, validators: Validators },
}

/// Why a download failed.
pub(crate) enum Failure {
    /// The request was not answered, or its body not read.
    Transport(reqwest::Error),
    /// The answer's status was not a success, nor a 304 to a conditional
    /// request.
    Status(StatusCode),
}

/// The JSON document at `url`, asked for conditionally when `copy` gives
/// what revalidates the copy kept.
pub(crate) async fn get(http: &reqwest::Client, url: &Url, copy: Option<&Validators>) -> Result<Answer, Failure> {
    let mut request = http
        .get(url.clone())
        .header(ACCEPT, HeaderValue::from_static("application/json"));
    if let Some(copy) = copy {
        if let Some(etag) = &copy.etag {
            request = request.header(IF_NONE_MATCH, etag.clone());
        }
        if let Some(modified) = &copy.last_modified {
            request = request.header(IF_MODIFIED_SINCE, modified.clone());
        }
    }
    let response = request.send().await.map_err(Failure::Transport)?;
    let status = response.status();
    if status == StatusCode::NOT_MODIFIED && copy.is_some() {
        return Ok(Answer::NotModified);
    }
    if !status.is_success() {
        return Err(Failure::Status(status));
    }
    let validators = Validators {
        etag: response.headers().get(ETAG).cloned(),
        last_modified: response.headers().get(LAST_MODIFIED).cloned(),
    };
    let body = response.bytes().await.map_err(Failure::Transport)?;
    Ok(Answer::Fetched { body, validators })
}
