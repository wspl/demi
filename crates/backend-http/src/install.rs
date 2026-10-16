//! The public installation routes (`web-api.md` § Resource index): the
//! runner installers for the backend's current runner release, the runner
//! executables of each release its server release's `runners/` names,
//! sourced into the object store on their first download
//! (`native-runtime.md` § Runner releases), and a local store's command
//! executables (§ Backend deployment configuration). None holds a
//! credential; pairing grants device access. Without runner releases, the
//! installers answer 503 and the runner executables 404.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, HOST,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::ops::Bound;

use axum_extra::TypedHeader;
use axum_extra::headers::Range;
use bytes::Bytes;
use demi_backend_database::devices::DeviceRecord;
use demi_backend_runners::devices::Download;
use demi_backend_runners::native::RunnerBytes;
use futures_util::stream::{BoxStream, StreamExt as _, TryStreamExt as _};
use demi_backend_runners::install::{
    backend_url, powershell_script, read_runner_release, shell_script,
};
use demi_command_protocol::{is_digest, is_target};
use demi_runner_protocol::release::{COMPRESSED_SUFFIX, RunnerRelease, TOKEN_HEADER};

use demi_web_api_protocol::error::ErrorCode;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::AppState;
use super::cookies::Https;
use super::error::ApiError;

/// How the backend is reached from outside, and what it serves besides the
/// API.
#[derive(Debug, Default)]
pub struct Site {
    /// The URL the product's pages and the runners reach the backend at;
    /// without it, a request's own origin.
    pub public_url: Option<Url>,
    /// Whether a request showed that a proxy in front of the backend drops
    /// `Origin`, which the edge warns about once.
    pub origin_dropped: AtomicBool,
}

const UNCONFIGURED: &str = "Runner releases are not configured on this backend.\n";

/// The caching of an immutable executable's download: a year.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub(super) async fn shell(
    State(state): State<AppState>,
    https: Https,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    installer(
        &state,
        https,
        &headers,
        shell_script,
        "text/x-shellscript; charset=utf-8",
    )
    .await
}

pub(super) async fn powershell(
    State(state): State<AppState>,
    https: Https,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    installer(
        &state,
        https,
        &headers,
        powershell_script,
        "text/plain; charset=utf-8",
    )
    .await
}

/// The installer `script` writes for the current release, as `media_type`.
async fn installer(
    state: &AppState,
    Https(https): Https,
    headers: &HeaderMap,
    script: fn(&Url, &RunnerRelease) -> String,
    media_type: &'static str,
) -> Result<Response, ApiError> {
    let site = &state.site;
    let Some(release) = current_runner_release(state).await? else {
        return Ok((StatusCode::SERVICE_UNAVAILABLE, UNCONFIGURED).into_response());
    };
    let backend = match &site.public_url {
        Some(url) => url.clone(),
        None => request_origin(https, headers)?,
    };
    let backend =
        backend_url(&backend).map_err(|error| ApiError::internal_message(error.to_string()))?;
    let answer = (
        [
            (CONTENT_TYPE, HeaderValue::from_static(media_type)),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        script(&backend, &release),
    );
    Ok(answer.into_response())
}

/// The runner release the installers install and every paired device's
/// runner follows; none for a backend without runner releases.
pub(super) async fn current_runner_release(state: &AppState) -> Result<Option<RunnerRelease>, ApiError> {
    state
        .services
        .runner_releases
        .current()
        .await
        .map_err(ApiError::internal_message)
}

/// The origin the request came to, for a backend that names no public URL.
fn request_origin(https: bool, headers: &HeaderMap) -> Result<Url, ApiError> {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::invalid_query("The request names no host"))?;
    let scheme = if https { "https" } else { "http" };
    Url::parse(&format!("{scheme}://{host}/"))
        .map_err(|_| ApiError::invalid_query("The request's host is no URL"))
}

/// A release's executable for one target, immutable once published: the
/// backend's own release's, or the next one's an upgrade announced
/// (`native-runtime.md` § Runner releases). The compressed copy answers a
/// Range request from a byte on with 206, and what it serves a paired
/// device whose token the request carries counts as that device's
/// download (`runner.md` § Runner updates).
pub(super) async fn artifact(
    State(state): State<AppState>,
    Path((release, target, file)): Path<(String, String, String)>,
    range: Option<TypedHeader<Range>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let not_found = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "No such runner artifact",
        )
    };
    let Some(releases) = state.services.runner_releases.root() else {
        return Err(not_found());
    };
    let executable = if target.contains("windows") {
        "demi-runner.exe"
    } else {
        "demi-runner"
    };
    // The compressed copy as it is stored, for a runner's update, or the
    // executable decoded as it streams, for an installer, which has no zstd.
    let compressed = match file.strip_suffix(COMPRESSED_SUFFIX) {
        Some(name) if name == executable => true,
        None if file == executable => false,
        _ => return Err(not_found()),
    };
    if !is_digest(&release) || !is_target(&target) {
        return Err(not_found());
    }
    let next = state
        .services
        .prefetches
        .next()
        .filter(|next| next.release.release == release);
    let manifest = match &next {
        Some(next) => next.release.clone(),
        None => read_release(releases.join(&release).join("manifest.json"))
            .await?
            .ok_or_else(not_found)?,
    };
    if manifest.release != release {
        return Err(not_found());
    }
    let artifact = manifest.targets.get(&target).ok_or_else(not_found)?;
    // A download from a byte on, as one that broke off resumes; a range
    // with an end, or several, is answered with the whole, as HTTP allows.
    let from = match range.filter(|_| compressed) {
        Some(TypedHeader(range)) => match range.satisfiable_ranges(u64::MAX).collect::<Vec<_>>()[..] {
            [(Bound::Included(from), Bound::Unbounded)] => from,
            _ => 0,
        },
        None => 0,
    };
    // The download ends the need when its requester goes away.
    let cancel = CancellationToken::new();
    let _abandoned = cancel.clone().drop_guard();
    let stored = state
        .services
        .native
        .runner_executable(&target, artifact, next.as_deref(), from, &cancel)
        .await
        .map_err(ApiError::internal_message)?;
    let stored = match stored {
        RunnerBytes::Stored(stored) => stored,
        RunnerBytes::Beyond { size } => {
            let answer = (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(CONTENT_RANGE, format!("bytes */{size}"))],
            );
            return Ok(answer.into_response());
        }
    };
    if compressed {
        // A runner's download names its device, whose page shows its bytes
        // as they reach it: no cache may answer it, nor a proxy buffer it.
        let delivery = if headers.contains_key(TOKEN_HEADER) {
            Delivery::Counted
        } else {
            Delivery::Shared
        };
        let device = super::runners::paired_device(&state, &headers).await;
        return stored_download(counted(&state, device, stored.into()).await, delivery);
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(artifact.size));
    let decoded = demi_shared_artifacts::decode_stream(stored.into_stream());
    Ok((headers, Body::from_stream(decoded)).into_response())
}

