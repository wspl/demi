//! Exa's REST search with the user's API key (`web.md` § Accounts), which
//! offers every option.

use std::time::Instant;

use demi_plugin_interface::{PluginPort, WebMethod, WebRequest};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::search::{Failure, Kind, Query, SearchError, SearchedPage, page, remaining, status_failure};

const ENDPOINT: &str = "https://api.exa.ai/search";
/// The most bytes of an answer: a search's pages and their lines.
const LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Deserialize)]
struct Answer {
    results: Vec<Found>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Found {
    #[serde(default)]
    title: Option<String>,
    url: String,
    #[serde(default)]
    published_date: Option<String>,
    #[serde(default)]
    highlights: Vec<String>,
}

/// The pages Exa finds for `query` with `key`.
pub(crate) async fn search(
    port: &PluginPort,
    key: &str,
    query: &Query<'_>,
    deadline: Instant,
) -> Result<Vec<SearchedPage>, SearchError> {
    let mut body = json!({
        "query": query.text,
        "numResults": query.count,
        "contents": { "highlights": true },
    });
    for (_, name, value) in query.filters.given() {
        body[name] = value;
    }
    let answer: Answer = send(port, key, &body, deadline).await?;
    Ok(answer
        .results
        .into_iter()
        .map(|found| {
            page(
                found.title.as_deref().unwrap_or_default(),
                &found.url,
                found.published_date.as_deref(),
                found.highlights,
            )
        })
        .collect())
}

/// Checks `key` with one search of one page: [`Failure::KeyRefused`] when
/// Exa refuses it.
pub(crate) async fn check(port: &PluginPort, key: &str) -> Result<(), SearchError> {
    let deadline = Instant::now() + crate::search::DEADLINE;
    let body = json!({ "query": "Exa", "numResults": 1 });
    send::<Value>(port, key, &body, deadline).await.map(drop)
}

async fn send<T: serde::de::DeserializeOwned>(
    port: &PluginPort,
    key: &str,
    body: &Value,
    deadline: Instant,
) -> Result<T, SearchError> {
    let request = WebRequest::new(
        WebMethod::Post,
        ENDPOINT.parse().expect("a valid URL"),
        remaining(deadline),
        LIMIT,
    )
    .header("Content-Type", "application/json")
    .header("Accept", "application/json")
    .header("x-api-key", key)
    .body(serde_json::to_vec(body).expect("a request encodes as JSON"));
    let response = port.send(request).await?;
    if !(200..300).contains(&response.status) {
        return Err(status_failure(&response, Kind::Key));
    }
    serde_json::from_slice(response.body.as_bytes()).map_err(|error| {
        Failure::Vendor(format!("an answer that could not be read: {error}")).into()
    })
}
