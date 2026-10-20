//! What the product shows of an entry's provider: its health, its accounts with
//! the quota kept for each, and its capabilities, read when the backend answers
//! and never stored. A user who only infers with an entry learns whether it
//! works, not whose account it is or how much of it is used
//! (`crates/backend-providers/src/vault/entries.rs`).

use std::sync::Arc;

use demi_provider_common::Provider;
use demi_provider_common::quota::ProbeCost;
use demi_shared_types::{AuthState, RuntimeState};
use demi_web_api_protocol::providers::{
    AccountDto, Availability, ProbeCost as ProbeCostDto, ProviderDetails, QuotaCapability,
    UnavailableReason,
};

use super::assembly::{AssemblyError, ProviderAssembly};
use crate::vault::entries::{EntryCredential, ProviderEntry};
use crate::vault::pool::meta;

impl ProviderAssembly {
    /// The provider's authentication and runtime state. A subscription entry
    /// without an account is unauthenticated whatever its provider says.
    pub async fn health(
        &self,
        entry: &ProviderEntry,
        provider: &Result<Arc<dyn Provider>, AssemblyError>,
    ) -> (AuthState, RuntimeState) {
        let provider = match provider {
            Ok(provider) => provider,
            // Nothing is known of a provider that is not set up yet.
            Err(AssemblyError::Incomplete) => {
                return (
                    AuthState::Unauthenticated { message: None },
                    RuntimeState::Unknown { message: None },
                );
            }
            Err(error) => {
                let auth = AuthState::Error {
                    message: error.to_string(),
                };
                return (auth, RuntimeState::Unknown { message: None });
            }
        };
        let missing_account = matches!(entry.credential, EntryCredential::Subscription { .. })
            && entry.active().is_none();
        let auth = if missing_account {
            AuthState::Unauthenticated {
                message: Some(crate::vault::accounts::NO_ACCOUNT.into()),
            }
        } else {
            provider.auth_status().await
        };
        (auth, provider.runtime_state())
    }

    /// The entry's details; `disclose` for a user who configures the entry,
    /// and without it only what a user who infers may know.
    pub async fn details(
        &self,
        entry: &ProviderEntry,
        disclose: bool,
    ) -> Result<ProviderDetails, AssemblyError> {
        let provider = self.provider_for(entry).await;
        let (auth, runtime) = self.health(entry, &provider).await;
        let provider = match provider {
            Ok(provider) => provider,
            // An entry without its key has no provider to ask,
            // nor accounts or quota yet.
            Err(AssemblyError::Incomplete) => {
                return Ok(ProviderDetails {
                    auth,
                    runtime,
                    accounts: Vec::new(),
                    active: None,
                    quota: None,
                    quota_capability: QuotaCapability::None {},
                    cli_package: None,
                });
            }
            Err(error) => return Err(error),
        };
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
            .find(|account| {
                Some(account.account.id.as_str()) == active.as_ref().map(|active| active.as_str())
            })
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
            cli_package: super::claude_releases::cli_package(provider.as_ref()),
        };
        Ok(if disclose {
            details
        } else {
            for_inference_only(details)
        })
    }
}

/// What a user who only infers may know: no accounts, no active account, no
/// quota, and no account label.
fn for_inference_only(details: ProviderDetails) -> ProviderDetails {
    let auth = match details.auth {
        AuthState::Authenticated { .. } => AuthState::Authenticated {
            account_label: None,
        },
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

/// Whether the entry's models can be used now.
///
/// They cannot while the entry lacks a part it needs, all of which it names.
/// Otherwise the entry's health decides: a credential that is refused or a
/// provider that cannot run makes them unavailable.
pub fn availability(
    entry: &ProviderEntry,
    auth: &AuthState,
    runtime: &RuntimeState,
) -> Availability {
    let missing = entry.missing();
    if !missing.is_empty() {
        return Availability::setup(missing);
    }
    if matches!(
        auth,
        AuthState::Unauthenticated { .. } | AuthState::Error { .. }
    ) {
        return Availability::Unavailable {
            reason: UnavailableReason::Authentication,
            message: "Provider login is unavailable".into(),
            missing: None,
        };
    }
    match runtime {
        RuntimeState::Unavailable { message } | RuntimeState::Error { message } => {
            Availability::Unavailable {
                reason: UnavailableReason::Runtime,
                message: message.clone(),
                missing: None,
            }
        }
        RuntimeState::Unknown { .. } | RuntimeState::Ready { .. } => Availability::Available {},
    }
}
