//! The models Anthropic lists for a key: `GET …/models`, every page, with
//! the key and the API's version as a run sends them
//! ([List Models](https://docs.anthropic.com/en/api/models-list)).

use demi_provider_common::{ProviderHttp, refusal_words};
use reqwest::Url;
use reqwest::header::HeaderValue;
use serde::Deserialize;

use crate::stream::ANTHROPIC_VERSION;

/// The most models one page holds, the most Anthropic takes.
const PAGE_SIZE: &str = "1000";

#[derive(Deserialize)]
struct Page {
    data: Vec<Listed>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    last_id: Option<String>,
}

#[derive(Deserialize)]
struct Listed {
    id: String,
}

/// The ids of every model `url` lists for `key`.
pub(crate) async fn list(http: &ProviderHttp, url: &Url, key: Option<&HeaderValue>) -> Result<Vec<String>, String> {
    let failed = |cause: String| format!("Anthropic's model list could not be read: {cause}");
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut page = url.clone();
        {
            let mut query = page.query_pairs_mut();
            query.append_pair("limit", PAGE_SIZE);
            if let Some(after) = &after {
                query.append_pair("after_id", after);
            }
        }
        let mut send = http.get(page).header("anthropic-version", ANTHROPIC_VERSION);
        if let Some(key) = key {
            send = send.header("x-api-key", key.clone());
        }
        let response = send.send().await.map_err(|error| failed(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(failed(refusal_words(response, "Anthropic").await));
        }
        let body = response.bytes().await.map_err(|error| failed(error.without_url().to_string()))?;
        let listed: Page = serde_json::from_slice(&body).map_err(|error| failed(error.to_string()))?;
        ids.extend(listed.data.into_iter().map(|model| model.id));
        match listed.last_id.filter(|_| listed.has_more) {
            Some(last) => after = Some(last),
            None => return Ok(ids),
        }
    }
}
