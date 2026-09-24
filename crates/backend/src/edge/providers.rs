//! `/api/providers` (`web-api.md` § Model configuration and provider
//! inspection): the entries of the caller's scope, the vendors an entry can
//! be added from, and each entry's status, quota and test. Every user of a
//! scope reads it; configuring, testing and refreshing usage are the
//! master's on a shared instance and each owner's on an isolated one. No
//! answer carries key material.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_core::WireApi;
use demi_provider::quota::QuotaError;
use demi_provider::{Provider, Secret};
use demi_web_api::auth::UserDto;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::{CredentialId, ProviderId};
use demi_web_api::providers::{
    CreateProvider, CredentialKind, ProviderAnswer, ProviderDetails, ProviderPatch, Providers, QuotaAnswer,
    QuotaRequest, SubscriptionFamily, TestRequest, TestResult, VendorCatalog,
};
use demi_web_api::text::EndpointUrl;

use super::body::{JsonBody, OptionalJsonBody};
use super::error::ApiError;
use super::gate::AuthUser;
use crate::backend::Services;
use crate::llm::families::ProviderFamily;
use crate::shard::Shards;
use crate::vault::entries::{ApiKeyConfig, EntryCredential, ProviderEntry};
use crate::vault::operations::OperationGuard;

/// Refuses a user who does not configure the scope's entries.
pub(super) fn configures(services: &Services, user: &UserDto) -> Result<(), ApiError> {
    if services.vault.configures(user) {
        Ok(())
    } else {
        Err(ApiError::forbidden("Providers are configured by the instance owner"))
    }
}

/// The entry the path names, when it is one of the caller's scope; any
/// other answers like a missing one.
pub(super) async fn scoped(services: &Services, user: &UserDto, id: &str) -> Result<ProviderEntry, ApiError> {
    let id = ProviderId::try_from(id).map_err(|_| ApiError::provider_not_found())?;
    services
        .vault
        .visible(&user.id, &id)
        .await?
        .ok_or_else(ApiError::provider_not_found)
}

/// Holds the entry for one change.
pub(super) fn reserve(services: &Services, entry: &ProviderEntry) -> Result<OperationGuard, ApiError> {
    services
        .operations
        .reserve(&entry.id)
        .ok_or_else(ApiError::provider_busy)
}

pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<Providers>, ApiError> {
    let owner = services.vault.owner_for(&user.id).await?;
    let entries = services.vault.entries(owner).await?;
    Ok(Json(Providers {
        providers: entries.iter().map(ProviderEntry::dto).collect(),
    }))
}

/// What the page can add: each subscription family with whether the scope
/// holds its entry, and the models.dev vendors a family speaks to.
pub(super) async fn catalog(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<VendorCatalog>, ApiError> {
    let owner = services.vault.owner_for(&user.id).await?;
    let entries = services.vault.entries(owner).await?;
    let assembly = &services.assembly;
    let subscriptions = assembly
        .families()
        .subscriptions()
        .map(|family| SubscriptionFamily {
            provider_type: family.to_owned(),
            configured: entries.iter().any(|entry| entry.family == family),
        })
        .collect();
    let vendors = assembly.vendors().vendors().await.map_err(catalog_unavailable)?;
    Ok(Json(VendorCatalog { subscriptions, vendors }))
}

fn catalog_unavailable(error: impl ToString) -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        ErrorCode::CatalogUnavailable,
        error.to_string(),
    )
}

fn unknown_family(family: &str) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        ErrorCode::UnknownProviderType,
        format!("Unknown provider type \"{family}\""),
    )
}

/// The key a body carries, as a credential: one line of text.
fn api_key(text: String) -> Result<Secret, ApiError> {
    Secret::try_from(text).map_err(|error| ApiError::invalid_body(format!("apiKey: {error}")))
}

/// The API-key family `name`, which must speak `wire_api` when the entry
/// names one.
fn api_key_family<'a>(
    services: &'a Services,
    name: &str,
    wire_api: Option<WireApi>,
) -> Result<&'a Arc<dyn ProviderFamily>, ApiError> {
    let family = services
        .assembly
        .families()
        .get(name)
        .ok_or_else(|| unknown_family(name))?;
    if family.credential() == CredentialKind::Subscription {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::SubscriptionOnly,
            format!("Provider type \"{name}\" is configured by its login"),
        ));
    }
    if let Some(wire) = wire_api.filter(|wire| !family.wires().contains(wire)) {
        return Err(ApiError::invalid_body(format!(
            "wireApi: the {name} family does not speak {wire}"
        )));
    }
    Ok(family)
}

