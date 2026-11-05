//! The models Anthropic lists for a key: `GET …/models`, every page, with
//! the key and the API's version as a run sends them
//! ([List Models](https://docs.anthropic.com/en/api/models-list)), each
//! with the thinking its `capabilities` state it takes.

use demi_provider_common::{OfferedModel, ProviderHttp, refusal_words};
use demi_shared_types::ThinkingForm;
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
    #[serde(default)]
    capabilities: Option<Capabilities>,
}

#[derive(Deserialize)]
struct Capabilities {
    #[serde(default)]
    thinking: Option<ThinkingCapability>,
}

#[derive(Deserialize)]
struct ThinkingCapability {
    #[serde(default)]
    types: Option<ThinkingTypes>,
}

#[derive(Deserialize)]
struct ThinkingTypes {
    #[serde(default)]
    adaptive: Option<Support>,
    #[serde(default)]
    enabled: Option<Support>,
}

#[derive(Deserialize)]
struct Support {
    supported: bool,
}

impl Listed {
    /// How the model takes a reasoning level: adaptive thinking at an
    /// effort where it takes that, else a budget where it takes one; none
    /// when the list states neither.
    fn offered(self) -> OfferedModel {
        let types = self.capabilities.and_then(|capabilities| capabilities.thinking?.types);
        let takes = |support: Option<&Support>| support.is_some_and(|support| support.supported);
        let thinking_form = types.and_then(|types| {
            if takes(types.adaptive.as_ref()) {
                Some(ThinkingForm::Effort)
            } else if takes(types.enabled.as_ref()) {
                Some(ThinkingForm::Budget)
            } else {
                None
            }
        });
        OfferedModel { id: self.id, thinking_form }
    }
}

/// Every model `url` lists for `key`.
pub(crate) async fn list(http: &ProviderHttp, url: &Url, key: Option<&HeaderValue>) -> Result<Vec<OfferedModel>, String> {
    let failed = |cause: String| format!("Anthropic's model list could not be read: {cause}");
    let mut models = Vec::new();
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
        models.extend(listed.data.into_iter().map(Listed::offered));
        match listed.last_id.filter(|_| listed.has_more) {
            Some(last) => after = Some(last),
            None => return Ok(models),
        }
    }
}
