//! Plugin calls at the edge.
//!
//! - A page's call of a plugin's method, for the user or for one of the user's
//!   conversations.
//! - The read of a plugin's state for one conversation (§ Conversation state of
//!   plugins).
//! - A file a plugin's pages load, such as a library its documents run, which
//!   its factory gives, for anyone ([`file`]).
//!
//! The user's core checks the parameters against the method's schema, and the
//! user's instance answers. What the call changes reaches the user's pages on
//! the synchronization channel.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::HeaderMap;
use axum::http::header::{
    ACCEPT_ENCODING, ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, VARY,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::response::{IntoResponse, Response};
use demi_plugin_host::{PageCall, PageCallError, SwitchError};
use demi_core::ReloadRefusal;
use demi_plugin_interface::{PluginError, PortRefusal};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::plugins::{PluginStateAnswer, PluginSwitch};
use serde_json::Value;

use super::AppState;
use super::body::{JsonBody, JsonValueBody};
use super::conversations::owned;
use super::error::{ApiError, status_of};
use super::gate::AuthUser;

/// `GET /api/plugins/:plugin/files/:name`: a file the plugin's pages load.
///
/// No session gates it: a sandboxed frame sends no cookie and a share's
/// reader has none, and a file is the same for every user, a plugin's whom
/// it is off for included. A frame's opaque origin reads it with `fetch`, so
/// any origin may, a refusal included.
///
/// Its name holds its version, so the answer may be kept for good, but Chrome
/// keeps nothing an opaque origin's frame loads in its HTTP cache: every
/// frame that shows fetches the file again. The gzip copy a file has answers
/// a browser that accepts gzip, which every one does.
pub(super) async fn file(
    State(state): State<AppState>,
    Path((plugin, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(file) = state.services.plugins.file(&plugin, &name) else {
        let mut refusal = ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "No such file").into_response();
        refusal
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, axum::http::HeaderValue::from_static("*"));
        return refusal;
    };
    let common = [
        (CONTENT_TYPE, file.content_type),
        (CACHE_CONTROL, "public, max-age=31536000, immutable"),
        (ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (VARY, "Accept-Encoding"),
    ];
    match file.gzip.filter(|_| accepts_gzip(&headers)) {
        Some(gzip) => (common, [(CONTENT_ENCODING, "gzip")], gzip).into_response(),
        None => (common, file.bytes).into_response(),
    }
}

/// Whether `Accept-Encoding` (RFC 9110 § 12.5.3) names gzip, or any coding,
/// without refusing it by `q=0`.
fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|coding| {
            let mut parts = coding.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default();
            let refused = parts.any(|parameter| {
                parameter
                    .strip_prefix("q=")
                    .and_then(|weight| weight.parse::<f32>().ok())
                    .is_some_and(|weight| weight == 0.0)
            });
            (name.eq_ignore_ascii_case("gzip") || name == "*") && !refused
        })
}

/// `PUT /plugins/:plugin { enabled }`: turns the plugin on or off for the
/// caller.
pub(super) async fn switch(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(plugin): Path<String>,
    JsonBody(PluginSwitch { enabled }): JsonBody<PluginSwitch>,
) -> Result<StatusCode, ApiError> {
    let switched = state
        .shards
        .of(&user.id)
        .call(move |core, _| async move { core.switch_plugin(&plugin, enabled).await })
        .await?;
    switch_answer(switched)
}

/// `PUT /plugins/:plugin/features/:feature { enabled }`: turns the
/// plugin's feature on or off for the caller.
pub(super) async fn switch_feature(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((plugin, feature)): Path<(String, String)>,
    JsonBody(PluginSwitch { enabled }): JsonBody<PluginSwitch>,
) -> Result<StatusCode, ApiError> {
    let switched = state
        .shards
        .of(&user.id)
        .call(move |core, _| async move { core.switch_plugin_feature(&plugin, &feature, enabled).await })
        .await?;
    switch_answer(switched)
}

/// The answer to a switch of a plugin or of one of its features.
fn switch_answer(switched: Result<(), SwitchError>) -> Result<StatusCode, ApiError> {
    match switched {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(error @ (SwitchError::UnknownPlugin(_) | SwitchError::UnknownFeature { .. })) => Err(ApiError::new(
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
        .call(move |core, _| async move { core.reload_conversation(&record.id).await })
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

/// `GET /conversations/:id/plugins/:plugin/state`, for a conversation the
/// user owns.
pub(super) async fn conversation_state(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, plugin)): Path<(String, String)>,
) -> Result<Json<PluginStateAnswer>, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    let answer = state
        .shards
        .of(&user.id)
        .call(move |core, cancel| async move {
            core
                .plugins()
                .conversation_state(&plugin, record.id, cancel)
                .await
        })
        .await?
        .map_err(refused)?;
    Ok(Json(answer))
}

async fn page_call(
    state: &AppState,
    user: &UserId,
    call: PageCall,
) -> Result<Json<Value>, ApiError> {
    let result = state
        .shards
        .of(user)
        .call(move |core, cancel| async move { core.plugins().page_call(call, cancel).await })
        .await?
        .map_err(refused)?;
    Ok(Json(result))
}

/// What a call or a state read that did not answer answers.
pub(super) fn refused(error: PageCallError) -> ApiError {
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
            tracing::warn!(event = "plugin_page_call.failed", error = message, "a plugin's page call failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::PluginFailed,
                message,
            )
        }
    }
}
