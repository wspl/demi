//! Demi's one download of a file over HTTP: resumed into a staging file the
//! caller names after a break or a stall, without an overall deadline, its
//! progress told to the caller, its redirects followed here so the caller
//! decides each hop's headers, and its bytes decoded and checked whole once
//! complete. Copies of local files are checked the same way.
//!
//! Every file Demi fetches over HTTP goes through it (a command executable, a
//! runner update, a Cloud image's software, a file `demi web fetch` saves, a
//! vendor's video, an attachment `demi attachment download` fetches), so a
//! fix for a slow or broken link helps them all. A stream, such as a model's
//! answer, a command's pipe or a live view, is not a download. The downloader
//! only reports the bytes received and the size when known; the caller
//! decides what to show.

use std::{path::Path, time::Duration};

use futures_util::{StreamExt, future::BoxFuture};
use reqwest::{Url, header::HeaderMap};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{CONTENT_CODING, Digest, Error, Verifier};

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

/// A client for a request that is not a file's download, such as an API's
/// answer: HTTPS only, no redirects, zstd decoded, 15 seconds to connect
/// and 60 seconds for any read.
pub fn client() -> Result<reqwest::Client, Error> {
    builder()
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// The client [`download`] takes: HTTPS only, at every hop of a redirect,
/// no redirects of its own, since the download follows them itself, and
/// nothing decoded, since the bytes are kept as served.
pub fn download_client() -> Result<reqwest::Client, Error> {
    built(builder().no_zstd())
}

/// [`download_client`], but plain HTTP too: for a caller that received the
/// size and SHA-256 over a connection it trusts, so the transport cannot
/// change what it keeps, such as a runner installing a command executable.
pub fn download_client_allowing_http() -> Result<reqwest::Client, Error> {
    download_client_through(|builder| builder)
}

/// [`download_client_allowing_http`] through the connection `connect` sets
/// up, such as a Cloud runner's backend socket
/// (`crates/backend-http/src/lib.rs`).
pub fn download_client_through(
    connect: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
) -> Result<reqwest::Client, Error> {
    built(connect(builder().https_only(false).no_zstd()))
}

fn built(builder: reqwest::ClientBuilder) -> Result<reqwest::Client, Error> {
    builder
        .build()
        .map_err(|error| Error::Download(error.without_url().to_string()))
}

/// What sends a download's requests: a `GET` of an address with headers,
/// following no redirect, which the download follows itself. A
/// [`download_client`] is one; a caller with its own way to the network,
/// such as a provider's to its owner's devices, is another.
pub trait Fetch: Send + Sync {
    fn fetch(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> BoxFuture<'_, Result<reqwest::Response, Unsent>>;
}

/// Why a request got no answer.
#[derive(Debug)]
pub enum Unsent {
    /// It failed on the way, as a connection does; a new one may succeed.
    Connection(Error),
    /// It cannot be sent, and would fail the same way again, such as one
    /// to plain HTTP over an HTTPS-only client.
    Refused(Error),
}

impl Unsent {
    /// What `error`, reqwest's, means for the download.
    pub fn of(error: reqwest::Error) -> Self {
        if error.is_builder() {
            Self::Refused(failed(error))
        } else {
            Self::Connection(failed(error))
        }
    }
}

impl Fetch for reqwest::Client {
    fn fetch(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> BoxFuture<'_, Result<reqwest::Response, Unsent>> {
        Box::pin(async move { self.get(url).headers(headers).send().await.map_err(Unsent::of) })
    }
}

/// How long a download waits for bytes: a connection that delivered some
/// and then nothing for `stall` is given up for a new one, and connections
/// that deliver nothing for `silence` fail the download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Patience {
    pub stall: Duration,
    pub silence: Duration,
}

impl Patience {
    /// 30 seconds before a stalled connection is replaced, 60 of nothing
    /// before the download fails.
    pub const DEFAULT: Self = Self {
        stall: Duration::from_secs(30),
        silence: Duration::from_secs(60),
    };
}

/// The headers a request to an address carries, chosen for each hop of its
/// redirects, so a credential never reaches another origin.
pub type Headers<'a> = &'a (dyn Fn(&Url) -> HeaderMap + Send + Sync);

/// No headers at any hop.
pub const NO_HEADERS: Headers<'static> = &|_| HeaderMap::new();

/// One file to download.
pub struct Download<'a> {
    pub url: &'a str,
    /// The most bytes the file may have; a larger one fails.
    pub limit: u64,
    pub patience: Patience,
    /// How many redirects a request follows.
    pub redirects: usize,
    pub headers: Headers<'a>,
    /// The answer to the first request, when the caller sent it to see
    /// the answer first, as `demi web fetch` does: a success, whose bytes
    /// the download writes from the start of the staging file.
    pub first: Option<reqwest::Response>,
    /// Whether the caller renews the address when the server refuses it,
    /// as a signed URL that expired during a long download: the refusal
    /// then keeps the staging file, for the download from the renewed
    /// address to continue.
    pub renewable: bool,
    /// Whether the client may have decoded `first`'s bytes on the way, so
    /// the staging file's length counts no byte the server sent: a break
    /// then asks again from the start, for the bytes uncoded.
    pub first_decoded: bool,
}

