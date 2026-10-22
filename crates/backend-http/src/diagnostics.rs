//! The page's lines of the log: `POST /log`, the one route they reach the
//! backend by.
//!
//! The route stores each line as the page's, with the user and the carrying
//! request's ID, so that the ID a toast shows finds the line the failure wrote.

use axum::extract::State;
use axum::http::StatusCode;
use demi_shared_types::{LogLine, LogOrigin};
use demi_web_api_protocol::diagnostics::PageLog;

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::request_id;

/// `POST /log`: 204 once the lines are stored.
pub(super) async fn log(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(PageLog { page, lines }): JsonBody<PageLog>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id::current().map(|id| id.to_string());
    let lines = lines
        .into_iter()
        .map(|line| {
            let mut stored = LogLine::new(
                i64::try_from(line.at).unwrap_or(i64::MAX),
                LogOrigin::Page,
                line.level.into(),
                line.event,
            );
            stored.request_id.clone_from(&request_id);
            stored.user_id = Some(user.id.to_string());
            stored.conversation_id = line.conversation_id;
            stored.page_id = Some(page.clone());
            stored.fields = line.fields.into_iter().collect();
            stored
        })
        .collect();
    state.services.log.store_page(lines);
    Ok(StatusCode::NO_CONTENT)
}
