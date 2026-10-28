//! Where an entry's catalog comes from (`crates/shared-types/src/catalog.rs`).
//!
//! An entry's catalog has one of three sources:
//!
//! - Its configured model list, read from the entry and never cached.
//! - Its vendor's list in models.dev, through the catalog cache.
//! - Its provider's directory, through the catalog cache.
//!
//! A compatible entry has no source but its configured list: empty until the
//! user adds a model. An unavailable source is not replaced by the next one. A
//! vendor entry whose vendor models.dev does not list has an empty catalog, and
//! a configured list is user configuration that no fetched catalog changes and
//! no refresh clears.
//!
//! The module also builds the account-wide catalog `GET /api/models` answers,
//! each model with the selection the backend builds from it.

use std::sync::Arc;

use demi_provider_common::Provider;
use demi_shared_types::{ModelCost, ModelSelection, ProviderModel, ProviderModelList, ServiceTier, Timestamp};
use demi_web_api_protocol::providers::{CatalogModel, CatalogProvider, ConfiguredModel, ModelKind};
use futures_util::future::join_all;
use sha2::{Digest, Sha256};

use crate::catalog_cache::CatalogFetch;
use crate::entry::{EntryCredential, ProviderEntry};
use crate::families::api_vendor;
use crate::registry::{ProviderRegistry, RegistryError};

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

/// `selection` with the facts of the entry's configured model applied again, as
/// every inference boundary applies them (`crates/shared-types/src/model.rs`).
///
/// An edit of a model's limits reaches the next request, and the thinking and
/// tier stay as the user chose them. An entry without a configured list keeps
/// the selection as it is.
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

/// How a read of an entry's catalog treats its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reading {
    /// Waits for the source when the entry has no record, or for a forced
    /// refresh when `refresh` is set.
    Waits { refresh: bool },
    /// Answers what is known now and waits for no source: an entry with no
    /// record shows no models until its source is read, whose end the
    /// cache announces.
    Known,
}

impl ProviderRegistry {
    /// The entry's catalog, from the first source it has. A failed read of
    /// the source is the catalog's warning: the last good copy stays, and
    /// without one the catalog is empty. `provider` is the entry's provider,
    /// or why it could not be built.
    pub async fn entry_catalog(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, RegistryError>,
        refresh: bool,
    ) -> ProviderModelList {
        self.read_entry_catalog(entry, provider, Reading::Waits { refresh }).await
    }

    async fn read_entry_catalog(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, RegistryError>,
        reading: Reading,
    ) -> ProviderModelList {
        // An entry of media alone has no text models to list.
        if !self.makes_text(entry) {
            return unfetched(Vec::new(), Vec::new());
        }
        let vendor = match &entry.credential {
            EntryCredential::ApiKey(config) => {
                // The model menu's are the text models; an image model of
                // the list is `demi image`'s
                // (`crates/backend-user-shard/src/provider_settings.rs`).
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
            (None, Err(RegistryError::Incomplete)) => return unfetched(Vec::new(), Vec::new()),
            (None, Err(error)) => return unfetched(Vec::new(), vec![error.to_string()]),
        };
        let key = catalog_key(entry);
        let answer = match reading {
            Reading::Waits { refresh } => self
                .catalogs()
                .read(&entry.id, key, revision, fetch, refresh)
                .await
                .map(Some),
            Reading::Known => self.catalogs().known(&entry.id, key, revision, fetch).await,
        };
        match answer {
            Ok(Some(catalog)) => catalog,
            Ok(None) => unfetched(Vec::new(), Vec::new()),
            Err(failure) => unfetched(Vec::new(), vec![failure]),
        }
    }

    /// The prices `model` of the entry is priced at as its calls' rows are
    /// written (`crates/backend-database/src/usage.rs`).
    ///
    /// - The entry's catalog gives them.
    /// - For a subscription, whose catalog has none, they are the same model's
    ///   prices on the vendor's API, from the backend's models.dev copy, which
    ///   asks models.dev at most once a day.
    ///
    /// The result is none for a model the API does not sell. `provider` is the
    /// entry's, as for its catalog.
    pub async fn model_price(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, RegistryError>,
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
        self.catalog_providers(entries, Reading::Waits { refresh }).await
    }

    /// [`model_catalog`](Self::model_catalog) as a page shows it, which waits
    /// for no source: an entry whose source was never read lists no models
    /// until the cache announces its read.
    pub async fn known_model_catalog(&self, entries: &[ProviderEntry]) -> Vec<CatalogProvider> {
        self.catalog_providers(entries, Reading::Known).await
    }

    async fn catalog_providers(&self, entries: &[ProviderEntry], reading: Reading) -> Vec<CatalogProvider> {
        join_all(entries.iter().map(|entry| self.catalog_provider(entry, reading))).await
    }

    async fn catalog_provider(&self, entry: &ProviderEntry, reading: Reading) -> CatalogProvider {
        let provider = self.provider_for(entry).await;
        let catalog = self.read_entry_catalog(entry, &provider, reading).await;
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
                .and_then(|provider| crate::claude_releases::cli_package(provider.as_ref())),
            models,
            // The media service fills them, which knows the media catalog.
            image_models: Vec::new(),
            video_models: Vec::new(),
            audio_models: Vec::new(),
            source_fetched_at: catalog.source_fetched_at,
            stale: catalog.stale,
            warnings: catalog.warnings,
            availability: crate::details::availability(entry, &auth, &runtime),
            auth,
            runtime,
        }
    }
}

/// The digest a catalog record is kept under: the SHA-256 of the RFC 8785
/// form of the entry's family, configuration and active account, so a
/// changed entry starts from an empty cache
/// (`crates/backend-database/src/columns.rs`).
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
