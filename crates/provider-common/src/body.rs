//! A request body, built and serialized off the user's shard
//! (`concurrency.md` § Blocking work): a request with images is megabytes of
//! base64, which would hold up every conversation of the user.

use serde::Serialize;

use crate::ProviderFailure;

/// Runs `encode`, which builds a request body and serializes it, such as
/// with [`json_body`], on the blocking pool. Building a body cannot fail: a
/// request carries its media's bytes, never a reference (`runtime.md`
/// § Media). A body whose work panicked fails the run without a code, as a
/// request Demi could not build. Dropping the future while it runs leaves
/// the work to finish there; it has no effect beyond its result, which is
/// then discarded.
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
