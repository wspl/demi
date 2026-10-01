//! The browser build served beside the API (`web-api.md` § Serving the
//! browser build): files as they are, and `index.html` for an extensionless
//! navigation that accepts HTML, so a deep page reloads. Any other miss is a
//! JSON 404.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::extract::Request;
use axum::handler::HandlerWithoutStateExt as _;
use axum::http::header::ACCEPT;
use axum::response::{IntoResponse, Response};
use tower::ServiceExt as _;
use tower_http::services::{ServeDir, ServeFile};

use super::error::ApiError;

/// `app` with the files of `directory` behind every path no route answers.
pub(super) fn serve<S: Clone + Send + Sync + 'static>(app: Router<S>, directory: PathBuf) -> Router<S> {
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
