//! The failure facts of a transcript's error blocks: each error block with a
//! failure record is read by the provider its model selection names, and the
//! facts travel beside the blocks, never stored. The conversation socket's
//! transcript frames and the history route both read them here. A usage limit
//! of a subscription entry with several accounts is read against them too
//! (`crates/backend-providers/src/llm/rotation.rs`): under rotation its moment
//! is the earliest one among the accounts once every one is used up, and
//! without rotation the dock offers the account with the most room now. A
//! request to a port its device had not exposed names the port, which the page
//! offers to expose or to try again, and one to an offline device names the
//! device.

use std::collections::HashMap;
use std::sync::Arc;

use demi_conversation_socket_protocol::Failures;
use demi_provider_common::Provider;
use demi_provider_common::ErrorCode;
use demi_shared_types::{
    Block, ContinueWith, ErrorBlock, OfflineDevice, ProviderFailureFacts, Rotation, SignedOutAccount, Timestamp,
};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};

use demi_backend_providers::llm::assembly::ProviderAssembly;
use demi_backend_providers::vault::entries::ProviderEntry;

use crate::shard::exposed_ports::endpoint_device;

/// The facts of the error blocks among `blocks`, by block id, as `viewer`
/// reads them; none when no block yields one. A block whose provider entry
/// is gone, or cannot be read, shows without facts. A port to expose is a
/// fact only for the entry's owner, whose device it is.
pub async fn failure_facts(assembly: &ProviderAssembly, viewer: &UserId, blocks: &[Block]) -> Option<Failures> {
    let mut failures = Failures::new();
    let mut readers: HashMap<String, Option<Reader>> = HashMap::new();
    for block in blocks {
        let Block::Error(error) = block else {
            continue;
        };
        let upstream = error
            .diagnostics
            .as_ref()
            .filter(|diagnostics| diagnostics.upstream.is_some());
        let used_up = error.code.as_deref() == Some(ErrorCode::UsageLimit.as_str());
        let refused = error.code.as_deref() == Some(ErrorCode::AuthExpired.as_str());
        let unexposed = error.code.as_deref() == Some(ErrorCode::PortNotExposed.as_str());
        let offline = error.code.as_deref() == Some(ErrorCode::DeviceOffline.as_str());
        if upstream.is_none() && !used_up && !refused && !unexposed && !offline {
            continue;
        }
        let provider = &error.model.provider_id;
        if !readers.contains_key(provider) {
            let reader = reader(assembly, provider).await;
            readers.insert(provider.clone(), reader);
        }
        let Some(Some(reader)) = readers.get(provider) else {
            continue;
        };
        let mut facts = match upstream {
            Some(diagnostics) => reader.provider.read_failure(diagnostics, error.created_at),
            None => ProviderFailureFacts {
                retry_at: None,
                continue_with: None,
                signed_out: None,
                not_exposed: None,
                device_offline: None,
            },
        };
        if used_up {
            if facts.retry_at.is_none() {
                facts.retry_at = quota_moment(assembly, reader, error).await;
            }
            account_facts(assembly, &reader.entry, error, &mut facts).await;
        }
        if refused {
            facts.signed_out = signed_out(assembly, &reader.entry, error).await;
        }
        if unexposed && reader.entry.owner == *viewer {
            facts.not_exposed = endpoint_device(assembly.vault().control(), &reader.entry).await;
        }
        if offline {
            facts.device_offline = endpoint_device(assembly.vault().control(), &reader.entry)
                .await
                .map(|named| OfflineDevice {
                    device_id: named.device_id,
                    device: named.device,
                });
        }
        if facts.retry_at.is_some()
            || facts.continue_with.is_some()
            || facts.signed_out.is_some()
            || facts.not_exposed.is_some()
            || facts.device_offline.is_some()
        {
            failures.insert(error.id.clone(), facts);
        }
    }
    (!failures.is_empty()).then_some(failures)
}

/// The entry a failure names and its provider.
struct Reader {
    entry: ProviderEntry,
    provider: Arc<dyn Provider>,
}

