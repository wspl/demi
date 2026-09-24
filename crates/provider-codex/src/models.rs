//! The Codex catalog (`models.md` § The Codex catalog): the account's model
//! list on the ChatGPT backend, read with the version of the Codex CLI
//! release the provider was last checked against, because the backend picks
//! the models it lists by that version.

use std::num::NonZeroU32;

use demi_core::{Clock, ProviderModel, ProviderModelList, ServiceTier};
use demi_provider::{
    CatalogError,
    wire::{NonEmpty, decode_untagged},
};
use http::{HeaderValue, StatusCode, header::ACCEPT};
use reqwest::Url;
use serde::{Deserialize, Deserializer};

use crate::{Shared, request::account_headers};

/// The Codex CLI release the provider was last checked against.
pub(crate) const CLIENT_VERSION: &str = "0.153.4";

/// Codex offers its Fast mode as the `priority` service tier.
const FAST_TIER: &str = "priority";

/// A fresh read of the account's model list. A request refused with HTTP 401
/// refreshes the token that was refused, once, and reads again.
pub(crate) async fn list(shared: &Shared) -> Result<ProviderModelList, CatalogError> {
    let mut refused_token = None;
    loop {
        let credentials = shared
            .auth
            .credentials(&shared.http, refused_token.as_ref())
            .await
            .map_err(|failure| failure.catalog_error())?;
        let mut headers = account_headers(&credentials, &shared.user_agent);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let response = shared
            .http
            .get(shared.models_url.clone())
            .headers(headers)
            .send()
            .await
            .map_err(|error| {
                CatalogError::Unavailable(format!(
                    "Codex models request failed: {}",
                    error.without_url()
                ))
            })?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED && refused_token.is_none() {
            refused_token = Some(credentials.access_token);
            continue;
        }
        if !status.is_success() {
            return Err(CatalogError::Unavailable(format!(
                "Codex models request failed with HTTP {}",
                status.as_u16()
            )));
        }
        let text = response.text().await.map_err(|error| {
            CatalogError::Unavailable(format!(
                "Codex models request failed: {}",
                error.without_url()
            ))
        })?;
        return catalog(&text, &*shared.clock);
    }
}

/// The catalog a models answer describes: the models the CLI's picker
/// lists, by ascending priority, the first the default. A malformed answer
/// is refused, never read around.
fn catalog(text: &str, clock: &dyn Clock) -> Result<ProviderModelList, CatalogError> {
    let answer: ModelsAnswer = decode_untagged(text).map_err(|error| {
        CatalogError::Invalid(format!("Codex models answer cannot be read: {error}"))
    })?;
    let mut listed: Vec<CodexModel> = answer
        .models
        .into_iter()
        .filter(|model| model.visibility == Visibility::List)
        .collect();
    listed.sort_by_key(|model| model.priority);
    let models: Vec<ProviderModel> = listed.into_iter().map(CodexModel::into_model).collect();
    Ok(ProviderModelList {
        default_model_id: models.first().map(|model| model.id.clone()),
        models,
        warnings: Vec::new(),
        source_fetched_at: clock.now(),
        stale: false,
    })
}

#[derive(Deserialize)]
struct ModelsAnswer {
    models: Vec<CodexModel>,
}

/// One model as the backend describes it; only the fields Demi reads.
#[derive(Deserialize)]
struct CodexModel {
    slug: NonEmpty,
    display_name: NonEmpty,
    #[serde(default)]
    description: Option<String>,
    visibility: Visibility,
    priority: i64,
    #[serde(default)]
    context_window: Option<NonZeroU32>,
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
    supported_reasoning_levels: Vec<ReasoningLevel>,
    #[serde(default)]
    default_reasoning_level: Option<NonEmpty>,
    #[serde(default)]
    service_tiers: Option<Vec<CodexTier>>,
    #[serde(default)]
    default_service_tier: Option<NonEmpty>,
    #[serde(default)]
    tool_mode: Option<String>,
    #[serde(default)]
    experimental_supported_tools: Option<Vec<String>>,
    /// Present, even as `null`, when the model has an apply-patch tool.
    #[serde(default, deserialize_with = "present")]
    apply_patch_tool_type: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    web_search_tool_type: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Visibility {
    List,
    Hide,
    None,
}

#[derive(Deserialize)]
struct ReasoningLevel {
    effort: NonEmpty,
}

#[derive(Deserialize)]
struct CodexTier {
    id: NonEmpty,
    name: NonEmpty,
    #[serde(default)]
    description: Option<String>,
}

impl CodexModel {
    fn into_model(self) -> ProviderModel {
        let supports_tools = self.supports_tools();
        let efforts: Vec<String> = self
            .supported_reasoning_levels
            .into_iter()
            .map(|level| level.effort.0)
            .collect();
        ProviderModel {
            id: self.slug.0,
            display_name: self.display_name.0,
            description: self.description,
            context_window: self.context_window.map(NonZeroU32::get),
            output_limit: None,
            supports_tools,
            supports_attachments: self
                .input_modalities
                .map(|modalities| modalities.iter().any(|modality| modality == "image")),
            supports_video: None,
            accepted_extensions: None,
            supports_reasoning: Some(!efforts.is_empty()),
            supported_thinking_efforts: Some(efforts),
            default_thinking_effort: self.default_reasoning_level.map(|level| level.0),
            // Leaving reasoning out gets Codex's default, not no reasoning.
            can_disable_thinking: Some(false),
            service_tiers: self
                .service_tiers
                .into_iter()
                .flatten()
                .map(|tier| ServiceTier {
                    fast: tier.id.0 == FAST_TIER,
                    id: tier.id.0,
                    label: tier.name.0,
                    description: tier
                        .description
                        .filter(|description| !description.is_empty()),
                })
                .collect(),
            default_service_tier_id: self.default_service_tier.map(|tier| tier.0),
            cost: None,
        }
    }

    /// Whether the model calls tools, by the tool fields the backend names;
    /// unknown when it names none.
    fn supports_tools(&self) -> Option<bool> {
        if self
            .tool_mode
            .as_deref()
            .is_some_and(|mode| !mode.is_empty())
        {
            return Some(true);
        }
        if let Some(tools) = &self.experimental_supported_tools {
            return Some(!tools.is_empty());
        }
        if self.apply_patch_tool_type.is_some() || self.web_search_tool_type.is_some() {
            return Some(true);
        }
        None
    }
}

/// A field that is present, `null` included.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

/// The catalog request's URL: the models under the backend, with the client
/// version the backend picks models by.
pub(crate) fn models_url(backend: &Url) -> Url {
    let mut url = crate::codex_url(backend, "/models");
    url.query_pairs_mut()
        .append_pair("client_version", CLIENT_VERSION);
    url
}
