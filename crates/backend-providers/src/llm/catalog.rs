//! Where an entry's catalog comes from (`models.md` § Catalog sources): its
//! configured model list, read from the entry and never cached; its vendor's
//! list in models.dev; or its provider's directory, both through the catalog
//! cache. A compatible entry has no source but its configured list: empty
//! until the user adds a model. And the account-wide catalog `GET /api/models` answers, each model
//! with the selection the backend builds from it.

use std::sync::Arc;

use demi_provider_common::Provider;
use demi_shared_types::{ModelCost, ModelSelection, ProviderModel, ProviderModelList, ServiceTier, Timestamp};
use demi_web_api_protocol::providers::{CatalogModel, CatalogProvider, ConfiguredModel, ModelKind};
use futures_util::future::join_all;
use sha2::{Digest, Sha256};

use super::assembly::{AssemblyError, ProviderAssembly};
use super::catalog_cache::CatalogFetch;
use super::families::api_vendor;
use crate::vault::entries::{EntryCredential, ProviderEntry};

/// A model the entry's configured list does not name; its request fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the model {0} is not in the provider's configured list")]
pub struct NotConfigured(pub String);

/// A configured model as a catalog model: it states its facts directly, its
/// Fast tier is its only tier, and it is taken to call tools.
pub fn configured_model(model: &ConfiguredModel) -> ProviderModel {
    let efforts = &model.thinking_efforts;
    ProviderModel {
        id: model.id.as_str().to_owned(),
        display_name: model.display_name.as_str().to_owned(),
        description: None,
        context_window: model.context_window,
        output_limit: model.output_limit,
        supports_tools: Some(true),
        accepted_extensions: model.accepted_extensions.clone(),
        supports_reasoning: Some(!efforts.is_empty()),
        supported_thinking_efforts: Some(efforts.clone()),
        can_disable_thinking: None,
        service_tiers: model
            .fast_tier
            .iter()
            .map(|tier| ServiceTier {
                id: tier.clone(),
                label: "Fast".into(),
                description: None,
                fast: true,
            })
            .collect(),
        default_service_tier_id: None,
        cost: None,
    }
}

/// `selection` with the facts of the entry's configured model applied again,
/// as every inference boundary applies them (`models.md` § Request
/// parameters): an edit of a model's limits reaches the next request, and
/// the thinking and tier stay as the user chose them. An entry without a
/// configured list keeps the selection as it is.
pub fn configured_selection(
    entry: &ProviderEntry,
    selection: ModelSelection,
) -> Result<ModelSelection, NotConfigured> {
    let EntryCredential::ApiKey(config) = &entry.credential else {
        return Ok(selection);
    };
    let Some(models) = &config.models else {
        return Ok(selection);
    };
    let Some(model) = models
        .0
        .iter()
        .find(|model| model.kind == ModelKind::Text && model.id.as_str() == selection.model.id)
    else {
        return Err(NotConfigured(selection.model.id));
    };
    Ok(configured_model(model).selection(
        entry.id.as_str(),
        selection.thinking,
        selection.service_tier_id,
    ))
}

/// A list never fetched: a configured list, or an empty catalog.
fn unfetched(models: Vec<ProviderModel>, warnings: Vec<String>) -> ProviderModelList {
    ProviderModelList {
        models,
        default_model_id: None,
        warnings,
        source_fetched_at: Timestamp::UNIX_EPOCH,
        stale: false,
    }
}

