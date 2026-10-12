//! The changes of the provider entries in a user's scope (`web-api.md`
//! § Model configuration and provider inspection), and the model search
//! that fills a configured model (`models.md` § Adding a model from
//! models.dev): one path, with its checks and its words, for the settings
//! page and for `demi providers` (`models.md` § Managing providers from a
//! conversation). An entry may be added with nothing but its kind; it is
//! incomplete until it has what it needs.

use demi_backend_database::StorageError;
use demi_backend_providers::llm::families::{ProviderFamily, family_name};
use demi_backend_providers::vault::entries::{ApiKeyConfig, EntryCredential, ProviderEntry};
use demi_backend_providers::vault::operations::OperationGuard;
use demi_provider_common::{Secret, SecretError};
use demi_shared_types::WireApi;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::ids::{ProviderId, UserId};
use demi_web_api_protocol::providers::{CreateProvider, CredentialKind, ModelMatch, ProviderPatch};
use demi_web_api_protocol::text::EndpointUrl;
use std::sync::Arc;

use crate::services::Services;

/// Why a change of an entry, or a search, was refused, in the words both
/// the settings page and `demi providers` show.
#[derive(Debug, thiserror::Error)]
pub enum ProviderRefusal {
    /// The user only infers with the scope's entries.
    #[error("Providers are configured by the instance owner")]
    NotConfiguring,
    #[error("No such provider")]
    NotFound,
    #[error("Another change of this provider is still running")]
    Busy,
    #[error("Unknown provider type \"{0}\"")]
    UnknownFamily(String),
    /// A subscription family, whose entry a login makes.
    #[error("Provider type \"{0}\" is configured by its login")]
    LoginFamily(String),
    #[error("wireApi: the {family} family does not speak {wire}")]
    Wire { family: String, wire: WireApi },
    #[error("Unknown vendor \"{0}\"")]
    UnknownVendor(String),
    /// The models.dev document could not be read.
    #[error("{0}")]
    CatalogUnavailable(String),
    #[error("apiKey: {0}")]
    Key(#[from] SecretError),
    /// A key or an endpoint for a subscription entry.
    #[error("A subscription entry takes only a new label")]
    SubscriptionOnly,
    /// A model list for a subscription entry, whose models come from its
    /// provider; it names the entry.
    #[error("{0} lists its own models; its list cannot be edited.")]
    OwnModels(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl Services {
    /// Refuses a user who does not configure the entries of their scope.
    pub fn may_configure(&self, user: &UserDto) -> Result<(), ProviderRefusal> {
        if self.vault.configures(user) {
            Ok(())
        } else {
            Err(ProviderRefusal::NotConfiguring)
        }
    }

    /// The entry `id` when it is one of `user`'s scope; any other reads as
    /// missing.
    pub async fn scoped_entry(
        &self,
        user: &UserId,
        id: &str,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        let id = ProviderId::try_from(id).map_err(|_| ProviderRefusal::NotFound)?;
        self.vault
            .visible(user, &id)
            .await?
            .ok_or(ProviderRefusal::NotFound)
    }

    /// Holds `entry` for one change.
    pub fn hold_entry(&self, entry: &ProviderEntry) -> Result<OperationGuard, ProviderRefusal> {
        self.operations
            .reserve(&entry.id)
            .ok_or(ProviderRefusal::Busy)
    }

    /// Creates an API-key entry from the vendor list or for a custom
    /// endpoint. Its label is the vendor's or the family's name unless the
    /// body gives one, and what the body leaves out leaves it incomplete.
    pub async fn create_provider(
        &self,
        user: &UserDto,
        body: CreateProvider,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        self.may_configure(user)?;
        let (family, label, config) = match body {
            CreateProvider::Vendor {
                vendor_id,
                label,
                api_key,
                base_url,
                models,
            } => {
                let vendor = self
                    .assembly
                    .vendors()
                    .vendor(&vendor_id)
                    .await
                    .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))?
                    .ok_or(ProviderRefusal::UnknownVendor(vendor_id))?;
                self.api_key_family(&vendor.provider_type, vendor.wire_api)?;
                let base_url = match base_url {
                    Some(endpoint) => Some(endpoint),
                    None => vendor
                        .base_url
                        .map(EndpointUrl::try_from)
                        .transpose()
                        .map_err(|_| {
                            ProviderRefusal::CatalogUnavailable(format!(
                                "models.dev names no usable endpoint for {}",
                                vendor.id
                            ))
                        })?,
                };
                let label = label.map_or(vendor.name, |label| label.into_string());
                let config = ApiKeyConfig {
                    api_key: api_key.map(key).transpose()?,
                    base_url,
                    wire_api: vendor.wire_api,
                    vendor_id: Some(vendor.id),
                    models,
                };
                (vendor.provider_type, label, config)
            }
            CreateProvider::Custom {
                provider_type,
                wire_api,
                label,
                api_key,
                base_url,
                models,
            } => {
                self.api_key_family(&provider_type, wire_api)?;
                let label = label.map_or_else(
                    || family_name(&provider_type).to_owned(),
                    |label| label.into_string(),
                );
                let config = ApiKeyConfig {
                    api_key: api_key.map(key).transpose()?,
                    base_url,
                    wire_api,
                    vendor_id: None,
                    models,
                };
                (provider_type, label, config)
            }
        };
        let owner = self.vault.owner_for(&user.id).await?;
        Ok(self
            .vault
            .create_api_key(owner, family, label, config)
            .await?)
    }

    /// Edits the entry `id`: the label of any, and the key, endpoint and
    /// model list of an API-key entry. A configuration edit starts the
    /// entry's provider and catalog afresh.
    pub async fn update_provider(
        &self,
        user: &UserDto,
        id: &str,
        patch: ProviderPatch,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        self.may_configure(user)?;
        let entry = self.scoped_entry(&user.id, id).await?;
        let _held = self.hold_entry(&entry)?;
        let reconfigures =
            patch.api_key.is_some() || patch.base_url.is_some() || patch.models.is_some();
        let config = if reconfigures {
            let EntryCredential::ApiKey(config) = &entry.credential else {
                return Err(if patch.models.is_some() {
                    ProviderRefusal::OwnModels(entry.label.clone())
                } else {
                    ProviderRefusal::SubscriptionOnly
                });
            };
            let mut config = config.clone();
            if let Some(text) = patch.api_key {
                config.api_key = Some(key(text)?);
            }
            if let Some(base_url) = patch.base_url {
                config.base_url = base_url;
            }
            if let Some(models) = patch.models {
                config.models = models;
            }
            Some(config)
        } else {
            None
        };
        let label = patch.label.map(|label| label.into_string());
        let updated = self
            .vault
            .update(entry.id.clone(), label, config)
            .await?
            .ok_or(ProviderRefusal::NotFound)?;
        if reconfigures {
            self.assembly.invalidate(&entry.id).await?;
        }
        Ok(updated)
    }

    /// Deletes the entry `id` with its accounts, catalog and quota
    /// snapshots; answers the entry as it was.
    pub async fn delete_provider(
        &self,
        user: &UserDto,
        id: &str,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        self.may_configure(user)?;
        let entry = self.scoped_entry(&user.id, id).await?;
        let _held = self.hold_entry(&entry)?;
        self.vault.delete(&entry).await?;
        self.assembly.forget(&entry.id).await?;
        self.cli_installs.forget(&entry.id);
        Ok(entry)
    }

    /// The models.dev models `query` matches, filled from the vendor of the
    /// entry `entry` of `user`'s scope where it lists them.
    pub async fn search_models(
        &self,
        user: &UserId,
        query: &str,
        entry: Option<&str>,
    ) -> Result<Vec<ModelMatch>, ProviderRefusal> {
        let vendor = match entry {
            Some(id) => match self.scoped_entry(user, id).await?.credential {
                EntryCredential::ApiKey(config) => config.vendor_id,
                EntryCredential::Subscription { .. } => None,
            },
            None => None,
        };
        self.assembly
            .vendors()
            .search(query, vendor.as_deref())
            .await
            .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))
    }

    /// The models.dev model `reference`, `<vendor>/<model id>`, filled as a
    /// search fills it; none when that vendor lists no such model.
    pub async fn models_dev_model(
        &self,
        reference: &str,
    ) -> Result<Option<ModelMatch>, ProviderRefusal> {
        self.assembly
            .vendors()
            .model(reference)
            .await
            .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))
    }

    /// The API-key family `name`, which must speak `wire_api` when the
    /// entry names one.
    fn api_key_family(
        &self,
        name: &str,
        wire_api: Option<WireApi>,
    ) -> Result<&Arc<dyn ProviderFamily>, ProviderRefusal> {
        let family = self
            .assembly
            .families()
            .get(name)
            .ok_or_else(|| ProviderRefusal::UnknownFamily(name.to_owned()))?;
        if family.credential() == CredentialKind::Subscription {
            return Err(ProviderRefusal::LoginFamily(name.to_owned()));
        }
        if let Some(wire) = wire_api.filter(|wire| !family.wires().contains(wire)) {
            return Err(ProviderRefusal::Wire {
                family: name.to_owned(),
                wire,
            });
        }
        Ok(family)
    }
}

/// A key as the vault keeps it: one line of text.
fn key(text: String) -> Result<Secret, ProviderRefusal> {
    Ok(Secret::try_from(text)?)
}
