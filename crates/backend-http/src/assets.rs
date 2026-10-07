//! The web app build served beside the API (`web-api.md` § Serving the web
//! app build): files as they are, and `index.html` for an extensionless
//! navigation that accepts HTML, so a deep page reloads. Any other miss is a
//! JSON 404. The build's `build.json` names it. `index.html` and
//! `build.json` are asked for again on every load and compared by their
//! content, never by their modification time, since a new build's files
//! may be older than the old build's, as after a rollback; the hashed files
//! under `assets/` never change.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::handler::HandlerWithoutStateExt as _;
use axum::http::{Method, StatusCode};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum_extra::headers::{CacheControl, ETag, HeaderMapExt as _, IfNoneMatch};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tower::ServiceBuilder;
use tower_http::services::ServeDir;

use super::error::ApiError;

/// `build.json` of a web app build: the build's id, which `vite build`
/// writes into the page too.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildFile {
    build: String,
}

/// Why a web app directory names no build.
#[derive(Debug, thiserror::Error)]
#[error("{} does not name the web app build: {reason}", path.display())]
pub struct WebBuildError {
    path: PathBuf,
    reason: String,
}

/// The build of the web app in `directory`, from its `build.json`.
pub fn web_build(directory: &Path) -> Result<String, WebBuildError> {
    let path = directory.join("build.json");
    let failed = |reason: String| WebBuildError {
        path: path.clone(),
        reason,
    };
    let text = std::fs::read_to_string(&path).map_err(|error| failed(error.to_string()))?;
    let file: BuildFile = serde_json::from_str(&text).map_err(|error| failed(error.to_string()))?;
    Ok(file.build)
}

/// Where the hashed files of a build lie, whose names change with their content.
const HASHED: &str = "/assets/";

/// How long a hashed file may be kept: a year, as long as caches keep anything.
const HASHED_MAX_AGE: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// `app` with the files of `directory` behind every path no route answers.
pub(super) fn serve<S: Clone + Send + Sync + 'static>(
    app: Router<S>,
    directory: PathBuf,
) -> Router<S> {
    let index = directory.join("index.html");
    let build = directory.join("build.json");
    let page = {
        let index = index.clone();
        move |request: Request| page(index.clone(), request)
    };
    // `/` is a navigation like any other, which the page answers.
    let files = ServeDir::new(directory)
        .append_index_html_on_directories(false)
        .call_fallback_on_method_not_allowed(true)
        .fallback(page.into_service());
    let files = ServiceBuilder::new()
        .layer(middleware::from_fn(hashed_immutable))
        .service(files);
    app.route(
        "/index.html",
        get(move |request: Request| revalidated(index.clone(), "text/html", request)),
    )
    .route(
        "/build.json",
        get(move |request: Request| revalidated(build.clone(), "application/json", request)),
    )
    .fallback_service(files)
}

async fn page(index: PathBuf, request: Request) -> Response {
    let navigation = matches!(*request.method(), Method::GET | Method::HEAD)
        && Path::new(request.uri().path()).extension().is_none()
        && request
            .headers()
            .get(ACCEPT)
            .and_then(|accept| accept.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"));
    if !navigation {
        return ApiError::no_route(request.method(), request.uri().path()).into_response();
    }
    revalidated(index, "text/html", request).await
}

/// A file of the build that names it, `index.html` or `build.json`, under
/// `Cache-Control: no-cache` and an `ETag` of its content: a request whose
/// `If-None-Match` names that content answers 304, whatever the file's
/// modification time. A file the build lacks is a 404.
async fn revalidated(path: PathBuf, content_type: &'static str, request: Request) -> Response {
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(path = %path.display(), "a file of the web app could not be read: {error}");
            return ApiError::no_route(request.method(), request.uri().path()).into_response();
        }
    };
    let etag: ETag = format!("\"{:x}\"", Sha256::digest(&bytes))
        .parse()
        .expect("a quoted hex digest is an entity tag");
    let unchanged = request
        .headers()
        .typed_get::<IfNoneMatch>()
        .is_some_and(|held| !held.precondition_passes(&etag));
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(CONTENT_TYPE, content_type)], Body::from(bytes)).into_response()
    };
    let headers = response.headers_mut();
    headers.typed_insert(etag);
    headers.typed_insert(CacheControl::new().with_no_cache());
    response
}

/// Marks a hashed file the build served as one that never changes.
async fn hashed_immutable(request: Request, next: Next) -> Response {
    let hashed = request.uri().path().starts_with(HASHED);
    let mut response = next.run(request).await;
    let served = response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED;
    if hashed && served {
        response.headers_mut().typed_insert(
            CacheControl::new()
                .with_public()
                .with_max_age(HASHED_MAX_AGE)
                .with_immutable(),
        );
    }
    response
}
