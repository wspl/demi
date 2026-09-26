//! The public installation routes (`web-api.md` § Resource index): the
//! runner installers for the backend's current runner release, and the
//! runner executables of each release, from the directory
//! `DEMI_RUNNER_RELEASE_DIR` names (`builds-and-releases.md` § Packaging).
//! Neither holds a credential; pairing grants device access. Without the
//! directory, the installers answer 503 and the executables 404.

use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, HOST};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_command_service::protocol::{is_digest, is_target};
use demi_runner_protocol::release::RunnerRelease;
use demi_web_api::error::ErrorCode;
use tokio_util::io::ReaderStream;
use url::Url;

use super::AppState;
use super::cookies::Https;
use super::error::ApiError;
use crate::runner::install::{backend_url, powershell_script, shell_script};

/// How the backend is reached from outside, and what it serves besides the
/// API.
#[derive(Debug, Clone, Default)]
pub(crate) struct Site {
    /// The URL the product's pages and the runners reach the backend at;
    /// without it, a request's own origin.
    pub(crate) public_url: Option<Url>,
    /// The runner releases the installers serve: `manifest.json` names the
    /// current one, and each release's directory holds its own and one
    /// executable per target.
    pub(crate) runner_releases: Option<PathBuf>,
}

const UNCONFIGURED: &str = "Runner releases are not configured on this backend.\n";

/// How much of a runner executable one read sends. `ReaderStream` reads 4 KiB
/// by default, which cut a runner of many megabytes into tens of thousands of
/// chunks.
const ARTIFACT_READ: usize = 256 * 1024;

pub(super) async fn shell(State(state): State<AppState>, https: Https, headers: HeaderMap) -> Result<Response, ApiError> {
    installer(&state, https, &headers, shell_script, "text/x-shellscript; charset=utf-8").await
}

pub(super) async fn powershell(State(state): State<AppState>, https: Https, headers: HeaderMap) -> Result<Response, ApiError> {
    installer(&state, https, &headers, powershell_script, "text/plain; charset=utf-8").await
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
    let Some(releases) = &site.runner_releases else {
        return Ok((StatusCode::SERVICE_UNAVAILABLE, UNCONFIGURED).into_response());
    };
    let release = read_release(releases.join("manifest.json"))
        .await?
        .ok_or_else(|| ApiError::internal_message("the runner release directory has no manifest.json"))?;
    let backend = match &site.public_url {
        Some(url) => url.clone(),
        None => request_origin(https, headers)?,
    };
    let backend = backend_url(&backend).map_err(|error| ApiError::internal_message(error.to_string()))?;
    let answer = (
        [
            (CONTENT_TYPE, HeaderValue::from_static(media_type)),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        script(&backend, &release),
    );
    Ok(answer.into_response())
}

/// The origin the request came to, for a backend that names no public URL.
fn request_origin(https: bool, headers: &HeaderMap) -> Result<Url, ApiError> {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::invalid_query("The request names no host"))?;
    let scheme = if https { "https" } else { "http" };
    Url::parse(&format!("{scheme}://{host}/")).map_err(|_| ApiError::invalid_query("The request's host is no URL"))
}

/// A release's executable for one target, immutable once published.
pub(super) async fn artifact(
    State(state): State<AppState>,
    Path((release, target, file)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "No such runner artifact");
    let Some(releases) = &state.site.runner_releases else {
        return Err(not_found());
    };
    let executable = if target.contains("windows") {
        "demi-runner.exe"
    } else {
        "demi-runner"
    };
    if !is_digest(&release) || !is_target(&target) || file != executable {
        return Err(not_found());
    }
    let directory = releases.join(&release);
    let manifest = read_release(directory.join("manifest.json")).await?.ok_or_else(not_found)?;
    if manifest.release != release || !manifest.targets.contains_key(&target) {
        return Err(not_found());
    }
    let opened = match tokio::fs::File::open(directory.join(&target).join(executable)).await {
        Ok(opened) => opened,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(not_found()),
        Err(error) => return Err(ApiError::internal_message(error.to_string())),
    };
    let size = opened
        .metadata()
        .await
        .map_err(|error| ApiError::internal_message(error.to_string()))?
        .len();
    let headers = [
        (CONTENT_TYPE, HeaderValue::from_static("application/octet-stream")),
        (CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable")),
        (CONTENT_LENGTH, HeaderValue::from(size)),
    ];
    Ok((headers, Body::from_stream(ReaderStream::with_capacity(opened, ARTIFACT_READ))).into_response())
}

/// The release record at `path`, when there is one; a record that does not
/// decode is the deployment's fault.
async fn read_release(path: PathBuf) -> Result<Option<RunnerRelease>, ApiError> {
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ApiError::internal_message(error.to_string())),
    };
    RunnerRelease::decode(&bytes)
        .map(Some)
        .map_err(|error| ApiError::internal_message(format!("{}: {error}", path.display())))
}
