//! The vendors from models.dev.
//!
//! The module holds:
//!
//! - The vendors whose client package maps to a protocol one of the families
//!   speaks.
//! - Each vendor's live model list, and how each of its models wants thinking
//!   sent and sent back, as models.dev states it per model.

use demi_provider_anthropic_api::AnthropicConfig;
use demi_provider_common::ModelWires;
use demi_provider_common::models_dev::{
    ModelsDevClient, ModelsDevError, ModelsDevMatch, ModelsDevVendor,
};
use demi_provider_openai_api::OpenAiConfig;
use demi_shared_types::{ProviderModelList, Timestamp, WireApi};
use demi_web_api_protocol::providers::{ModelMatch, Vendor};
use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};

/// The family and wire each models.dev client package is written for. A
/// vendor of another package is not offered.
const FAMILIES: [(&str, &str, Option<WireApi>); 5] = [
    (
        "@ai-sdk/openai-compatible",
        "openai",
        Some(WireApi::ChatCompletions),
    ),
    ("@ai-sdk/openai", "openai", Some(WireApi::Responses)),
    ("@ai-sdk/anthropic", "anthropic", None),
    ("@ai-sdk/google", "google", None),
    ("@ai-sdk/xai", "openai", Some(WireApi::Responses)),
];

/// xAI's API, which models.dev does not name.
const XAI_BASE_URL: &str = "https://api.x.ai/v1";

/// Vendors whose endpoint needs an authentication scheme of its own, which
/// an API key cannot satisfy.
const NOT_OFFERED: [&str; 1] = ["github-copilot"];

/// The vendor list over the backend's models.dev copy.
#[derive(Clone)]
pub struct VendorCatalog {
    models_dev: ModelsDevClient,
}

impl VendorCatalog {
    pub fn new(models_dev: ModelsDevClient) -> Self {
        Self { models_dev }
    }

    pub fn models_dev(&self) -> &ModelsDevClient {
        &self.models_dev
    }

    /// How each model of vendor `vendor_id` wants thinking sent and sent
    /// back, from the backend's kept copy; none without one, so each model
    /// takes the defaults.
    pub async fn wires(&self, vendor_id: &str) -> ModelWires {
        match self.models_dev.kept().await {
            Ok(snapshot) => ModelWires::new(snapshot.wires(vendor_id)),
            Err(error) => {
                tracing::warn!(
                    event = "vendor_wires.unread",
                    vendor = vendor_id,
                    error = %error,
                    "a vendor's models were not read for how they take thinking; each takes the defaults"
                );
                ModelWires::default()
            }
        }
    }

    /// The vendors an entry can be added from, by name as the web app sorts
    /// names, from a copy less than a day old.
    pub async fn vendors(&self) -> Result<Vec<Vendor>, ModelsDevError> {
        let snapshot = self.models_dev.current().await?;
        let mut vendors: Vec<Vendor> = snapshot.vendors().filter_map(offered).collect();
        sort_by_name(&mut vendors);
        Ok(vendors)
    }

    /// The vendor `vendor_id`, when it is offered.
    pub async fn vendor(&self, vendor_id: &str) -> Result<Option<Vendor>, ModelsDevError> {
        let snapshot = self.models_dev.current().await?;
        Ok(snapshot.vendor(vendor_id).and_then(offered))
    }

    /// The models.dev models `query` matches, filled from vendor `vendor_id` where it lists them,
    /// from the backend's kept copy: only a backend without one asks
    /// models.dev.
    pub async fn search(
        &self,
        query: &str,
        vendor_id: Option<&str>,
    ) -> Result<Vec<ModelMatch>, ModelsDevError> {
        let snapshot = self.models_dev.kept().await?;
        Ok(snapshot
            .search(query, vendor_id)
            .into_iter()
            .map(filled)
            .collect())
    }

    /// The model `reference`, `<vendor>/<model id>`, as a search answers
    /// it; none when that vendor lists no such model.
    pub async fn model(&self, reference: &str) -> Result<Option<ModelMatch>, ModelsDevError> {
        let Some((vendor, id)) = reference.split_once('/') else {
            return Ok(None);
        };
        let snapshot = self.models_dev.kept().await?;
        Ok(snapshot.listing(vendor, id).map(filled))
    }

    /// The live model list of vendor `vendor_id`, read again: the models an
    /// agent can work with; a vendor the document no longer offers has an
    /// empty list, never its family's.
    pub async fn models(&self, vendor_id: &str) -> Result<ProviderModelList, String> {
        let snapshot = self
            .models_dev
            .refreshed()
            .await
            .map_err(|error| error.to_string())?;
        let list = snapshot
            .vendor(vendor_id)
            .and_then(offered)
            .and_then(|_| snapshot.agent_models(vendor_id));
        Ok(list.unwrap_or_else(|| ProviderModelList {
            models: Vec::new(),
            default_model_id: None,
            warnings: snapshot.warnings.clone(),
            source_fetched_at: Timestamp::UNIX_EPOCH,
            stale: snapshot.stale,
        }))
    }
}

/// A models.dev model filled as a configured model would be: a fact the
/// document does not state is left empty.
fn filled(found: ModelsDevMatch) -> ModelMatch {
    ModelMatch {
        reference: format!("{}/{}", found.vendor, found.id),
        display_name: found.model.display_name,
        id: found.id,
        vendor: found.vendor,
        context_window: found.model.context_window,
        output_limit: found.model.output_limit,
        thinking_efforts: found.model.supported_thinking_efforts.unwrap_or_default(),
        accepted_extensions: found.model.accepted_extensions,
        thinking_form: found.model.thinking_form,
        interleaved_thinking: found.model.interleaved_thinking,
        release_date: found.release_date,
    }
}

/// The vendor as the page offers it, or `None` for one no family speaks.
fn offered(vendor: &ModelsDevVendor) -> Option<Vendor> {
    if NOT_OFFERED.contains(&vendor.id.as_str()) {
        return None;
    }
    let npm = vendor.npm.as_deref()?;
    let (_, family, wire_api) = FAMILIES.iter().find(|(package, _, _)| *package == npm)?;
    let base_url = vendor.api.clone().or_else(|| official_base_url(&vendor.id));
    Some(Vendor {
        id: vendor.id.clone(),
        name: vendor.name.clone(),
        provider_type: (*family).to_owned(),
        wire_api: *wire_api,
        base_url,
        doc: vendor.doc.clone(),
    })
}

/// The endpoint of a first-party vendor that models.dev lists without one,
/// because its own clients know it.
fn official_base_url(vendor_id: &str) -> Option<String> {
    match vendor_id {
        "anthropic" => Some(AnthropicConfig::DEFAULT_BASE_URL.to_owned()),
        "openai" => Some(OpenAiConfig::DEFAULT_BASE_URL.to_owned()),
        "xai" => Some(XAI_BASE_URL.to_owned()),
        _ => None,
    }
}

/// `vendors` by name, as the web app sorts names.
pub fn sort_by_name(vendors: &mut [Vendor]) {
    let collator = root_collator();
    vendors.sort_by(|left, right| collator.compare(&left.name, &right.name));
}

/// The root locale's collation, which orders names as a web browser's
/// `localeCompare` does.
fn root_collator() -> CollatorBorrowed<'static> {
    // The root locale's data is compiled in.
    Collator::try_new(Default::default(), CollatorOptions::default())
        .expect("the root collation is compiled in")
}
