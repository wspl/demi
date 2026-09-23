//! The vendors from models.dev (`providers.md` § Vendors from models.dev):
//! the vendors whose client package maps to a protocol one of the families
//! speaks, each vendor's live model list, and the request requirements the
//! backend applies to a vendor's models.

use demi_core::{ProviderModelList, Timestamp};
use demi_provider::models_dev::{ModelsDevClient, ModelsDevError, ModelsDevVendor};
use demi_provider_anthropic_api::AnthropicConfig;
use demi_provider_openai_api::{OpenAiConfig, VendorPolicy};
use demi_web_api::providers::{Vendor, WireApi};
use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};

/// The family and wire each models.dev client package is written for. A
/// vendor of another package is not offered.
const FAMILIES: [(&str, &str, Option<WireApi>); 4] = [
    ("@ai-sdk/openai-compatible", "openai", Some(WireApi::ChatCompletions)),
    ("@ai-sdk/openai", "openai", Some(WireApi::Responses)),
    ("@ai-sdk/anthropic", "anthropic", None),
    ("@ai-sdk/google", "google", None),
];

/// Vendors whose endpoint needs an authentication scheme of its own, which
/// an API key cannot satisfy.
const NOT_OFFERED: [&str; 1] = ["github-copilot"];

/// The request requirements of the vendors that have any.
fn policy_of(vendor_id: &str) -> VendorPolicy {
    match vendor_id {
        "deepseek" => VendorPolicy {
            pass_back_reasoning_content: true,
            ..VendorPolicy::default()
        },
        _ => VendorPolicy::default(),
    }
}

/// The vendor list over the backend's models.dev copy.
#[derive(Clone)]
pub(crate) struct VendorCatalog {
    models_dev: ModelsDevClient,
}

impl VendorCatalog {
    pub(crate) fn new(models_dev: ModelsDevClient) -> Self {
        Self { models_dev }
    }

    pub(crate) fn models_dev(&self) -> &ModelsDevClient {
        &self.models_dev
    }

    /// The request requirements of the models of vendor `vendor_id`.
    pub(crate) fn policy(&self, vendor_id: Option<&str>) -> VendorPolicy {
        vendor_id.map(policy_of).unwrap_or_default()
    }

    /// The vendors an entry can be added from, by name as the browser sorts
    /// names, from a copy less than a day old.
    pub(crate) async fn vendors(&self) -> Result<Vec<Vendor>, ModelsDevError> {
        let snapshot = self.models_dev.current().await?;
        let mut vendors: Vec<Vendor> = snapshot.vendors().filter_map(offered).collect();
        let collator = root_collator();
        vendors.sort_by(|left, right| collator.compare(&left.name, &right.name));
        Ok(vendors)
    }

    /// The vendor `vendor_id`, when it is offered.
    pub(crate) async fn vendor(&self, vendor_id: &str) -> Result<Option<Vendor>, ModelsDevError> {
        let snapshot = self.models_dev.current().await?;
        Ok(snapshot.vendor(vendor_id).and_then(offered))
    }

    /// The live model list of vendor `vendor_id`, read again: a vendor the
    /// document no longer offers has an empty list, never its family's.
    pub(crate) async fn models(&self, vendor_id: &str) -> Result<ProviderModelList, String> {
        let snapshot = self.models_dev.refreshed().await.map_err(|error| error.to_string())?;
        let list = snapshot
            .vendor(vendor_id)
            .and_then(offered)
            .and_then(|_| snapshot.vendor_models(vendor_id));
        Ok(list.unwrap_or_else(|| ProviderModelList {
            models: Vec::new(),
            default_model_id: None,
            warnings: snapshot.warnings.clone(),
            source_fetched_at: Timestamp::UNIX_EPOCH,
            stale: snapshot.stale,
        }))
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
        _ => None,
    }
}

/// The root locale's collation, which orders names as the browser's
/// `localeCompare` does.
fn root_collator() -> CollatorBorrowed<'static> {
    // The root locale's data is compiled in.
    Collator::try_new(Default::default(), CollatorOptions::default()).expect("the root collation is compiled in")
}
