//! The web app build served beside the API (`web-api.md` § Serving the web
//! app build): files as they are, and `index.html` for an extensionless
//! navigation that accepts HTML, so a deep page reloads. Any other miss is a
//! JSON 404. The build's `build.json` names it.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::extract::Request;
use axum::handler::HandlerWithoutStateExt as _;
use axum::http::header::ACCEPT;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tower::ServiceExt as _;
use tower_http::services::{ServeDir, ServeFile};

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

/// `app` with the files of `directory` behind every path no route answers.
pub(super) fn serve<S: Clone + Send + Sync + 'static>(
    app: Router<S>,
    directory: PathBuf,
) -> Router<S> {
    let index = directory.join("index.html");
    let page = move |request: Request| page(index.clone(), request);
    let files = ServeDir::new(directory)
        .call_fallback_on_method_not_allowed(true)
        .fallback(page.into_service());
    app.fallback_service(files)
}

async fn page(index: PathBuf, request: Request) -> Response {
    let navigation = Path::new(request.uri().path()).extension().is_none()
        && request
            .headers()
            .get(ACCEPT)
            .and_then(|accept| accept.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"));
    if !navigation {
        return ApiError::no_route(request.method(), request.uri().path()).into_response();
    }
    match ServeFile::new(index).oneshot(request).await {
        Ok(response) => response.into_response(),
        Err(never) => match never {},
    }
}