/// Creates an API-key entry, from the vendor list or for a custom endpoint.
pub(super) async fn create(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(body): JsonBody<CreateProvider>,
) -> Result<(StatusCode, Json<ProviderAnswer>), ApiError> {
    configures(&services, &user)?;
    let (family, label, config) = match body {
        CreateProvider::Vendor {
            vendor_id,
            label,
            api_key: key,
            base_url,
            models,
        } => {
            let vendor = services
                .assembly
                .vendors()
                .vendor(&vendor_id)
                .await
                .map_err(catalog_unavailable)?
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        ErrorCode::UnknownVendor,
                        format!("Unknown vendor \"{vendor_id}\""),
                    )
                })?;
            api_key_family(&services, &vendor.provider_type, vendor.wire_api)?;
            let base_url =
                match base_url {
                    Some(endpoint) => Some(endpoint),
                    None => vendor.base_url.map(EndpointUrl::try_from).transpose().map_err(|_| {
                        catalog_unavailable(format!("models.dev names no usable endpoint for {vendor_id}"))
                    })?,
                };
            let config = ApiKeyConfig {
                api_key: api_key(key)?,
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
            api_key: key,
            base_url,
            models,
        } => {
            api_key_family(&services, &provider_type, wire_api)?;
            let config = ApiKeyConfig {
                api_key: api_key(key)?,
                base_url,
                wire_api,
                vendor_id: None,
                models,
            };
            (provider_type, label, config)
        }
    };
    let owner = services.vault.owner_for(&user.id).await?;
    let entry = services
        .vault
        .create_api_key(owner, family, label.into_string(), config)
        .await?;
    Ok((StatusCode::CREATED, Json(ProviderAnswer { provider: entry.dto() })))
}

/// Edits an entry: the label of any, and the key, endpoint and model list of
/// an API-key entry. A configuration edit starts the entry's provider and
/// catalog afresh.
pub(super) async fn update(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<ProviderPatch>,
) -> Result<Json<ProviderAnswer>, ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let _held = reserve(&services, &entry)?;
    let reconfigures = patch.api_key.is_some() || patch.base_url.is_some() || patch.models.is_some();
    let config = if reconfigures {
        let EntryCredential::ApiKey(config) = &entry.credential else {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::SubscriptionOnly,
                "A subscription entry takes only a new label",
            ));
        };
        let mut config = config.clone();
        if let Some(key) = patch.api_key {
            config.api_key = api_key(key)?;
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
    let updated = services
        .vault
        .update(entry.id.clone(), label, config)
        .await?
        .ok_or_else(ApiError::provider_not_found)?;
    if reconfigures {
        services.assembly.invalidate(&entry.id).await?;
    }
    Ok(Json(ProviderAnswer {
        provider: updated.dto(),
    }))
}

/// Deletes an entry with its accounts, catalog and quota snapshots.
pub(super) async fn delete(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let _held = reserve(&services, &entry)?;
    services.vault.delete(entry.id.clone()).await?;
    services.assembly.forget(&entry.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The entry's health, accounts and quota; reading it never probes and
/// never runs inference.
pub(super) async fn status(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ProviderDetails>, ApiError> {
    let entry = scoped(&services, &user, &id).await?;
    let disclose = services.vault.configures(&user);
    Ok(Json(services.assembly.details(&entry, disclose).await?))
}

/// The entry's provider for `account`, or for its active account without
/// one.
async fn account_provider(
    services: &Services,
    entry: &ProviderEntry,
    account: Option<&CredentialId>,
) -> Result<Arc<dyn Provider>, ApiError> {
    match account {
        Some(account) => services
            .assembly
            .for_account(entry, account)
            .await?
            .ok_or_else(ApiError::account_not_found),
        None => Ok(services.assembly.provider_for(entry).await?),
    }
}

/// Probes the quota of the account the body names, or of the active one:
/// only a free probe runs, and a family that cannot probe answers its kept
/// snapshot. Cancelling the request stops the probe.
pub(super) async fn quota(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    OptionalJsonBody(request): OptionalJsonBody<QuotaRequest>,
) -> Result<Json<QuotaAnswer>, ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let provider = account_provider(&services, &entry, request.credential_id.as_ref()).await?;
    let Some(quota) = provider.quota() else {
        return Ok(Json(QuotaAnswer { quota: None }));
    };
    let snapshot = match quota.probe().await {
        Ok(snapshot) => Some(snapshot),
        Err(QuotaError::Unsupported) => quota.latest(),
        Err(QuotaError::RequiresInference) => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::QuotaRequiresInference,
                "This provider cannot read its usage without an inference request",
            ));
        }
        Err(error) => {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                ErrorCode::QuotaUnavailable,
                error.to_string(),
            ));
        }
    };
    Ok(Json(QuotaAnswer {
        quota: snapshot.map(|snapshot| (*snapshot).clone()),
    }))
}

/// One real request to the model the body names, with the account it names
/// or the active one, on the acting user's shard.
pub(super) async fn test(
    State(services): State<Arc<Services>>,
    State(shards): State<Shards>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<TestRequest>,
) -> Result<Json<TestResult>, ApiError> {
    configures(&services, &user)?;
    let entry = scoped(&services, &user, &id).await?;
    let _held = reserve(&services, &entry)?;
    let provider = account_provider(&services, &entry, request.credential_id.as_ref()).await?;
    let model = request.model_id;
    let result = shards
        .of(&user.id)
        .call(move |shard, cancel| async move { shard.test_provider(entry, provider, model, cancel).await })
        .await?;
    Ok(Json(result))
}
