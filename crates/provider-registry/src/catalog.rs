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
//! A vendor's list leaves out each model that does not serve a
//! conversation, as far as Demi can tell:
//!
//! - One models.dev marks deprecated or not for chat, or LiteLLM dates as
//!   shut down (`crate::vendors`).
//! - One the vendor's own list for the key does not hold, for a provider
//!   that reads it ([`Provider::offered_models`]); a failed read fails the
//!   refresh, which keeps the last list.
//! - One retired for the entry's key: the vendor refused it to the key, or
//!   answered with another model it routes it to
//!   ([`ProviderRegistry::retire_model`]).
//!
//! A configured list and a subscription's directory are taken as they are.
//!
//! The module also builds the account-wide catalog `GET /api/models` answers,
//! each model with the selection the backend builds from it.

use std::sync::Arc;

use demi_provider_common::{Provider, is_snapshot};
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
        thinking_form: model.thinking_form,
        interleaved_thinking: model.interleaved_thinking,
    }
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
                let offering = provider.as_ref().ok().cloned();
                let http = self.http_for(entry, self.http().clone());
                let store = self.shared_store();
                let retiring = retiring_key(entry).map(|digest| (entry.id.clone(), digest));
                Box::new(move || {
                    Box::pin(async move {
                        let mut list = vendors.models(&vendor).await?;
                        if let Some(provider) = offering
                            && let Some(offered) = provider.offered_models(&http).await
                        {
                            // An alias is offered when the vendor lists a
                            // snapshot of it, as Anthropic lists dated IDs.
                            let offered = offered?;
                            list.models.retain(|model| {
                                offered.iter().any(|listed| *listed == model.id || is_snapshot(listed, &model.id))
                            });
                        }
                        if let Some((id, digest)) = retiring {
                            let retired = store.retired_models(id, digest).await.map_err(|error| error.to_string())?;
                            list.models.retain(|model| !retired.contains(&model.id));
                        }
                        Ok(list)
                    })
                })
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

    /// `selection` with the facts the entry's catalog states for its model
    /// now, as every inference boundary applies them
    /// (`crates/shared-types/src/model.rs`): its window, output limit,
    /// thinking capabilities and file types, while the thinking and tier stay
    /// as the user chose them. `provider` is the entry's, as for its catalog.
    ///
    /// The catalog is read as known now, so no boundary waits for a source.
    /// A model it does not list keeps the facts it has, since a catalog not
    /// read yet lists nothing; on an entry with a configured list, which names
    /// every model the entry serves, it is refused.
    pub async fn current_selection(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, RegistryError>,
        selection: ModelSelection,
    ) -> Result<ModelSelection, NotConfigured> {
        let catalog = self.read_entry_catalog(entry, provider, Reading::Known).await;
        if let Some(listed) = catalog.models.iter().find(|listed| listed.id == selection.model.id) {
            return Ok(listed.selection(
                entry.id.as_str(),
                selection.thinking,
                selection.service_tier_id,
            ));
        }
        match &entry.credential {
            EntryCredential::ApiKey(config) if config.models.is_some() => {
                Err(NotConfigured(selection.model.id))
            }
            _ => Ok(selection),
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

/// The digest of the key a model is retired for `entry` under: the SHA-256
/// of its API key; none for an entry whose list does not come from
/// models.dev, which retires no model.
fn retiring_key(entry: &ProviderEntry) -> Option<String> {
    let EntryCredential::ApiKey(config) = &entry.credential else {
        return None;
    };
    if config.models.is_some() || config.is_compatible() || config.vendor_id.is_none() {
        return None;
    }
    let key = config.api_key.as_ref()?;
    Some(hex::encode(Sha256::digest(key.expose().as_bytes())))
}

impl ProviderRegistry {
    /// Takes `model` out of `entry`'s list for as long as it holds its key:
    /// the vendor refused it to the key, or answered with `answered_by`,
    /// which it routes it to. The list is read again, so the pages hear of
    /// it, unless the model was retired already. An entry whose list does
    /// not come from models.dev keeps its models.
    pub async fn retire_model(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, RegistryError>,
        model: String,
        answered_by: Option<String>,
    ) {
        let Some(digest) = retiring_key(entry) else {
            return;
        };
        let stored = self
            .store()
            .retire_model(entry.id.clone(), digest, model.clone(), answered_by.clone(), self.now())
            .await;
        match stored {
            Ok(true) => {
                tracing::info!(
                    event = "model.retired",
                    provider = %entry.id,
                    model,
                    answered_by = answered_by.as_deref(),
                    "a model left an entry's list"
                );
                self.entry_catalog(entry, provider, true).await;
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(
                event = "model.retire_failed",
                provider = %entry.id,
                model,
                error = &error as &dyn std::error::Error,
                "a retired model was not kept"
            ),
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