impl<'a> Download<'a> {
    /// `url`: any size, no redirects, no headers and the default patience.
    pub fn new(url: &'a str) -> Self {
        Self {
            url,
            limit: u64::MAX,
            patience: Patience::DEFAULT,
            redirects: 0,
            headers: NO_HEADERS,
            first: None,
            renewable: false,
            first_decoded: false,
        }
    }
}

/// What a download holds once complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// The content coding the server declared, if any.
    pub coding: Option<String>,
    pub size: u64,
}

impl Received {
    /// Writes the staged bytes at `staging`, decoded from their content
    /// coding, into `output`, checked against `expected`; the staging file
    /// is removed once checked, and when it fails the check, but kept when
    /// cancelled, for a later download to continue.
    pub async fn verify(
        &self,
        staging: &Path,
        expected: &Digest,
        output: &mut (impl AsyncWrite + Unpin),
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let checked = match self.coding.as_deref() {
            None => {
                let mut input = tokio::fs::File::open(staging).await?;
                copy(&mut input, expected, output, cancel).await
            }
            Some(CONTENT_CODING) => crate::decode_file(staging, expected, output, cancel).await,
            Some(coding) => Err(Error::Coding(coding.to_owned())),
        };
        if matches!(checked, Err(Error::Cancelled)) {
            return checked;
        }
        // Checked bytes are installed and failed ones no start for the next
        // download: either way the staging file has served. One that cannot
        // be removed is checked again by the next download that finds it
        // complete, and replaced by one that finds it not.
        discard(staging).await;
        checked
    }

    /// Fails when the bytes arrived in a content coding, for a caller that
    /// takes them as they are.
    pub fn plain(&self) -> Result<(), Error> {
        match &self.coding {
            None => Ok(()),
            Some(coding) => Err(Error::Coding(coding.clone())),
        }
    }
}

/// How long a silent run of connections waits before it opens the next,
/// so a connection refused at once is not asked again in a tight loop.
const SILENT_PAUSE: Duration = Duration::from_secs(1);

/// How many times in a row a server may answer a resumed request with the
/// whole file and break before its end, before the download gives up on it.
const WHOLE_BREAKS: u32 = 3;