/// `stored`, whose bytes count as `device`'s download as they are served:
/// its shard hears of each, until the download ends (`runner.md` § Runner
/// updates). Without a device, nothing counts.
async fn counted(state: &AppState, device: Option<DeviceRecord>, stored: Stored) -> Stored {
    let Some(device) = device else {
        return stored;
    };
    let download = Download {
        served: stored.range.start,
        size: stored.meta.size,
    };
    let (served, serving) = tokio::sync::watch::channel(download);
    let id = device.id;
    let told = state
        .shards
        .of(&device.user)
        .call(move |shard, _| async move { shard.runner_downloading(id, serving) })
        .await;
    if told.is_err() {
        // A closing backend counts nothing more; the bytes go out all the same.
        return stored;
    }
    stored.counting(move |count| {
        served.send_modify(|download| download.served += count);
    })
}

/// A local store's command executable, by its SHA-256, as S3 would serve
/// it: its compressed copy, in zstd. Only one of the catalog's that a
/// runner's need put in the store; any other digest answers 404.
pub(super) async fn native_artifact(
    State(state): State<AppState>,
    Path(sha256): Path<String>,
) -> Result<Response, ApiError> {
    let Some(artifact) = state.services.native.local_artifact(&sha256).await else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "No such native artifact",
        ));
    };
    let artifact = artifact.map_err(|error| ApiError::internal_message(error.to_string()))?;
    stored_download(artifact, Delivery::Shared)
}

/// How a stored artifact's download reaches its requester.
#[derive(Clone, Copy)]
enum Delivery {
    /// Through any cache, for a year.
    Shared,
    /// Straight from this backend, which counts its bytes as they go out:
    /// no shared cache keeps it, and no proxy in front buffers it.
    Counted,
}

/// A stored artifact, or the part of it from a byte on, as an immutable
/// download delivered as `delivery` says: the stored bytes with the content
/// coding they were stored with.
fn stored_download(stored: impl Into<Stored>, delivery: Delivery) -> Result<Response, ApiError> {
    let stored = stored.into();
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    match delivery {
        Delivery::Shared => {
            headers.insert(CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
        }
        Delivery::Counted => {
            headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
            headers.insert(
                HeaderName::from_static("x-accel-buffering"),
                HeaderValue::from_static("no"),
            );
        }
    }
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let range = stored.range.clone();
    let size = stored.meta.size;
    headers.insert(CONTENT_LENGTH, HeaderValue::from(range.end - range.start));
    let status = if range.start > 0 {
        let whole = format!("bytes {}-{}/{size}", range.start, range.end.saturating_sub(1));
        headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&whole).map_err(|error| ApiError::internal_message(error.to_string()))?,
        );
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    if let Some(coding) = stored
        .attributes
        .get(&object_store::Attribute::ContentEncoding)
    {
        let coding = HeaderValue::from_str(coding.as_ref())
            .map_err(|error| ApiError::internal_message(error.to_string()))?;
        headers.insert(CONTENT_ENCODING, coding);
    }
    Ok((status, headers, Body::from_stream(stored.stream)).into_response())
}

/// A stored artifact on its way out: its bytes, from where they start, and
/// what the store says of it.
struct Stored {
    stream: BoxStream<'static, object_store::Result<Bytes>>,
    range: std::ops::Range<u64>,
    meta: object_store::ObjectMeta,
    attributes: object_store::Attributes,
}

impl Stored {
    /// The same bytes, each chunk counted by `count` as it goes out.
    fn counting(self, mut count: impl FnMut(u64) + Send + 'static) -> Self {
        let stream = self
            .stream
            .inspect_ok(move |chunk| count(chunk.len() as u64))
            .boxed();
        Self { stream, ..self }
    }
}

impl From<object_store::GetResult> for Stored {
    fn from(stored: object_store::GetResult) -> Self {
        Self {
            range: stored.range.clone(),
            meta: stored.meta.clone(),
            attributes: stored.attributes.clone(),
            stream: stored.into_stream(),
        }
    }
}

/// The release record at `path`, when there is one; a record that does not
/// decode is the deployment's fault.
async fn read_release(path: PathBuf) -> Result<Option<RunnerRelease>, ApiError> {
    read_runner_release(&path, RunnerRelease::decode)
        .await
        .map_err(ApiError::internal_message)
}
