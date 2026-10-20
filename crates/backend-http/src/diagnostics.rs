//! The page's own error reports and the server log.
//!
//! - `POST /page-errors` logs a failure the page found itself under the
//!   report's request ID.
//! - `GET /server/log` is how an administrator finds a request's lines.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use demi_backend_database::server_log::LogQuery;
use demi_web_api_protocol::diagnostics::{PageErrorReport, ServerLog, ServerLogQuery};
use demi_web_api_protocol::error::ErrorCode;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::{AdminUser, CurrentSession};
use super::query::QueryParams;

/// `POST /page-errors`: logged as a warning under the request's ID, which
/// the answer's `X-Request-Id` gives the page to show; 429 past thirty a
/// minute.
pub(super) async fn report(
    State(state): State<AppState>,
    session: CurrentSession,
    JsonBody(report): JsonBody<PageErrorReport>,
) -> Result<StatusCode, ApiError> {
    if !state.services.page_errors.attempt(&session.id) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "The page reported more errors than a minute allows",
        )
        .with_retry_after(Duration::from_secs(60)));
    }
    tracing::warn!(
        title = report.title,
        error = report.message,
        detail = report.detail,
        "the page reported an error"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /server/log`: an administrator's.
pub(super) async fn log(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    QueryParams(query): QueryParams<ServerLogQuery>,
) -> Result<Json<ServerLog>, ApiError> {
    let read = state
        .log
        .read(LogQuery {
            request: query.request,
            since: query.since,
            limit: query.limit.get(),
            level: query.level,
        })
        .await?;
    Ok(Json(ServerLog {
        lines: read.lines,
        next: read.next,
    }))
}
