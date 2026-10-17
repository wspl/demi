//! `/api/providers` (`web-api-settings.md` § Model configuration and provider
//! inspection): the entries of the caller's scope, the vendors an entry can
//! be added from, and each entry's status, quota and test. Every user of a
//! scope reads it; configuring, testing and refreshing usage are the
//! master's on a shared instance and each owner's on an isolated one. No
//! answer carries key material.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use demi_backend_providers::llm::assembly::AssemblyError;
use demi_backend_providers::vault::entries::ProviderEntry;
use demi_backend_providers::vault::operations::OperationGuard;
use demi_provider_common::Provider;
use demi_provider_common::quota::QuotaError;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::CredentialId;
use demi_web_api_protocol::providers::{
    CreateProvider, ProviderAnswer, ProviderDetails, ProviderPatch, Providers, QuotaAnswer,
    QuotaRequest, SubscriptionFamily, TestRequest, TestResult, VendorCatalog,
};

use super::body::{JsonBody, OptionalJsonBody};
use super::error::ApiError;
use super::gate::AuthUser;
use demi_backend_user_shard::provider_settings::ProviderRefusal;
use demi_backend_user_shard::services::Services;
use demi_backend_user_shard::shard::Shards;

/// Refuses a user who does not configure the scope's entries.
pub(super) fn configures(services: &Services, user: &UserDto) -> Result<(), ApiError> {
    services.may_configure(user).map_err(refused)
}

/// The entry the path names, when it is one of the caller's scope; any
/// other answers like a missing one.
pub(super) async fn scoped(
    services: &Services,
    user: &UserDto,
    id: &str,
) -> Result<ProviderEntry, ApiError> {
    services.scoped_entry(&user.id, id).await.map_err(refused)
}

/// Holds the entry for one change.
pub(super) fn reserve(
    services: &Services,
    entry: &ProviderEntry,
) -> Result<OperationGuard, ApiError> {
    services.hold_entry(entry).map_err(refused)
}

/// A refused change or search as the page receives it.
pub(super) fn refused(refusal: ProviderRefusal) -> ApiError {
    let message = refusal.to_string();
    let (status, code) = match refusal {
        ProviderRefusal::Storage(error) => return error.into(),
        ProviderRefusal::NotConfiguring => return ApiError::forbidden(message),
        ProviderRefusal::NotFound => return ApiError::provider_not_found(),
        ProviderRefusal::Busy => return ApiError::provider_busy(),
        ProviderRefusal::Wire { .. } | ProviderRefusal::Key(_) | ProviderRefusal::MakesNoImages(_) => {
            return ApiError::invalid_body(message);
        }
        ProviderRefusal::UnknownFamily(_) => {
            (StatusCode::BAD_REQUEST, ErrorCode::UnknownProviderType)
        }
        ProviderRefusal::UnknownVendor(_) => (StatusCode::BAD_REQUEST, ErrorCode::UnknownVendor),
        ProviderRefusal::LoginFamily(_)
        | ProviderRefusal::SubscriptionOnly
        | ProviderRefusal::OwnModels(_) => (StatusCode::BAD_REQUEST, ErrorCode::SubscriptionOnly),
        ProviderRefusal::VendorEndpoint(_) | ProviderRefusal::VendorProtocol(_) => {
            (StatusCode::BAD_REQUEST, ErrorCode::EndpointKind)
        }
        ProviderRefusal::DuplicateModel(_) => (StatusCode::BAD_REQUEST, ErrorCode::DuplicateModel),
        ProviderRefusal::RotationNeedsAccounts => {
            (StatusCode::BAD_REQUEST, ErrorCode::RotationNeedsAccounts)
        }
        ProviderRefusal::CatalogUnavailable(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::CatalogUnavailable)
        }
    };
    ApiError::new(status, code, message)
}

pub(super) async fn list(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
) -> Result<Json<Providers>, ApiError> {
    let owner = services.vault.owner_for(&user.id).await?;
    let entries = services.vault.entries(owner).await?;
    Ok(Json(Providers {
        providers: entries
            .iter()
            .map(|entry| services.assembly.provider_dto(entry))
            .collect(),
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
    let vendors = assembly
        .vendors()
        .vendors()
        .await
        .map_err(catalog_unavailable)?;
    Ok(Json(VendorCatalog {
        subscriptions,
        vendors,
    }))
}

fn catalog_unavailable(error: impl ToString) -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::CatalogUnavailable,
        error.to_string(),
    )
}

/// Creates an API-key entry, a vendor's or a compatible API;
/// what the body leaves out leaves it incomplete.
pub(super) async fn create(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    JsonBody(body): JsonBody<CreateProvider>,
) -> Result<(StatusCode, Json<ProviderAnswer>), ApiError> {
    let entry = services
        .create_provider(&user, body)
        .await
        .map_err(refused)?;
    Ok((
        StatusCode::CREATED,
        Json(ProviderAnswer {
            provider: services.assembly.provider_dto(&entry),
        }),
    ))
}

/// Edits an entry: the label of any, and the key, endpoint and model list of
/// an API-key entry.
pub(super) async fn update(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
    JsonBody(patch): JsonBody<ProviderPatch>,
) -> Result<Json<ProviderAnswer>, ApiError> {
    let updated = services
        .update_provider(&user, &id, patch)
        .await
        .map_err(refused)?;
    Ok(Json(ProviderAnswer {
        provider: services.assembly.provider_dto(&updated),
    }))
}

/// Deletes an entry with its accounts, catalog and quota snapshots.
pub(super) async fn delete(
    State(services): State<Arc<Services>>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    services
        .delete_provider(&user, &id)
        .await
        .map_err(refused)?;
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
    match services.assembly.details(&entry, disclose).await {
        Ok(details) => Ok(Json(details)),
        Err(AssemblyError::Storage(error)) => Err(error.into()),
        // As the entry's `failed` details in the sync state say it.
        Err(error) => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::ProviderStatusFailed,
            error.to_string(),
        )),
    }
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
                StatusCode::SERVICE_UNAVAILABLE,
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
    let account = request.credential_id;
    let provider = account_provider(&services, &entry, account.as_ref()).await?;
    let model = request.model_id;
    let result = shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard
                .test_provider(entry, provider, account, model, cancel)
                .await
        })
        .await?;
    Ok(Json(result))
}
