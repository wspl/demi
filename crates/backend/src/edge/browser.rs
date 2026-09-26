//! The conversation browser's tab routes (`web-api.md` § Conversation
//! browser tabs): each runs the operation the agent's `demi browser`
//! command runs, in the package the `browser` user stream declares, as a
//! one-shot user call. Listing, closing and moving a tab never wake a
//! stopped Cloud, and an archive, a switch or a detach ends them; opening a
//! tab is ordinary demand. Listing is a look, which is no activity; the
//! others operate the browser (`resource-lifecycle.md` § Activity). The
//! backend holds no browser logic.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use bytes::Bytes;
use demi_builtin_protocol::browser::{
    BackInput, BrowserCreatedBy, BrowserErrorCode, BrowserFailure, BrowserOperation, BrowserTab, CloseInput,
    FailureDocument, ForwardInput, GotoInput, OpenInput, OpenResult, PREFIX, ReloadInput, TabId, TabsInput, TabsResult,
};
use demi_host_remote::ServiceCallError;
use demi_shell::HostErrorKind;
use demi_web_api::browser::{BrowserTabs, HistoryAction, NavigateTab, OpenTab, TabHistory};
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::UserId;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::AppState;
use super::body::JsonBody;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::conversation::host_access::{HostAccessError, Refusal};
use crate::conversation::stream::{BROWSER_STREAM, ServiceBinding, ServiceCall, UserCallError, UserCallKind};
use crate::storage::conversation_index::ConversationRecord;

/// The most bytes of an operation's JSON answer.
const ANSWER_MAX_BYTES: usize = 1024 * 1024;

pub(super) async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<BrowserTabs>, ApiError> {
    let record = open_conversation(&state, &user.id, &id).await?;
    let input = TabsInput {
        offset: None,
        limit: None,
        timeout: None,
    };
    let listed = call(&state, &user.id, record, UserCallKind::Looks, BrowserOperation::Tabs, input).await?;
    // A stopped Cloud runs no browser.
    let tabs = match stopped_is_none(listed)? {
        Some(answer) => decode::<TabsResult>(&answer)?.tabs,
        None => Vec::new(),
    };
    Ok(Json(BrowserTabs { tabs }))
}

/// Opens a tab as the user, starting the browser when needed; the page
/// shows its loading, so the answer does not wait for it.
pub(super) async fn open(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<OpenTab>,
) -> Result<Json<BrowserTab>, ApiError> {
    let record = open_conversation(&state, &user.id, &id).await?;
    let input = OpenInput {
        url: request.url.unwrap_or_else(|| "about:blank".into()),
        load: None,
        timeout: None,
    };
    let answer = call(&state, &user.id, record, UserCallKind::Starts, BrowserOperation::Open, input)
        .await?
        .map_err(refused)?;
    let opened = decode::<OpenResult>(&answer)?;
    Ok(Json(BrowserTab {
        id: opened.tab,
        title: opened.title.unwrap_or_default(),
        url: opened.url,
        created_by: BrowserCreatedBy::User {},
    }))
}

/// Closes a tab; a tab the browser no longer has, or a stopped Cloud's, is
/// closed already.
pub(super) async fn close(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, tab)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let Ok(tab) = TabId::try_from(tab) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let input = CloseInput { tab, timeout: None };
    match call(&state, &user.id, record, UserCallKind::Operates, BrowserOperation::Close, input).await? {
        Ok(_) | Err(UserCallError::Access(HostAccessError::Refused(Refusal::Stopped))) => Ok(StatusCode::NO_CONTENT),
        Err(error) if browser_failure(&error).is_some_and(|failure| failure.code == BrowserErrorCode::TabNotFound) => {
            Ok(StatusCode::NO_CONTENT)
        }
        Err(error) => Err(refused(error)),
    }
}

pub(super) async fn navigate(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, tab)): Path<(String, String)>,
    JsonBody(NavigateTab { url }): JsonBody<NavigateTab>,
) -> Result<StatusCode, ApiError> {
    let record = open_conversation(&state, &user.id, &id).await?;
    let input = GotoInput {
        tab: tab_id(tab)?,
        url,
        load: None,
        timeout: None,
    };
    on_tab(call(&state, &user.id, record, UserCallKind::Operates, BrowserOperation::Goto, input).await?)
}

