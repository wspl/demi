//! The failure facts of a transcript's error blocks (`backend.md` § Failure
//! facts): each error block with a failure record is read by the provider
//! its model selection names, and the facts travel beside the blocks, never
//! stored. The conversation socket's transcript frames and the history
//! route both read them here. A usage limit of a subscription entry with
//! several accounts is read against them too (`providers.md` § Account
//! rotation): under rotation its moment is the earliest one among the
//! accounts once every one is used up, and without rotation the dock
//! offers the account with the most room now.

use std::collections::HashMap;
use std::sync::Arc;

use demi_conversation_socket_protocol::Failures;
use demi_provider_common::Provider;
use demi_shared_types::{Block, ContinueWith, ErrorBlock, ProviderFailureFacts, Rotation};
use demi_web_api_protocol::ids::ProviderId;

use demi_backend_providers::llm::assembly::ProviderAssembly;
use demi_backend_providers::vault::entries::ProviderEntry;

/// A used-up account's failure code.
const USAGE_LIMIT: &str = "usage_limit";

/// The facts of the error blocks among `blocks`, by block id; none when no
/// block yields one. A block whose provider entry is gone, or cannot be
/// read, shows without facts.
pub async fn failure_facts(assembly: &ProviderAssembly, blocks: &[Block]) -> Option<Failures> {
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
        let used_up = error.code.as_deref() == Some(USAGE_LIMIT);
        if upstream.is_none() && !used_up {
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
            },
        };
        if used_up {
            account_facts(assembly, &reader.entry, error, &mut facts).await;
        }
        if facts.retry_at.is_some() || facts.continue_with.is_some() {
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
