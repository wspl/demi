//! The Files API of a Gemini API key, by which a request names a file it
//! sends rather than holding it as base64
//! ([Files API](https://ai.google.dev/gemini-api/docs/files)).
//!
//! 1. `POST {origin}/upload/{version}/files` starts a resumable upload and
//!    answers its address in `x-goog-upload-url`.
//! 2. One `POST` of the bytes to that address uploads and finalizes it, and
//!    answers the file.
//! 3. `GET …/files/<id>` is read until the file is `ACTIVE`: Google
//!    processes a video before a request may name it.
//!
//! Google deletes a file 48 hours after its upload.

use std::time::Duration;

use bytes::Bytes;
use demi_provider_common::{
    json_body,
    media::{Ask, Holding, MediaFailure, unreadable},
};
use demi_shared_types::VendorTrace;
use reqwest::Url;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;

use crate::images::GeminiKey;

/// How often a processing file is read again.
const PROCESSING_READ_EVERY: Duration = Duration::from_secs(2);

/// How many reads a file's processing may take, two minutes, as long as
/// one request may take: Google processes a video of ten seconds in a few.
const PROCESSING_READS: u32 = 60;

#[derive(Deserialize)]
struct Uploaded {
    file: File,
}

#[derive(Deserialize)]
struct File {
    name: String,
    uri: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<FileError>,
}

#[derive(Deserialize)]
struct FileError {
    #[serde(default)]
    message: Option<String>,
}

impl GeminiKey {
    /// Uploads `bytes` of `media_type` and answers the file's address once
    /// Google has processed it.
    pub(crate) async fn upload(
        &self,
        http: &demi_provider_common::ProviderHttp,
        bytes: Bytes,
        media_type: &'static str,
    ) -> Result<String, MediaFailure> {
        let start = self.upload_url();
        let mut headers = self.headers();
        let set = |headers: &mut HeaderMap, name: &'static str, value: &str| {
            let value = HeaderValue::from_str(value).expect("an upload header's value is ASCII");
            headers.insert(HeaderName::from_static(name), value);
        };
        set(&mut headers, "x-goog-upload-protocol", "resumable");
        set(&mut headers, "x-goog-upload-command", "start");
        set(&mut headers, "x-goog-upload-header-content-length", &bytes.len().to_string());
        set(&mut headers, "x-goog-upload-header-content-type", media_type);
        let body = Bytes::from(json_body(&serde_json::json!({ "file": { "display_name": "demi" } })));
        let started = self
            .call()
            .send(http, Ask::post_json(&start, body), Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
            .result?;
        let at = started
            .headers
            .get("x-goog-upload-url")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Url::parse(value).ok())
            .ok_or_else(|| unreadable("its upload named no address to continue at".to_owned()))?;
        let mut headers = self.headers();
        set(&mut headers, "x-goog-upload-offset", "0");
        set(&mut headers, "x-goog-upload-command", "upload, finalize");
        let answered = self
            .call()
            .send(http, Ask::post(&at, media_type, bytes), Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
            .result?;
        let uploaded: Uploaded = serde_json::from_slice(&answered.body)
            .map_err(|error| unreadable(format!("its uploaded file could not be read: {error}")))?;
        let mut file = uploaded.file;
        for _ in 0..PROCESSING_READS {
            match file.state.as_deref() {
                Some("PROCESSING") => {}
                Some("FAILED") => {
                    let trace = VendorTrace::default().with_words(file.error.and_then(|error| error.message));
                    let reason = trace.words.is_none().then(|| "it could not process the file".to_owned());
                    return Err(MediaFailure::Refused { reason, code: None, trace });
                }
                _ => return Ok(file.uri),
            }
            tokio::time::sleep(PROCESSING_READ_EVERY).await;
            let url = demi_provider_common::endpoint_url(self.base(), &format!("/{}", file.name));
            let body = self.call().get(http, &url, self.headers()).await?;
            file = serde_json::from_slice(&body)
                .map_err(|error| unreadable(format!("its file could not be read: {error}")))?;
        }
        Err(unreadable("its file was still processing after two minutes".to_owned()))
    }

    /// `{origin}/upload/{the base's path}/files`, such as
    /// `https://generativelanguage.googleapis.com/upload/v1beta/files`.
    fn upload_url(&self) -> Url {
        let mut url = self.base().clone();
        let path = format!("/upload{}/files", self.base().path().trim_end_matches('/'));
        url.set_path(&path);
        url
    }
}