/// Downloads `download` into the file at `staging`, which keeps the bytes
/// as served, continuing from the bytes
/// the file holds with a Range request, so a download that broke off, in
/// this call or an earlier one, does not start again. The content coding
/// the bytes were sent in is recorded beside the file as the download
/// starts, and is what [`Received`] names, whatever a later answer
/// without a body says. A server that answers the whole file, or a range
/// that does not start where the file ends, starts the file again; one
/// that answers the whole file and breaks before its end three times in a
/// row fails the download. A connection that breaks off after bytes
/// arrived, or that delivers none for `patience.stall` after some, is
/// given up and a new one resumes at once, since a new connection often
/// takes a better path than one that stalled. New connections that deliver
/// nothing are opened again, a second apart, until `patience.silence` has
/// passed without a byte; then the download fails with the last cause. A
/// download stopped so, or cancelled, or refused at an address its caller
/// renews, keeps its staging file for a later one; any other failure, such as a refusal or more bytes than the limit,
/// removes it. `progress` hears how many bytes the file holds after each
/// piece, and of how many when the server says.
pub async fn download(
    client: &(impl Fetch + ?Sized),
    download: Download<'_>,
    staging: &Path,
    cancel: &CancellationToken,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<Received, Error> {
    match fetched(client, download, staging, cancel, progress).await {
        Ok(received) => Ok(received),
        Err(Stop::Keep(error)) => Err(error),
        Err(Stop::Remove(error)) => {
            discard(staging).await;
            Err(error)
        }
    }
}

/// How a download stopped short: with its staging file kept, as by the
/// network or its caller, or removed.
enum Stop {
    Keep(Error),
    Remove(Error),
}

impl From<Error> for Stop {
    fn from(error: Error) -> Self {
        Self::Remove(error)
    }
}

impl From<std::io::Error> for Stop {
    fn from(error: std::io::Error) -> Self {
        Self::Remove(error.into())
    }
}

/// The file beside `staging` that records the content coding of its bytes,
/// empty for none.
fn coding_record(staging: &Path) -> std::path::PathBuf {
    let mut name = staging.file_name().unwrap_or_default().to_owned();
    name.push(".coding");
    staging.with_file_name(name)
}

/// Removes the staging file `staging` of a download and its coding record,
/// as a cache's pruning does for an artifact no longer named. One that
/// cannot be removed is left for the next download, which starts it again
/// or checks it whole.
pub async fn discard(staging: &Path) {
    let _removed = tokio::fs::remove_file(staging).await;
    let _removed = tokio::fs::remove_file(coding_record(staging)).await;
}

/// The content coding recorded for `staging`, or none when nothing records
/// one, in which case the file is no start for a resume.
async fn recorded_coding(staging: &Path) -> std::io::Result<Option<Option<String>>> {
    match tokio::fs::read_to_string(coding_record(staging)).await {
        Ok(coding) => Ok(Some((!coding.is_empty()).then_some(coding))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

async fn fetched(
    client: &(impl Fetch + ?Sized),
    mut download: Download<'_>,
    staging: &Path,
    cancel: &CancellationToken,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Received, Stop> {
    let mut first = download.first.take();
    // Whether the bytes are asked for uncoded from the start, after an
    // answer decoded on the way broke off.
    let mut uncoded = false;
    // When the connections that delivered nothing began, since the last byte.
    let mut silent_since = None;
    // Whole answers to a resumed request that broke before their end, in a row.
    let mut whole_breaks = 0;
    loop {
        let since = *silent_since.get_or_insert_with(tokio::time::Instant::now);
        let silence = since + download.patience.silence;
        // Whether this answer is the caller's, its bytes perhaps decoded.
        let decoded = first.is_some() && download.first_decoded;
        let (held, response) = match first.take() {
            // The caller's answer starts the file.
            Some(response) => (0, Ok(response)),
            None => {
                let held = match tokio::fs::metadata(staging).await {
                    Ok(metadata) => metadata.len(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(error) => return Err(error.into()),
                };
                // Bytes whose coding nothing records are no start.
                let held = if held > 0 && recorded_coding(staging).await?.is_none() {
                    discard(staging).await;
                    0
                } else {
                    held
                };
                let response = tokio::select! {
                    _ = cancel.cancelled() => return Err(Stop::Keep(Error::Cancelled)),
                    _ = tokio::time::sleep_until(silence) => return Err(Stop::Keep(silent(download.patience))),
                    response = send(client, &download, held, uncoded) => response,
                };
                (held, response)
            }
        };
        let response = match response {
            Ok(response) => response,
            Err(Unsent::Refused(error)) => return Err(error.into()),
            Err(Unsent::Connection(error)) => {
                pause(silence, cancel, || error).await.map_err(Stop::Keep)?;
                continue;
            }
        };
        let range = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .map(ContentRange::parse);
        let status = response.status();
        let (mut file, total) = match status {
            reqwest::StatusCode::PARTIAL_CONTENT if held > 0 => {
                let Some(Some(ContentRange { start: Some(start), total })) = range else {
                    return Err(Error::Download("a partial answer names no range".into()).into());
                };
                if start != held {
                    // Not the rest of the file: it starts again.
                    discard(staging).await;
                    continue;
                }
                whole_breaks = 0;
                let file = tokio::fs::OpenOptions::new().append(true).open(staging).await?;
                (file, total)
            }
            status if status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT => {
                let coding = response
                    .headers()
                    .get(reqwest::header::CONTENT_ENCODING)
                    .map(|coding| String::from_utf8_lossy(coding.as_bytes()).into_owned());
                // The coding is on disk before the first byte, so a staging
                // file never holds bytes of an unknown coding.
                tokio::fs::write(coding_record(staging), coding.unwrap_or_default()).await?;
                let file = tokio::fs::File::create(staging).await?;
                (file, response.content_length())
            }
            // The file holds every byte already, as when a download ended
            // before its bytes were checked.
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE
                if matches!(range, Some(Some(ContentRange { total: Some(total), .. })) if total == held) =>
            {
                let coding = recorded_coding(staging).await?.flatten();
                return Ok(Received { coding, size: held });
            }
            status => {
                let refused = Error::Rejected { status: status.as_u16() };
                return Err(if download.renewable {
                    Stop::Keep(refused)
                } else {
                    refused.into()
                });
            }
        };
        if let Some(total) = total
            && total > download.limit
        {
            return Err(Error::TooLarge {
                declared: download.limit,
            }
            .into());
        }
        let mut written = file.metadata().await?.len();
        progress(written, total);
        let mut body = response.bytes_stream();
        let mut received = 0;
        let ended = loop {
            // The first byte may take what is left of the silence; a
            // connection that delivered stalls after `stall`.
            let waits = if received == 0 {
                silence
            } else {
                tokio::time::Instant::now() + download.patience.stall
            };
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(Stop::Keep(Error::Cancelled)),
                _ = tokio::time::sleep_until(waits) => break Some(Ended::Stalled),
                chunk = body.next() => chunk,
            };
            match chunk {
                None => break None,
                Some(Ok(chunk)) => {
                    if written + chunk.len() as u64 > download.limit {
                        return Err(Error::TooLarge {
                            declared: download.limit,
                        }
                        .into());
                    }
                    file.write_all(&chunk).await?;
                    received += chunk.len() as u64;
                    written += chunk.len() as u64;
                    progress(written, total);
                }
                Some(Err(error)) => break Some(Ended::Broke(error)),
            }
        };
        file.flush().await?;
        drop(file);
        if ended.is_none() && total.is_none_or(|total| total == written) {
            let coding = recorded_coding(staging).await?.flatten();
            return Ok(Received {
                coding,
                size: written,
            });
        }
        if received > 0 {
            silent_since = None;
        }
        if decoded {
            // Decoded bytes count no byte the server sent: the file is asked
            // for again from its start, uncoded.
            discard(staging).await;
            uncoded = true;
        } else if held > 0 && status == reqwest::StatusCode::OK {
            whole_breaks += 1;
            if whole_breaks == WHOLE_BREAKS {
                return Err(Error::Download(format!(
                    "the server answers a resumed download with the whole file and broke off before its end {WHOLE_BREAKS} times in a row"
                ))
                .into());
            }
        }
        match ended {
            // A connection that delivered bytes and then broke off or
            // stalled: a new one resumes at once.
            _ if received > 0 => {}
            None => pause(silence, cancel, || silent(download.patience))
                .await
                .map_err(Stop::Keep)?,
            Some(Ended::Stalled) => return Err(Stop::Keep(silent(download.patience))),
            Some(Ended::Broke(error)) => pause(silence, cancel, || failed(error))
                .await
                .map_err(Stop::Keep)?,
        }
    }
}

/// [`download`] of a file its caller holds in memory, such as a vendor's
/// generated video: staged in a temporary file, which goes once read. Its
/// bytes are as served; the caller checks their coding.
pub async fn download_bytes(
    client: &(impl Fetch + ?Sized),
    download: Download<'_>,
    cancel: &CancellationToken,
) -> Result<(Received, Vec<u8>), Error> {
    let staging = tempfile::Builder::new().prefix("demi-download-").tempdir()?;
    let path = staging.path().join("download");
    let received = self::download(client, download, &path, cancel, |_, _| {}).await?;
    let bytes = tokio::fs::read(&path).await?;
    Ok((received, bytes))
}

/// Sends `download`'s request from byte `held` on, following its redirects
/// with the headers each hop's address is given. A resumed request, or one
/// `uncoded`, asks for the bytes as they are, unless the caller names a
/// coding, so a range counts the bytes the file holds.
async fn send(
    client: &(impl Fetch + ?Sized),
    download: &Download<'_>,
    held: u64,
    uncoded: bool,
) -> Result<reqwest::Response, Unsent> {
    let mut url = Url::parse(download.url)
        .map_err(|error| Unsent::Refused(Error::Download(format!("not a URL: {error}"))))?;
    for _ in 0..=download.redirects {
        let mut headers = (download.headers)(&url);
        if held > 0 {
            headers.insert(
                reqwest::header::RANGE,
                reqwest::header::HeaderValue::from_str(&format!("bytes={held}-"))
                    .expect("a range is a header value"),
            );
        }
        if held > 0 || uncoded {
            headers
                .entry(reqwest::header::ACCEPT_ENCODING)
                .or_insert(reqwest::header::HeaderValue::from_static("identity"));
        }
        let response = client.fetch(url.clone(), headers).await?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let next = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|location| location.to_str().ok())
            .and_then(|location| url.join(location).ok());
        let Some(next) = next else {
            return Ok(response);
        };
        url = next;
    }
    Err(Unsent::Refused(Error::Download(format!(
        "the address redirected more than {} times",
        download.redirects
    ))))
}

/// How a connection's body ended before its last byte.
enum Ended {
    Stalled,
    Broke(reqwest::Error),
}

/// Waits before the next of a silent run of connections; fails with
/// `failure` once the run's `silence` would pass meanwhile.
async fn pause(
    silence: tokio::time::Instant,
    cancel: &CancellationToken,
    failure: impl FnOnce() -> Error,
) -> Result<(), Error> {
    let next = tokio::time::Instant::now() + SILENT_PAUSE;
    if next >= silence {
        return Err(failure());
    }
    tokio::select! {
        _ = cancel.cancelled() => Err(Error::Cancelled),
        _ = tokio::time::sleep_until(next) => Ok(()),
    }
}

/// The failure of a download whose connections delivered nothing for the
/// whole of its patience.
fn silent(patience: Patience) -> Error {
    Error::Download(format!(
        "timed out: nothing arrived for {} seconds",
        patience.silence.as_secs()
    ))
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

/// What a request failure means, with each cause beneath it, such as
/// *connection reset*, since reqwest names only the layer that saw it, such
/// as *error decoding response body*; the message leaves out the URL, which
/// may carry a signature.
fn failed(error: reqwest::Error) -> Error {
    let error = error.without_url();
    let mut message = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(error) = cause {
        let said = error.to_string();
        if !message.ends_with(&said) {
            message.push_str(": ");
            message.push_str(&said);
        }
        cause = error.source();
    }
    Error::Download(message)
}