pub(super) async fn history(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, tab)): Path<(String, String)>,
    JsonBody(TabHistory { action }): JsonBody<TabHistory>,
) -> Result<StatusCode, ApiError> {
    let record = open_conversation(&state, &user.id, &id).await?;
    let tab = tab_id(tab)?;
    let answered = match action {
        HistoryAction::Back => {
            let input = BackInput {
                tab,
                load: None,
                timeout: None,
            };
            call(&state, &user.id, record, UserCallKind::Operates, BrowserOperation::Back, input).await?
        }
        HistoryAction::Forward => {
            let input = ForwardInput {
                tab,
                load: None,
                timeout: None,
            };
            call(&state, &user.id, record, UserCallKind::Operates, BrowserOperation::Forward, input).await?
        }
        HistoryAction::Reload => {
            let input = ReloadInput {
                tab,
                load: None,
                timeout: None,
            };
            call(&state, &user.id, record, UserCallKind::Operates, BrowserOperation::Reload, input).await?
        }
    };
    on_tab(answered)
}

/// The user's conversation that is not archived.
async fn open_conversation(state: &AppState, user: &UserId, id: &str) -> Result<ConversationRecord, ApiError> {
    let record = owned(&state.services, user, id).await?;
    if record.archived {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationArchived,
            "The conversation is archived",
        ));
    }
    Ok(record)
}

/// Runs `operation`'s one-shot call with `input` on the conversation's
/// main Host, in the package the `browser` stream declares. The outer
/// error is the backend's own: no browser on this backend, or a closing
/// shard.
async fn call<I: Serialize>(
    state: &AppState,
    user: &UserId,
    record: ConversationRecord,
    kind: UserCallKind,
    operation: fn(I) -> BrowserOperation,
    input: I,
) -> Result<Result<Bytes, UserCallError>, ApiError> {
    // A backend whose catalog serves no browser has no tab routes.
    let browser = state
        .services
        .user_streams
        .get(BROWSER_STREAM)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "This backend runs no conversation browser"))?;
    let args = match serde_json::to_value(&input) {
        Ok(serde_json::Value::Object(args)) => args,
        _ => unreachable!("a browser operation's input serializes to an object"),
    };
    let call = ServiceCall {
        binding: ServiceBinding {
            package: browser.package.clone(),
            operation: format!("{PREFIX}{}", operation(input).name()),
        },
        args,
        max_bytes: ANSWER_MAX_BYTES,
    };
    let answered = state
        .shards
        .of(user)
        .call(move |shard, cancel| async move { shard.user_call(&record.id, kind, &call, &cancel).await })
        .await?;
    Ok(answered)
}

/// An answer, or none when the Cloud is stopped.
fn stopped_is_none(answered: Result<Bytes, UserCallError>) -> Result<Option<Bytes>, ApiError> {
    match answered {
        Ok(answer) => Ok(Some(answer)),
        Err(UserCallError::Access(HostAccessError::Refused(Refusal::Stopped))) => Ok(None),
        Err(error) => Err(refused(error)),
    }
}

/// What an operation on one tab answers: 204, or 409 `host_stopped` for a
/// stopped Cloud, which it never wakes.
fn on_tab(answered: Result<Bytes, UserCallError>) -> Result<StatusCode, ApiError> {
    answered.map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The tab a route names; one that is no tab's id is a tab the browser does
/// not have.
fn tab_id(tab: String) -> Result<TabId, ApiError> {
    TabId::try_from(tab).map_err(|_| tab_not_found("No such tab"))
}

fn tab_not_found(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::TabNotFound, message)
}

/// An operation's answer as its type: one the backend cannot read is the
/// browser's failure.
fn decode<T: DeserializeOwned>(answer: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(answer)
        .map_err(|error| browser_failed(format!("The browser answered what the backend cannot read: {error}")))
}

fn browser_failed(message: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, ErrorCode::BrowserFailed, message)
}

/// The browser's own failure, when the call failed with one: what the
/// operation wrote to its standard error.
fn browser_failure(error: &UserCallError) -> Option<BrowserFailure> {
    let UserCallError::Call(ServiceCallError::Exited { stderr, .. }) = error else {
        return None;
    };
    let document = serde_json::from_str::<FailureDocument>(stderr.trim()).ok()?;
    Some(document.error)
}

/// What a failed call answers: the host access's own refusal, a tab the
/// browser does not have, or the browser's failure with its code and words.
fn refused(error: UserCallError) -> ApiError {
    if let Some(failure) = browser_failure(&error) {
        if failure.code == BrowserErrorCode::TabNotFound {
            return tab_not_found(failure.message);
        }
        return browser_failed(format!("{}: {}", failure.code, failure.message));
    }
    match error {
        UserCallError::Access(error) => error.into(),
        UserCallError::Call(ServiceCallError::Host(error)) if matches!(error.kind, HostErrorKind::Offline) => {
            HostAccessError::Host(error).into()
        }
        UserCallError::Call(error) => browser_failed(error.to_string()),
    }
}
