//! A request body, built and serialized off the user's shard
//! (`concurrency.md` § Blocking work): a request with images is megabytes of
//! base64, which would hold up every conversation of the user.

use serde::Serialize;

use crate::ProviderFailure;

/// A request that names media the session did not load back before the
/// request; it names the blob.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("media {0} was not loaded")]
pub struct UnloadedMedia(pub String);

/// Runs `encode`, which builds a request body and serializes it, such as
/// with [`json_body`], on the blocking pool. A body that names unloaded media
/// fails the run without a code, as a request Demi could not build. Dropping
/// the future while it runs leaves the work to finish there; it has no
/// effect beyond its result, which is then discarded.
pub async fn encode_body<T, F>(label: &str, encode: F) -> Result<T, ProviderFailure>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, UnloadedMedia> + Send + 'static,
{
    let encoded = tokio::task::spawn_blocking(encode).await;
    let unbuilt = |message: String| ProviderFailure {
        message,
        code: None,
        diagnostics: None,
        retry_after: None,
    };
    match encoded {
        Ok(Ok(body)) => Ok(body),
        Ok(Err(UnloadedMedia(blob))) => Err(unbuilt(format!(
            "{label} API request names media {blob} that was not loaded"
        ))),
        Err(error) => Err(unbuilt(format!("{label} API request body was not built: {error}"))),
    }
}

/// A request body as JSON bytes.
pub fn json_body(body: &impl Serialize) -> Vec<u8> {
    // A body is strings, numbers and JSON values, which always serialize.
    serde_json::to_vec(body).expect("a request body serializes")
}
