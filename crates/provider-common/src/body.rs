//! A request body, built and serialized off the user's shard: a request with images is megabytes of
//! base64, which would hold up every conversation of the user.

use serde::Serialize;

use crate::ProviderFailure;

/// Runs `encode`, which builds a request body and serializes it, such as
/// with [`json_body`], on the blocking pool. Building a body cannot fail: a
/// request carries its media's bytes, never a reference. A body whose work
/// panicked fails the run without a code, as a request Demi could not
/// build. Dropping the future while it runs leaves the work to finish
/// there; it has no effect beyond its result, which is then discarded.
pub async fn encode_body<T, F>(label: &str, encode: F) -> Result<T, ProviderFailure>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(encode)
        .await
        .map_err(|error| ProviderFailure {
            message: format!("{label} API request body was not built: {error}"),
            code: None,
            diagnostics: None,
            retry_after: None,
        })
}

/// A request body as JSON bytes.
pub fn json_body(body: &impl Serialize) -> Vec<u8> {
    // A body is strings, numbers and JSON values, which always serialize.
    serde_json::to_vec(body).expect("a request body serializes")
}

/// A `multipart/form-data` body built whole (RFC 7578), which a request to
/// a device's exposed port needs: reqwest's own form is a stream, which
/// the device route cannot carry, and it does not give its bytes.
pub struct FormBody {
    boundary: String,
    bytes: Vec<u8>,
}

impl FormBody {
    pub fn new() -> Self {
        Self {
            boundary: format!("demi-{}", uuid::Uuid::new_v4().simple()),
            bytes: Vec::new(),
        }
    }

    /// Adds the text field `name`.
    pub fn text(mut self, name: &str, value: &str) -> Self {
        self.head(name, None);
        self.bytes.extend_from_slice(value.as_bytes());
        self.bytes.extend_from_slice(b"\r\n");
        self
    }

    /// Adds the file field `name`, its `file_name` and `media_type` given.
    pub fn file(mut self, name: &str, file_name: &str, media_type: &str, bytes: &[u8]) -> Self {
        self.head(name, Some((file_name, media_type)));
        self.bytes.extend_from_slice(bytes);
        self.bytes.extend_from_slice(b"\r\n");
        self
    }

    fn head(&mut self, name: &str, file: Option<(&str, &str)>) {
        let mut head = format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"", self.boundary, quoted(name));
        if let Some((file_name, media_type)) = file {
            head.push_str(&format!("; filename=\"{}\"\r\nContent-Type: {media_type}", quoted(file_name)));
        }
        head.push_str("\r\n\r\n");
        self.bytes.extend_from_slice(head.as_bytes());
    }

    /// The body's `Content-Type`, with its boundary, and its bytes.
    pub fn finish(mut self) -> (String, bytes::Bytes) {
        self.bytes.extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (format!("multipart/form-data; boundary={}", self.boundary), self.bytes.into())
    }
}

impl Default for FormBody {
    fn default() -> Self {
        Self::new()
    }
}

/// A field or file name inside quotes, its quote and line breaks escaped
/// as RFC 7578 § 2 and the HTML form encoding do.
fn quoted(name: &str) -> String {
    name.replace('"', "%22").replace('\r', "%0D").replace('\n', "%0A")
}
