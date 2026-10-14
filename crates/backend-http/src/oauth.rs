//! A plugin's OAuth callback (`plugins.md` § OAuth sign-in): where the
//! vendor sends the browser back after the user signed in. It is a session
//! route: the browser's own navigation carries the session cookie, so the
//! caller's shard completes only a sign-in its user started. It answers a
//! page the user reads in the tab the sign-in opened.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use demi_backend_plugins::{Callback, CallbackError};
use demi_shared_types::xml_escaped;

use super::AppState;
use super::error::ApiError;
use super::gate::AuthUser;

/// `GET /plugins/:plugin/oauth/callback?code&state`, or `?error&state`
/// when the user declined at the vendor: 200 once signed in; 400 for a
/// `state` the caller did not start, one older than 10 minutes or used
/// before, or a vendor's error; 404 for a plugin that does not sign in or
/// that the caller has off; 502 when the vendor did not issue the tokens.
pub(super) async fn callback(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(plugin): Path<String>,
    Query(callback): Query<Callback>,
) -> Result<(StatusCode, Html<String>), ApiError> {
    let completed = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard.plugins().oauth_callback(&plugin, callback, cancel).await
        })
        .await?;
    Ok(match completed {
        Ok(vendor) => (
            StatusCode::OK,
            page(
                &format!("Signed in to {vendor}"),
                &format!("Signed in to {vendor}. You can close this tab."),
            ),
        ),
        Err(CallbackError::UnknownPlugin(_)) => (
            StatusCode::NOT_FOUND,
            page(
                "Sign-in not found",
                "Nothing in Demi signs in here. Close this tab.",
            ),
        ),
        Err(CallbackError::UnknownState) => (
            StatusCode::BAD_REQUEST,
            page(
                "Sign-in expired",
                "This sign-in expired or was already used. Close this tab and sign in again in Demi.",
            ),
        ),
        Err(CallbackError::Denied { vendor, error }) => (
            StatusCode::BAD_REQUEST,
            page(
                &format!("Not signed in to {vendor}"),
                &format!("{vendor} didn't sign you in ({error}). Close this tab and try again in Demi."),
            ),
        ),
        Err(CallbackError::Storage(error)) => return Err(error.into()),
        Err(CallbackError::Exchange { vendor, failure }) => {
            tracing::warn!(vendor, %failure, "a plugin's sign-in did not complete");
            (
                StatusCode::BAD_GATEWAY,
                page(
                    &format!("Not signed in to {vendor}"),
                    &format!("{vendor} didn't complete the sign-in: {failure}. Close this tab and try again in Demi."),
                ),
            )
        }
    })
}

/// A page of one sentence under its title. A vendor's answer may hold any
/// character, so both are escaped.
fn page(title: &str, text: &str) -> Html<String> {
    let (title, text) = (xml_escaped(title), xml_escaped(text));
    Html(format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{title}</title><body style=\"font: 15px/1.5 -apple-system, system-ui, sans-serif; margin: 4em auto; max-width: 32em; padding: 0 1em\"><p>{text}</p></body></html>"
    ))
}
