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
        // zstd is the one coding an artifact arrives in; the workspace's one
        // reqwest has the other decoders on for `demi-fetch`, and a coding
        // the download does not name must fail rather than be decoded.
        .no_gzip()
        .no_brotli()
        .no_deflate()
}

/// The client every artifact download uses: HTTPS only, no redirects, 15
/// seconds to connect and 60 seconds for any read.
pub fn client() -> Result<reqwest::Client, Error> {
    builder()
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// [`client`], but plain HTTP too, and following redirects: for a caller
/// that received the declared size and SHA-256 over a connection it trusts,
/// so neither the transport nor where the bytes come from can change what
/// it keeps, such as a runner installing a command executable or the Chrome
/// runtime, which GitHub serves through a redirect to short-lived storage
/// (`native-runtime.md` § Install artifacts). A download whose digest
/// comes from the same server keeps [`client`].
pub fn client_allowing_http() -> Result<reqwest::Client, Error> {
    builder()
        .https_only(false)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// [`client_allowing_http`] for a backend's own artifacts, through the
/// connection `connect` sets up, such as a Cloud runner's backend socket
/// (`managed-hosts.md` § Backend socket). It follows no redirect, which would
/// leave through that same connection.
pub fn client_through(
    connect: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
) -> Result<reqwest::Client, Error> {
    connect(builder().https_only(false))
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// [`client_allowing_http`] for a download that [`download_resumed`]
/// keeps as it is served, in its content coding, such as a runner's update
/// from its backend: it follows no redirect, decodes nothing, and gives a
/// connection up once `stall` passes without a byte arriving.
pub fn client_resuming(stall: Duration) -> Result<reqwest::Client, Error> {
    builder()
        .https_only(false)
        .no_zstd()
        .read_timeout(stall)
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// [`client`], but following redirects: for a server release's files, which
/// GitHub serves through a redirect to short-lived storage. The backend
/// checks every file against the size and SHA-256 its own release records
/// give, so where the bytes come from cannot change what it keeps
/// (`native-runtime.md` § Publish packages, then source artifacts on demand).
pub fn client_following_redirects() -> Result<reqwest::Client, Error> {
    builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// Streams `url` into `output`, decoding its content coding, and enforces the
/// declared size and SHA-256 on the decoded bytes. The caller owns `output`
/// and discards it on any failure.
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

/// Downloads `url` with `headers` into the file at `path` as it is served,
/// in its content coding, continuing from the bytes the file holds with a
/// Range request, so a download that broke off, in this call or an
/// earlier one, does not start again. A connection that breaks off after
/// bytes arrived, or that the client's read timeout ends
/// ([`client_resuming`]), is given up and a new one resumes at once, since
/// a new connection often takes a better path than one that stalled. A
/// request that fails before any byte arrives fails the download and leaves
/// the file for the next to resume. The file is complete once it holds the
/// length the server declares; the caller checks what it holds.
pub async fn download_resumed(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
    path: &std::path::Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    loop {
        let held = match tokio::fs::metadata(path).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        let mut request = client.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if held > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={held}-"));
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            response = request.send() => response.map_err(failed)?,
        };
        let range = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .map(ContentRange::parse);
        let (mut file, total) = match response.status() {
            reqwest::StatusCode::PARTIAL_CONTENT => {
                let Some(Some(ContentRange { start: Some(start), total })) = range else {
                    return Err(Error::Download("a partial answer names no range".into()));
                };
                if start != held {
                    return Err(Error::Download(format!(
                        "the server answered bytes from {start}, not from {held}"
                    )));
                }
                let file = tokio::fs::OpenOptions::new().append(true).open(path).await?;
                (file, total)
            }
            reqwest::StatusCode::OK => {
                let file = tokio::fs::File::create(path).await?;
                (file, response.content_length())
            }
            // The file holds every byte already, as when a download ended
            // before its bytes were checked.
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE
                if matches!(range, Some(Some(ContentRange { total: Some(total), .. })) if total == held) =>
            {
                return Ok(());
            }
            status => return Err(Error::Rejected { status: status.as_u16() }),
        };
        let mut body = response.bytes_stream();
        let mut received = 0;
        let ended = loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                chunk = body.next() => chunk,
            };
            match chunk {
                None => break None,
                Some(Ok(chunk)) => {
                    file.write_all(&chunk).await?;
                    received += chunk.len() as u64;
                }
                Some(Err(error)) => break Some(error),
            }
        };
        file.flush().await?;
        let held = file.metadata().await?.len();
        match ended {
            None if total.is_none_or(|total| total == held) => return Ok(()),
            None => {}
            // A stalled connection, or one that broke off after bytes
            // arrived: a new one resumes.
            Some(error) if received > 0 || error.is_timeout() => {}
            Some(error) => return Err(failed(error)),
        }
    }
}

/// A `Content-Range` answer's range: where it starts, none for the
/// `bytes */<total>` of an unsatisfiable range, and the whole's length when
/// the server knows it.
struct ContentRange {
    start: Option<u64>,
    total: Option<u64>,
}

impl ContentRange {
    /// `bytes <start>-<end>/<total>`, either of `<start>-<end>` and
    /// `<total>` possibly `*`; none for anything else.
    fn parse(value: &str) -> Option<Self> {
        let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
        let total = match total {
            "*" => None,
            total => Some(total.parse().ok()?),
        };
        let start = match range {
            "*" => None,
            range => Some(range.split_once('-')?.0.parse().ok()?),
        };
        Some(Self { start, total })
    }
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
async fn get(
    client: &reqwest::Client,
    url: &str,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, Error> {
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
    // The client decodes zstd and then drops the header, and with it the
    // encoded length: a coding still named is one it does not decode.
    if let Some(coding) = response.headers().get(reqwest::header::CONTENT_ENCODING) {
        return Err(Error::Coding(
            String::from_utf8_lossy(coding.as_bytes()).into_owned(),
        ));
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