/// A usage limit's moment when its failure names none: the latest reset
/// among the windows the quota of the account the failing request used
/// shows used up, the account the error block names under rotation, else
/// the active one.
async fn quota_moment(
    assembly: &ProviderAssembly,
    reader: &Reader,
    error: &ErrorBlock,
) -> Option<Timestamp> {
    let named = error
        .account
        .as_deref()
        .and_then(|account| CredentialId::try_from(account).ok());
    let provider = match named {
        Some(account) => match assembly.for_account(&reader.entry, &account).await {
            Ok(provider) => provider?,
            Err(failure) => {
                tracing::warn!(provider = %reader.entry.id, error = &failure as &dyn std::error::Error, "a usage limit's account was not read");
                return None;
            }
        },
        None => reader.provider.clone(),
    };
    provider.quota()?.used_up_until(error.created_at)
}

/// What a usage limit of `entry` adds from its accounts: under rotation,
/// the earliest moment among them once every one is used up; without it,
/// the account with the most room now, other than the active one that
/// failed. A read that fails adds nothing: the facts only add to the
/// failure.
async fn account_facts(
    assembly: &ProviderAssembly,
    entry: &ProviderEntry,
    error: &ErrorBlock,
    facts: &mut ProviderFailureFacts,
) {
    let candidates = match assembly.candidates(entry, false).await {
        Ok(candidates) if candidates.len() >= 2 => candidates,
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(provider = %entry.id, error = &error as &dyn std::error::Error, "a usage limit's accounts were not read");
            return;
        }
    };
    let rotation = assembly.rotation();
    let model = &error.model.model;
    let now = assembly.now();
    match entry.rotation() {
        Rotation::Manual => {
            let others: Vec<_> = candidates
                .into_iter()
                .filter(|candidate| Some(&candidate.id) != entry.active())
                .collect();
            facts.continue_with = rotation
                .most_room(&entry.id, &others, model, now)
                .and_then(|id| others.into_iter().find(|candidate| candidate.id == id))
                .map(|candidate| ContinueWith {
                    credential_id: candidate.id.to_string(),
                    label: candidate.label,
                });
        }
        Rotation::InOrder | Rotation::Balanced => {
            if let Some(earliest) = rotation.all_used_up(&entry.id, &candidates, model, now) {
                facts.retry_at = earliest.or(facts.retry_at);
            }
        }
    }
}

/// For a refused sign-in, the account the request used, the one the error
/// block names, else the entry's active one, while its vendor has it
/// signed out (`crates/provider-claude-code/src/account.rs`): an account signed
/// in again since shows the failure as it was. A read that fails adds
/// nothing.
async fn signed_out(
    assembly: &ProviderAssembly,
    entry: &ProviderEntry,
    error: &ErrorBlock,
) -> Option<SignedOutAccount> {
    let named = error
        .account
        .as_deref()
        .and_then(|account| CredentialId::try_from(account).ok());
    let account = named.or_else(|| entry.active().cloned())?;
    let record = match assembly.vault().account(entry.id.clone(), account).await {
        Ok(record) => record?,
        Err(failure) => {
            tracing::warn!(provider = %entry.id, error = &failure as &dyn std::error::Error, "a refused sign-in's account was not read");
            return None;
        }
    };
    record.signed_out.then(|| SignedOutAccount {
        provider: entry.label.clone(),
        account: record.label,
    })
}

/// The entry `provider` and its provider. Facts only add to what the web
/// app receives anyway, so an entry that is gone or cannot be read yields
/// no reader rather than holding back the transcript.
async fn reader(assembly: &ProviderAssembly, provider: &str) -> Option<Reader> {
    let id = ProviderId::try_from(provider).ok()?;
    let entry = match assembly.vault().entry(id).await {
        Ok(entry) => entry?,
        Err(error) => {
            tracing::warn!(
                provider,
                error = &error as &dyn std::error::Error,
                "a failure's provider entry was not read"
            );
            return None;
        }
    };
    match assembly.provider_for(&entry).await {
        Ok(provider) => Some(Reader { entry, provider }),
        Err(error) => {
            tracing::warn!(provider = %entry.id, error = &error as &dyn std::error::Error, "a failure's provider was not built");
            None
        }
    }
}