impl ProviderAssembly {
    /// The entry's catalog, from the first source it has. A failed read of
    /// the source is the catalog's warning: the last good copy stays, and
    /// without one the catalog is empty. `provider` is the entry's provider,
    /// or why it could not be built.
    pub async fn entry_catalog(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, AssemblyError>,
        refresh: bool,
    ) -> ProviderModelList {
        // An entry of media alone has no text models to list.
        if !self.makes_text(entry) {
            return unfetched(Vec::new(), Vec::new());
        }
        let vendor = match &entry.credential {
            EntryCredential::ApiKey(config) => {
                // The model menu's are the text models; an image model of
                // the list is `demi image`'s (`providers.md` § Provider
                // management).
                if let Some(models) = &config.models {
                    let text = models.0.iter().filter(|model| model.kind == ModelKind::Text);
                    return unfetched(text.map(configured_model).collect(), Vec::new());
                }
                // A compatible endpoint decides its own models, so it never
                // borrows its family's directory or a vendor's list.
                if config.is_compatible() {
                    return unfetched(Vec::new(), Vec::new());
                }
                config.vendor_id.clone()
            }
            EntryCredential::Subscription { .. } => None,
        };
        let mut revision = None;
        let fetch: Box<dyn FnOnce() -> CatalogFetch + Send> = match (vendor, provider) {
            (Some(vendor), _) => {
                let vendors = self.vendors().clone();
                Box::new(move || Box::pin(async move { vendors.models(&vendor).await }))
            }
            (None, Ok(provider)) => {
                revision = provider.catalog_revision();
                let provider = provider.clone();
                Box::new(move || {
                    Box::pin(async move {
                        provider
                            .list_models()
                            .await
                            .map_err(|error| error.to_string())
                    })
                })
            }
            // An incomplete entry's availability says what it lacks.
            (None, Err(AssemblyError::Incomplete)) => return unfetched(Vec::new(), Vec::new()),
            (None, Err(error)) => return unfetched(Vec::new(), vec![error.to_string()]),
        };
        match self
            .catalogs()
            .read(&entry.id, catalog_key(entry), revision, fetch, refresh)
            .await
        {
            Ok(catalog) => catalog,
            Err(failure) => unfetched(Vec::new(), vec![failure]),
        }
    }

    /// The prices `model` of the entry is priced at as its calls' rows are
    /// written (`usage-and-quota.md` § How the figures are counted): its
    /// catalog's, and for a subscription, whose catalog has none, the same
    /// model's on the vendor's API, from the backend's models.dev copy,
    /// which asks models.dev at most once a day. None for a model the API
    /// does not sell. `provider` is the entry's, as for its catalog.
    pub async fn model_price(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, AssemblyError>,
        model: &str,
    ) -> Option<ModelCost> {
        let catalog = self.entry_catalog(entry, provider, false).await;
        let listed = catalog.models.into_iter().find(|listed| listed.id == model);
        if let Some(cost) = listed.and_then(|listed| listed.cost) {
            return Some(cost);
        }
        let vendor = api_vendor(&entry.family)?;
        // Without the document the calls are unpriced, as a model the API
        // does not sell; the next runtime asks again.
        let document = self.vendors().models_dev().current().await.ok()?;
        document.listing(vendor, model)?.model.cost
    }

    /// The catalog of every entry in `entries`, each with its provider's
    /// health: one entry's failure leaves the others intact.
    pub async fn model_catalog(
        &self,
        entries: &[ProviderEntry],
        refresh: bool,
    ) -> Vec<CatalogProvider> {
        join_all(
            entries
                .iter()
                .map(|entry| self.catalog_provider(entry, refresh)),
        )
        .await
    }

    async fn catalog_provider(&self, entry: &ProviderEntry, refresh: bool) -> CatalogProvider {
        let provider = self.provider_for(entry).await;
        let catalog = self.entry_catalog(entry, &provider, refresh).await;
        let (auth, runtime) = self.health(entry, &provider).await;
        let models = catalog
            .models
            .into_iter()
            .map(|model| CatalogModel {
                selection: model.selection(entry.id.as_str(), None, None),
                model,
            })
            .collect();
        CatalogProvider {
            provider_id: entry.id.clone(),
            display_name: entry.label.clone(),
            cli_package: provider
                .as_ref()
                .ok()
                .and_then(|provider| super::claude_releases::cli_package(provider.as_ref())),
            models,
            // The media service fills them, which knows the media catalog.
            image_models: Vec::new(),
            video_models: Vec::new(),
            audio_models: Vec::new(),
            source_fetched_at: catalog.source_fetched_at,
            stale: catalog.stale,
            warnings: catalog.warnings,
            availability: super::details::availability(entry, &auth, &runtime),
            auth,
            runtime,
        }
    }
}

