//! Where an entry's catalog comes from (`models.md` § Catalog sources): its
//! configured model list, read from the entry and never cached; its vendor's
//! list in models.dev; or its provider's directory, both through the catalog
//! cache. And the account-wide catalog `GET /api/models` answers, each model
//! with the selection the backend builds from it.

use std::sync::Arc;

use demi_core::{ModelSelection, ProviderModel, ProviderModelList, ServiceTier, Timestamp};
use demi_provider::Provider;
use demi_web_api::providers::{CatalogModel, CatalogProvider, ConfiguredModel};
use futures_util::future::join_all;
use sha2::{Digest, Sha256};

use super::assembly::{AssemblyError, ProviderAssembly};
use super::catalog_cache::CatalogFetch;
use crate::vault::entries::{EntryCredential, ProviderEntry};

/// A model the entry's configured list does not name; its request fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the model {0} is not in the provider's configured list")]
pub(crate) struct NotConfigured(pub(crate) String);

/// A configured model as a catalog model: it states its facts directly, its
/// first effort is its default, its Fast tier is its only tier, and it is
/// taken to call tools.
pub(crate) fn configured_model(model: &ConfiguredModel) -> ProviderModel {
    let efforts = &model.thinking_efforts;
    ProviderModel {
        id: model.id.as_str().to_owned(),
        display_name: model.display_name.as_str().to_owned(),
        description: None,
        context_window: Some(model.context_window),
        output_limit: model.output_limit,
        supports_tools: Some(true),
        supports_attachments: model.accepted_extensions.as_ref().map(|accepted| !accepted.is_empty()),
        supports_video: None,
        accepted_extensions: model.accepted_extensions.clone(),
        supports_reasoning: Some(!efforts.is_empty()),
        supported_thinking_efforts: Some(efforts.clone()),
        default_thinking_effort: efforts.first().cloned(),
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
pub(crate) fn configured_selection(
    entry: &ProviderEntry,
    selection: ModelSelection,
) -> Result<ModelSelection, NotConfigured> {
    let EntryCredential::ApiKey(config) = &entry.credential else {
        return Ok(selection);
    };
    let Some(models) = &config.models else {
        return Ok(selection);
    };
    let Some(model) = models.0.iter().find(|model| model.id.as_str() == selection.model.id) else {
        return Err(NotConfigured(selection.model.id));
    };
    Ok(configured_model(model).selection(entry.id.as_str(), selection.thinking, selection.service_tier_id))
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
    pub(crate) async fn entry_catalog(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, AssemblyError>,
        refresh: bool,
    ) -> ProviderModelList {
        let vendor = match &entry.credential {
            EntryCredential::ApiKey(config) => {
                if let Some(models) = &config.models {
                    return unfetched(models.0.iter().map(configured_model).collect(), Vec::new());
                }
                config.vendor_id.clone()
            }
            EntryCredential::Subscription { .. } => None,
        };
        let fetch: Box<dyn FnOnce() -> CatalogFetch + Send> = match (vendor, provider) {
            (Some(vendor), _) => {
                let vendors = self.vendors().clone();
                Box::new(move || Box::pin(async move { vendors.models(&vendor).await }))
            }
            (None, Ok(provider)) => {
                let provider = provider.clone();
                Box::new(move || {
                    Box::pin(async move { provider.list_models().await.map_err(|error| error.to_string()) })
                })
            }
            (None, Err(error)) => return unfetched(Vec::new(), vec![error.to_string()]),
        };
        match self
            .catalogs()
            .read(&entry.id, catalog_key(entry), fetch, refresh)
            .await
        {
            Ok(catalog) => catalog,
            Err(failure) => unfetched(Vec::new(), vec![failure]),
        }
    }

    /// The catalog of every entry in `entries`, each with its provider's
    /// health: one entry's failure leaves the others intact.
    pub(crate) async fn model_catalog(&self, entries: &[ProviderEntry], refresh: bool) -> Vec<CatalogProvider> {
        join_all(entries.iter().map(|entry| self.catalog_provider(entry, refresh))).await
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
            requires_process_capable_host: provider
                .as_ref()
                .is_ok_and(|provider| provider.capabilities().process_host),
            models,
            source_fetched_at: catalog.source_fetched_at,
            stale: catalog.stale,
            warnings: catalog.warnings,
            availability: super::details::availability(&auth, &runtime),
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
    let canonical = serde_json_canonicalizer::to_string(&identity).expect("a JSON value has a canonical form");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

#[cfg(test)]
mod tests {
    use demi_core::{Model, ThinkingConfig};
    use demi_provider::Secret;
    use demi_web_api::ids::{ProviderId, UserId};
    use demi_web_api::providers::ConfiguredModels;

    use super::*;
    use crate::vault::entries::ApiKeyConfig;

    fn entry(models: Option<serde_json::Value>) -> ProviderEntry {
        let models = models.map(|models| serde_json::from_value::<ConfiguredModels>(models).unwrap());
        ProviderEntry {
            id: ProviderId::try_from("entry-1").unwrap(),
            owner: UserId::try_from("owner").unwrap(),
            family: "openai".into(),
            label: "Work".into(),
            credential: EntryCredential::ApiKey(ApiKeyConfig {
                api_key: Secret::try_from("sk-1".to_owned()).unwrap(),
                base_url: None,
                wire_api: None,
                vendor_id: None,
                models,
            }),
            created_at: Timestamp::UNIX_EPOCH,
        }
    }

    fn model(output_limit: u32) -> serde_json::Value {
        serde_json::json!({
            "id": "gpt-5.5", "displayName": "GPT-5.5", "contextWindow": 272_000, "outputLimit": output_limit,
            "thinkingEfforts": ["low", "high"], "acceptedExtensions": ["png", "pdf"], "fastTier": "priority"
        })
    }

    /// The selection a browser sent with a conversation's first request,
    /// from an older catalog.
    fn chosen() -> ModelSelection {
        ModelSelection {
            provider_id: "entry-1".into(),
            model: Model {
                id: "gpt-5.5".into(),
                name: "stale".into(),
                context_window: 1000,
                input_limit: None,
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
        let applied = configured_selection(&entry(Some(serde_json::json!([model(4_000)]))), chosen()).unwrap();
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
            Some(vec![demi_core::FileExtension::Png, demi_core::FileExtension::Pdf])
        );
        assert_eq!(
            (applied.thinking, applied.service_tier_id),
            (chosen().thinking, chosen().service_tier_id)
        );
        // An edit of the list reaches the next request.
        let edited = configured_selection(&entry(Some(serde_json::json!([model(8_000)]))), chosen()).unwrap();
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
}
