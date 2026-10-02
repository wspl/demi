//! Plugin calls at the edge (`web-api.md` § Plugin calls): a page's call of
//! a plugin's method, for the user or for one of the user's conversations.
//! The user's shard checks the parameters against the method's schema and
//! the user's instance answers; what the call changes reaches the user's
//! pages on the synchronization channel.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_plugins::{PageCall, PageCallError, SwitchError};
use demi_backend_user_shard::shard::ReloadRefusal;
use demi_plugin_interface::{PluginError, PortRefusal};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::plugins::PluginSwitch;
use serde_json::Value;

use super::AppState;
use super::body::{JsonBody, JsonValueBody};
use super::conversations::owned;
use super::error::{ApiError, status_of};
use super::gate::AuthUser;

/// `PUT /plugins/:plugin { enabled }`: turns the plugin on or off for the
/// caller (`web-api.md` § A user's plugins).
pub(super) async fn switch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(plugin): Path<String>,
    JsonBody(PluginSwitch { enabled }): JsonBody<PluginSwitch>,
) -> Result<StatusCode, ApiError> {
    let switched = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.switch_plugin(&plugin, enabled).await })
        .await?;
    match switched {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(error @ SwitchError::UnknownPlugin(_)) => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::UnknownPlugin,
            error.to_string(),
        )),
        Err(SwitchError::Storage(error)) => Err(error.into()),
    }
}

/// `POST /conversations/:id/reload`: closes the conversation's tree so
/// that it opens again with the caller's current plugins.
pub(super) async fn reload(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let reloaded = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { shard.reload_conversation(&record.id).await })
        .await?;
    match reloaded {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(ReloadRefusal::Access(error)) => Err(error.into()),
        Err(error @ ReloadRefusal::Archived) => Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationArchived,
            error.to_string(),
        )),
        Err(error @ ReloadRefusal::Working) => Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::TurnInFlight,
            error.to_string(),
        )),
    }
}

/// `POST /plugins/:plugin/calls/:method`.
pub(super) async fn user_call(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((plugin, method)): Path<(String, String)>,
    JsonValueBody(params): JsonValueBody,
) -> Result<Json<Value>, ApiError> {
    let call = PageCall {
        plugin,
        method,
        params,
        conversation: None,
    };
    page_call(&state, &user.id, call).await
}

/// `POST /conversations/:id/plugins/:plugin/calls/:method`, for a
/// conversation the user owns.
pub(super) async fn conversation_call(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, plugin, method)): Path<(String, String, String)>,
    JsonValueBody(params): JsonValueBody,
) -> Result<Json<Value>, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let call = PageCall {
        plugin,
        method,
        params,
        conversation: Some(record.id),
    };
    page_call(&state, &user.id, call).await
}

async fn page_call(
    state: &AppState,
    user: &UserId,
    call: PageCall,
) -> Result<Json<Value>, ApiError> {
    let result = state
        .shards
        .of(user)
        .call(move |shard, cancel| async move { shard.plugins().page_call(call, cancel).await })
        .await?
        .map_err(refused)?;
    Ok(Json(result))
}

/// What a call that did not answer answers (`web-api.md` § Plugin calls).
fn refused(error: PageCallError) -> ApiError {
    let message = error.to_string();
    match error {
        PageCallError::UnknownPlugin(_) => {
            ApiError::new(StatusCode::NOT_FOUND, ErrorCode::UnknownPlugin, message)
        }
        PageCallError::Disabled(_) => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::PluginDisabled, message)
        }
        PageCallError::UnknownMethod { .. } => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::UnknownPluginMethod,
            message,
        ),
        PageCallError::InvalidParams(_) | PageCallError::Plugin(PluginError::Usage { .. }) => {
            ApiError::invalid_body(message)
        }
        PageCallError::Plugin(PluginError::Refused { reason, message }) => {
            ApiError::new(StatusCode::CONFLICT, ErrorCode::PluginRefused, message)
                .with_reason(reason)
        }
        PageCallError::Plugin(PluginError::Port {
            refusal:
                PortRefusal::Host {
                    code,
                    status,
                    message,
                },
        }) => ApiError::new(status_of(status), code, message),
        PageCallError::Plugin(PluginError::Ended { .. }) => ApiError::backend_closing(),
        PageCallError::Plugin(PluginError::Port { .. } | PluginError::Failed { .. }) => {
            tracing::warn!(error = message, "a plugin's page call failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::PluginFailed,
                message,
            )
        }
    }
}
