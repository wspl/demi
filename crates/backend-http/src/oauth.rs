//! A plugin's OAuth callback (`plugins.md` § OAuth sign-in): where the
//! vendor sends the browser back after the user signed in. It is a session
//! route: the browser's own navigation carries the session cookie, so the
//! caller's shard completes only a sign-in its user started. It answers a
//! page the user reads in the tab the sign-in opened.

use axum::extract::{Path, Query, State};
use axum::http::header::ACCEPT_LANGUAGE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use demi_backend_plugins::{Callback, CallbackError};
use demi_shared_types::xml_escaped;
use demi_web_api_protocol::settings::Language;
use rust_i18n::t;

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
    headers: HeaderMap,
) -> Result<(StatusCode, Html<String>), ApiError> {
    let completed = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard.plugins().oauth_callback(&plugin, callback, cancel).await
        })
        .await?;
    let language = page_language(&headers);
    let locale = language.to_string();
    let locale = locale.as_str();
    Ok(match completed {
        Ok(vendor) => (
            StatusCode::OK,
            page(
                language,
                &t!("oauth.signed_in.title", locale = locale, vendor = vendor),
                &t!("oauth.signed_in.text", locale = locale, vendor = vendor),
            ),
        ),
        Err(CallbackError::UnknownPlugin(_)) => (
            StatusCode::NOT_FOUND,
            page(
                language,
                &t!("oauth.unknown_plugin.title", locale = locale),
                &t!("oauth.unknown_plugin.text", locale = locale),
            ),
        ),
        Err(CallbackError::UnknownState) => (
            StatusCode::BAD_REQUEST,
            page(
                language,
                &t!("oauth.unknown_state.title", locale = locale),
                &t!("oauth.unknown_state.text", locale = locale),
            ),
        ),
        Err(CallbackError::Denied { vendor, error }) => (
            StatusCode::BAD_REQUEST,
            page(
                language,
                &t!("oauth.denied.title", locale = locale, vendor = vendor),
                &t!("oauth.denied.text", locale = locale, vendor = vendor, error = error),
            ),
        ),
        Err(CallbackError::Storage(error)) => return Err(error.into()),
        Err(CallbackError::Exchange { vendor, failure }) => {
            tracing::warn!(vendor, %failure, "a plugin's sign-in did not complete");
            (
                StatusCode::BAD_GATEWAY,
                page(
                    language,
                    &t!("oauth.exchange_failed.title", locale = locale, vendor = vendor),
                    &t!(
                        "oauth.exchange_failed.text",
                        locale = locale,
                        vendor = vendor,
                        failure = failure
                    ),
                ),
            )
        }
    })
}

/// The language of a page the backend renders: the first of the
/// languages the request's `Accept-Language` prefers that Demi speaks
/// (`localization.md` § Text from the backend). A language the header
/// rules out with `q=0` is not one it prefers.
fn page_language(headers: &HeaderMap) -> Language {
    let accepted = headers
        .get_all(ACCEPT_LANGUAGE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','));
    let mut ranked: Vec<(&str, f32)> = accepted
        .filter_map(|entry| {
            let mut parts = entry.split(';').map(str::trim);
            let tag = parts.next().filter(|tag| !tag.is_empty())?;
            let weight = parts
                .find_map(|part| part.strip_prefix("q="))
                .map_or(Some(1.0), |q| q.parse::<f32>().ok())?;
            (weight > 0.0).then_some((tag, weight))
        })
        .collect();
    // Stable: languages of one weight keep the header's order.
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    Language::first_spoken(ranked.into_iter().map(|(tag, _)| tag))
}

/// A page of one sentence under its title, in `language`. A vendor's answer
/// may hold any character, so both are escaped.
fn page(language: Language, title: &str, text: &str) -> Html<String> {
    let (title, text) = (xml_escaped(title), xml_escaped(text));
    Html(format!(
        "<!doctype html><html lang=\"{language}\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{title}</title><body style=\"font: 15px/1.5 -apple-system, system-ui, sans-serif; margin: 4em auto; max-width: 32em; padding: 0 1em\"><p>{text}</p></body></html>"
    ))
}
