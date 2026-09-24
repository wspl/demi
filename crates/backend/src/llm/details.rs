//! What the product shows of an entry's provider (`web-api.md` § Model
//! configuration and provider inspection): its health, its accounts with the
//! quota kept for each, and its capabilities, read when the backend answers
//! and never stored. A user who only infers with an entry learns whether it
//! works, not whose account it is or how much of it is used
//! (`providers.md` § Scope).

use std::sync::Arc;

use demi_core::{AuthState, RuntimeState};
use demi_provider::Provider;
use demi_provider::quota::ProbeCost;
use demi_web_api::providers::{
    AccountDto, Availability, ProbeCost as ProbeCostDto, ProviderDetails, QuotaCapability, UnavailableReason,
};

use super::assembly::{AssemblyError, ProviderAssembly};
use crate::vault::entries::{EntryCredential, ProviderEntry};
use crate::vault::pool::meta;

impl ProviderAssembly {
    /// The provider's authentication and runtime state. A subscription entry
    /// without an account is unauthenticated whatever its provider says.
    pub(crate) async fn health(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, AssemblyError>,
    ) -> (AuthState, RuntimeState) {
        let provider = match provider {
            Ok(provider) => provider,
            Err(error) => {
                let auth = AuthState::Error {
                    message: error.to_string(),
                };
                return (auth, RuntimeState::Unknown { message: None });
            }
        };
        let missing_account =
            matches!(entry.credential, EntryCredential::Subscription { .. }) && entry.active().is_none();
        let auth = if missing_account {
            AuthState::Unauthenticated {
                message: Some("No subscription account configured".into()),
            }
        } else {
            provider.auth_status().await
        };
        (auth, provider.runtime_state())
    }

    /// The entry's details; `disclose` for a user who configures the entry,
    /// and without it only what a user who infers may know.
    pub(crate) async fn details(
        &self,
        entry: &ProviderEntry,
        disclose: bool,
    ) -> Result<ProviderDetails, AssemblyError> {
        let provider = self.provider_for(entry).await;
        let (auth, runtime) = self.health(entry, &provider).await;
        let provider = provider?;
        let accounts = match entry.credential {
            EntryCredential::Subscription { .. } => self.vault().accounts(entry.id.clone()).await?,
            EntryCredential::ApiKey(_) => Vec::new(),
        };
        let accounts: Vec<AccountDto> = accounts
            .iter()
            .map(|record| AccountDto {
                account: meta(record).info(),
                quota: self.quotas().latest(&entry.id, record),
            })
            .collect();
        let active = entry.active().cloned();
        let quota = accounts
            .iter()
            .find(|account| Some(account.account.id.as_str()) == active.as_ref().map(|active| active.as_str()))
            .and_then(|account| account.quota.clone());
        let quota_capability = match provider.quota() {
            None => QuotaCapability::None {},
            Some(quota) => QuotaCapability::Supported {
                probe: quota.probe_cost().map(|cost| match cost {
                    ProbeCost::Free => ProbeCostDto::Free,
                    ProbeCost::Inference => ProbeCostDto::Inference,
                }),
            },
        };
        let details = ProviderDetails {
            auth,
            runtime,
            accounts,
            active,
            quota,
            quota_capability,
            requires_process_capable_host: provider.capabilities().process_host,
        };
        Ok(if disclose { details } else { for_inference_only(details) })
    }
}

/// What a user who only infers may know: no accounts, no active account, no
/// quota, and no account label.
fn for_inference_only(details: ProviderDetails) -> ProviderDetails {
    let auth = match details.auth {
        AuthState::Authenticated { .. } => AuthState::Authenticated { account_label: None },
        other => other,
    };
    ProviderDetails {
        auth,
        accounts: Vec::new(),
        active: None,
        quota: None,
        ..details
    }
}

/// Whether the entry's models can be used now, from its health: a credential
/// that is missing or refused, or a provider that cannot run, makes them
/// unavailable.
pub(crate) fn availability(auth: &AuthState, runtime: &RuntimeState) -> Availability {
    if matches!(auth, AuthState::Unauthenticated { .. } | AuthState::Error { .. }) {
        return Availability::Unavailable {
            reason: UnavailableReason::Authentication,
            message: "Provider login is unavailable".into(),
        };
    }
    match runtime {
        RuntimeState::Unavailable { message } | RuntimeState::Error { message } => Availability::Unavailable {
            reason: UnavailableReason::Runtime,
            message: message.clone(),
        },
        RuntimeState::Unknown { .. } | RuntimeState::Ready { .. } => Availability::Available {},
    }
}