/// The digest a catalog record is kept under: the SHA-256 of the RFC 8785
/// form of the entry's family, configuration and active account, so a
/// changed entry starts from an empty cache (`storage.md` § Encodings and
/// digests).
fn catalog_key(entry: &ProviderEntry) -> String {
    let config = match &entry.credential {
        EntryCredential::ApiKey(config) => serde_json::to_value(config),
        EntryCredential::Subscription { .. } => Ok(serde_json::Value::Null),
    };
    // A configuration is strings, numbers and lists, which always serialize.
    let config = config.expect("an entry's configuration serializes");
    let identity = serde_json::json!({
        "family": entry.family,
        "config": config,
        "account": entry.active().map(|account| account.as_str()),
    });
    let canonical =
        serde_json_canonicalizer::to_string(&identity).expect("a JSON value has a canonical form");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

#[cfg(test)]
mod tests {
    use demi_backend_database::control::{ControlService, testing};
    use demi_provider_common::Secret;
    use demi_provider_common::models_dev::ModelsDevClient;
    use demi_provider_common::testing::{MockResponse, MockVendor};
    use demi_shared_types::{Model, Rotation, SystemClock, ThinkingConfig};
    use demi_web_api_protocol::ids::{ProviderId, UserId};
    use demi_web_api_protocol::providers::ConfiguredModels;
    use demi_web_api_protocol::settings::InstanceMode;

    use super::*;
    use crate::llm::catalog_cache::ModelCatalogCache;
    use crate::llm::families::FamilyRegistry;
    use crate::llm::vendors::VendorCatalog;
    use crate::vault::entries::{ApiKeyConfig, Vault};
    use crate::vault::quotas::AccountQuotas;
    use crate::vault::seal::VaultKey;

    fn entry(models: Option<serde_json::Value>) -> ProviderEntry {
        let models =
            models.map(|models| serde_json::from_value::<ConfiguredModels>(models).unwrap());
        ProviderEntry {
            id: ProviderId::try_from("entry-1").unwrap(),
            owner: UserId::try_from("owner").unwrap(),
            family: "openai".into(),
            label: "Work".into(),
            credential: EntryCredential::ApiKey(ApiKeyConfig {
                api_key: Some(Secret::try_from("sk-1".to_owned()).unwrap()),
                base_url: None,
                wire_api: None,
                vendor_id: None,
                models,
                image_shape: None,
                video_shape: None,
                audio_shape: None,
            }),
            created_at: Timestamp::UNIX_EPOCH,
            image_generation: true,
            video_generation: true,
            audio_generation: true,
        }
    }

    fn model(output_limit: u32) -> serde_json::Value {
        serde_json::json!({
            "id": "gpt-5.5", "displayName": "GPT-5.5", "contextWindow": 272_000, "outputLimit": output_limit,
            "thinkingEfforts": ["low", "high"], "acceptedExtensions": ["png", "pdf"], "fastTier": "priority"
        })
    }

    /// The selection the web app sent with a conversation's first request,
    /// from an older catalog.
    fn chosen() -> ModelSelection {
        ModelSelection {
            provider_id: "entry-1".into(),
            model: Model {
                id: "gpt-5.5".into(),
                name: "stale".into(),
                context_window: 1000,
                output_limit: Some(100),
                thinking: Vec::new(),
                accepted_extensions: Some(Vec::new()),
            },
            thinking: Some(ThinkingConfig::Effort {
                effort: "high".into(),
                summary: None,
            }),
            service_tier_id: Some("priority".into()),
        }
    }

    #[test]
    fn every_request_takes_the_configured_facts_and_keeps_the_users_choices() {
        let applied =
            configured_selection(&entry(Some(serde_json::json!([model(4_000)]))), chosen())
                .unwrap();
        assert_eq!(
            (
                applied.model.name.as_str(),
                applied.model.context_window,
                applied.model.output_limit
            ),
            ("GPT-5.5", 272_000, Some(4_000))
        );
        assert_eq!(
            applied.model.accepted_extensions,
            Some(vec![
                demi_shared_types::FileExtension::Png,
                demi_shared_types::FileExtension::Pdf
            ])
        );
        assert_eq!(
            (applied.thinking, applied.service_tier_id),
            (chosen().thinking, chosen().service_tier_id)
        );
        // An edit of the list reaches the next request.
        let edited =
            configured_selection(&entry(Some(serde_json::json!([model(8_000)]))), chosen())
                .unwrap();
        assert_eq!(edited.model.output_limit, Some(8_000));
        // A model the list does not name fails the request; an entry without
        // a list keeps the selection as it is.
        let mut other = chosen();
        other.model.id = "gpt-4".into();
        assert_eq!(
            configured_selection(&entry(Some(serde_json::json!([model(8_000)]))), other),
            Err(NotConfigured("gpt-4".into()))
        );
        assert_eq!(configured_selection(&entry(None), chosen()), Ok(chosen()));
    }

    /// A subscription's model is priced as the same model on the vendor's
    /// API, from the backend's models.dev copy, Codex's at OpenAI's; a model
    /// the API does not sell, and an API entry whose own catalog has no
    /// price, stay unpriced (`usage-and-quota.md` § How the figures are
    /// counted). Planted defects: a subscription priced from its own catalog
    /// alone, or every entry priced from the vendor's listing.
    /// One served document on an empty ledger: milliseconds.
    #[tokio::test]
    async fn a_subscription_is_priced_as_its_model_on_the_vendors_api() {
        let vendor = MockVendor::start().await;
        let document = serde_json::json!({ "openai": { "id": "openai", "name": "OpenAI", "models": {
            "gpt-5.5": { "cost": { "input": 1.25, "output": 10.0, "cache_read": 0.125 } },
        } } });
        vendor.respond_at(
            "/api.json",
            MockResponse::status(200).header("content-type", "application/json").chunk(document.to_string()),
        );
        let data = tempfile::tempdir().unwrap();
        let clock: Arc<dyn demi_shared_types::Clock> = Arc::new(SystemClock);
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            clock.clone(),
            Arc::new(demi_backend_database::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        testing::master(&control).await;
        let vault = Vault::new(control.clone(), VaultKey::new([3; 32]), InstanceMode::Shared, Default::default());
        let edge = tokio::runtime::Handle::current();
        let models_dev = ModelsDevClient::new(reqwest::Client::new(), vendor.url("/api.json").parse().unwrap(), clock.clone());
        let assembly = ProviderAssembly::new(
            vault.clone(),
            FamilyRegistry::default(),
            AccountQuotas::new(vault, edge.clone(), clock.clone()),
            ModelCatalogCache::new(control.clone(), clock.clone(), edge),
            VendorCatalog::new(models_dev),
            reqwest::Client::new(),
            clock,
            demi_provider_common::media_catalog::MediaCatalog::new(
                reqwest::Client::new(),
                None,
                demi_provider_common::media_catalog::Catalog::carried(),
            ),
        );
        let codex = ProviderEntry {
            family: "codex".into(),
            credential: EntryCredential::Subscription { active: None, rotation: Rotation::Manual },
            ..entry(None)
        };
        let unbuilt = Err(AssemblyError::Incomplete);
        assert_eq!(
            assembly.model_price(&codex, &unbuilt, "gpt-5.5").await,
            Some(ModelCost { input: Some(1.25), output: Some(10.0), cache_read: Some(0.125), cache_write: None })
        );
        assert_eq!(assembly.model_price(&codex, &unbuilt, "codex-internal").await, None);
        let configured = entry(Some(serde_json::json!([model(4_000)])));
        assert_eq!(assembly.model_price(&configured, &unbuilt, "gpt-5.5").await, None);
        control.close().await.unwrap();
    }

}
