//! The Grok Build catalog (`models.md` § Directories): `GET /v1/models` on
//! the chat proxy with the account's session. A catalog that cannot be read
//! is a failed refresh, and the backend keeps its last one.

use demi_core::{Clock, ProviderModel, ProviderModelList};
use demi_provider::{
    CatalogError,
    wire::{NonEmpty, decode_untagged},
};
use http::{HeaderValue, header::ACCEPT};
use serde::Deserialize;

use crate::{Shared, request::identity_headers};

/// A fresh read of the proxy's model list.
pub(crate) async fn list(shared: &Shared) -> Result<ProviderModelList, CatalogError> {
    let credentials = shared
        .auth
        .credentials(&shared.http, None)
        .await
        .map_err(|failure| failure.catalog_error())?;
    let mut headers = identity_headers(&credentials);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    let response = shared
        .http
        .get(shared.models_url.clone())
        .headers(headers)
        .send()
        .await
        .map_err(|error| {
            CatalogError::Unavailable(format!(
                "Grok Build models request failed: {}",
                error.without_url()
            ))
        })?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(CatalogError::Unavailable(format!(
            "Grok Build models request failed with HTTP {status}"
        )));
    }
    let text = response.text().await.map_err(|error| {
        CatalogError::Unavailable(format!(
            "Grok Build models request failed: {}",
            error.without_url()
        ))
    })?;
    catalog(&text, &*shared.clock)
}

/// The catalog of a models answer: an OpenAI-style `data` envelope or the
/// bare list. A model names itself by `id` or `model`; one that names
/// neither is skipped. A catalog without models is refused.
fn catalog(text: &str, clock: &dyn Clock) -> Result<ProviderModelList, CatalogError> {
    let invalid = |error: String| {
        CatalogError::Invalid(format!("Grok Build models answer cannot be read: {error}"))
    };
    let value: serde_json::Value =
        decode_untagged(text).map_err(|error| invalid(error.to_string()))?;
    let listed: Vec<GrokModel> = if value.is_array() {
        decode_untagged(text).map_err(|error| invalid(error.to_string()))?
    } else {
        let envelope: Envelope =
            decode_untagged(text).map_err(|error| invalid(error.to_string()))?;
        envelope.data.unwrap_or_default()
    };
    let models: Vec<ProviderModel> = listed
        .into_iter()
        .filter_map(GrokModel::into_model)
        .collect();
    if models.is_empty() {
        return Err(CatalogError::Invalid(
            "Grok Build models answer lists no model".into(),
        ));
    }
    Ok(ProviderModelList {
        default_model_id: models.first().map(|model| model.id.clone()),
        models,
        warnings: Vec::new(),
        source_fetched_at: clock.now(),
        stale: false,
    })
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    data: Option<Vec<GrokModel>>,
}

/// One model as the proxy describes it; only the fields Demi reads. A
/// context window is a count of tokens.
#[derive(Deserialize)]
struct GrokModel {
    #[serde(default)]
    id: Option<NonEmpty>,
    #[serde(default)]
    model: Option<NonEmpty>,
    #[serde(default)]
    name: Option<NonEmpty>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    context_window: Option<std::num::NonZeroU32>,
    #[serde(default)]
    supports_reasoning_effort: Option<bool>,
    #[serde(default)]
    reasoning_effort: Option<NonEmpty>,
    #[serde(default)]
    reasoning_efforts: Option<Vec<ReasoningEffort>>,
}

/// An effort a model offers, named by `id` or, on older deployments, by
/// `value`.
#[derive(Deserialize)]
struct ReasoningEffort {
    #[serde(default)]
    id: Option<NonEmpty>,
    #[serde(default)]
    value: Option<NonEmpty>,
    #[serde(default)]
    default: Option<bool>,
}

impl GrokModel {
    fn into_model(self) -> Option<ProviderModel> {
        let id = self.id.or(self.model)?.0;
        let mut efforts = Vec::new();
        let mut flagged = None;
        for effort in self.reasoning_efforts.into_iter().flatten() {
            let Some(name) = effort.id.or(effort.value) else {
                continue;
            };
            if effort.default == Some(true) && flagged.is_none() {
                flagged = Some(name.0.clone());
            }
            efforts.push(name.0);
        }
        // The flagged effort starts, else the model's own, else the first.
        let default_effort = flagged
            .or(self.reasoning_effort.map(|effort| effort.0))
            .or_else(|| efforts.first().cloned());
        let reasoning = self.supports_reasoning_effort == Some(true) || !efforts.is_empty();
        Some(ProviderModel {
            display_name: self.name.map_or_else(|| id.clone(), |name| name.0),
            id,
            description: self.description,
            context_window: self.context_window.map(std::num::NonZeroU32::get),
            output_limit: None,
            supports_tools: Some(true),
            // The proxy states no modalities; Grok Build's own harness sends
            // images natively.
            supports_attachments: Some(true),
            supports_video: None,
            accepted_extensions: None,
            supports_reasoning: reasoning.then_some(true),
            supported_thinking_efforts: (!efforts.is_empty()).then_some(efforts),
            default_thinking_effort: default_effort,
            can_disable_thinking: None,
            service_tiers: Vec::new(),
            default_service_tier_id: None,
            cost: None,
        })
    }
}
