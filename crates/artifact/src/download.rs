//! Bytes from an HTTPS URL or a local file, verified as they are written.

use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{Digest, Error, Verifier, digest::Measure};

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

/// [`client`], but plain HTTP too: for a caller that received the declared
/// size and SHA-256 over a connection it trusts, so the transport cannot
/// change what it keeps, such as a runner installing a command executable
/// (`native-runtime.md` § Install the selected executable). A download whose
/// digest comes from the same server keeps [`client`].
pub fn client_allowing_http() -> Result<reqwest::Client, Error> {
    builder()
        .https_only(false)
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
    let response = get(client, url, cancel).await?;
    if let Some(length) = response.content_length()
        && length != expected.size
    {
        return Err(Error::Size {
            declared: expected.size,
            actual: length,
        });
    }
    let mut verifier = Verifier::new(expected);
    body(response, output, cancel, |chunk| verifier.update(chunk)).await?;
    verifier.finish()
}

/// Streams `url` into `output` and returns the size and SHA-256 of what
/// arrived, for bytes nobody has declared a digest for yet, such as an
/// archive whose release record is being prepared. More than `limit` bytes
/// fail, and so does a body other than the length the server declared. The
/// caller owns `output` and discards it on any failure.
pub async fn download_measured(
    client: &reqwest::Client,
    url: &str,
    limit: u64,
    output: &mut (impl AsyncWrite + Unpin),
    cancel: &CancellationToken,
) -> Result<Digest, Error> {
    let response = get(client, url, cancel).await?;
    let declared = response.content_length();
    if declared.is_some_and(|length| length > limit) {
        return Err(Error::TooLarge { declared: limit });
    }
    // Bytes past a declared length fail as soon as they arrive.
    let mut measure = Measure::new(declared.unwrap_or(limit));
    body(response, output, cancel, |chunk| measure.update(chunk)).await?;
    let measured = measure.finish();
    if let Some(declared) = declared
        && declared != measured.size
    {
        return Err(Error::Size {
            declared,
            actual: measured.size,
        });
    }
    Ok(measured)
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

/// What a request failure means; the message leaves out the URL, which may
/// carry a signature.
fn failed(error: reqwest::Error) -> Error {
    Error::Download(error.without_url().to_string())
}

/// The answer to a GET of `url`, once it is a success.
async fn get(client: &reqwest::Client, url: &str, cancel: &CancellationToken) -> Result<reqwest::Response, Error> {
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
    Ok(response)
}

/// Writes the body of `response` into `output`, handing each chunk to
/// `check` before it is written.
async fn body(
    response: reqwest::Response,
    output: &mut (impl AsyncWrite + Unpin),
    cancel: &CancellationToken,
    mut check: impl FnMut(&[u8]) -> Result<(), Error>,
) -> Result<(), Error> {
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
        check(&chunk)?;
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    Ok(())
}
