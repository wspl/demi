//! Bytes from an HTTPS URL or a local file, verified as they are written.

use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{Digest, Error, Verifier};

pub(crate) fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
}

/// The client every artifact download uses: HTTPS only, no redirects, 15
/// seconds to connect and 60 seconds for any read.
pub fn client() -> Result<reqwest::Client, Error> {
    builder()
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// Streams `url` into `output`, enforcing the declared size and SHA-256. The
/// caller owns `output` and discards it on any failure.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    expected: &Digest,
    output: &mut (impl AsyncWrite + Unpin),
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let failed = |error: reqwest::Error| Error::Download(error.without_url().to_string());
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        response = client.get(url).send() => response.map_err(failed)?,
    };
    // With redirects off, a redirect is an answer that is not the artifact.
    if !response.status().is_success() {
        return Err(Error::Rejected {
            status: response.status().as_u16(),
        });
    }
    if let Some(length) = response.content_length()
        && length != expected.size
    {
        return Err(Error::Size {
            declared: expected.size,
            actual: length,
        });
    }
    let mut verifier = Verifier::new(expected);
    let mut body = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            chunk = body.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(failed)?;
        verifier.update(&chunk)?;
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    verifier.finish()
}

/// Copies `input`, such as a local file, into `output`, enforcing the
/// declared size and SHA-256.
pub async fn copy(
    input: &mut (impl AsyncRead + Unpin),
    expected: &Digest,
    output: &mut (impl AsyncWrite + Unpin),
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let mut verifier = Verifier::new(expected);
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            count = input.read(&mut buffer) => count?,
        };
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count])?;
        output.write_all(&buffer[..count]).await?;
    }
    output.flush().await?;
    verifier.finish()
}
