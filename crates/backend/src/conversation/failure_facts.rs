//! The failure facts of a transcript's error blocks (`backend.md` § Failure
//! facts): each error block with a failure record is read by the provider
//! its model selection names, and the facts travel beside the blocks, never
//! stored. The conversation socket's transcript frames and the history
//! route both read them here.

use std::collections::HashMap;
use std::sync::Arc;

use demi_agent_protocol::Failures;
use demi_core::Block;
use demi_provider::Provider;
use demi_web_api::ids::ProviderId;

use crate::llm::assembly::ProviderAssembly;

/// The facts of the error blocks among `blocks`, by block id; none when no
/// block yields one. A block whose provider entry is gone, or cannot be
/// read, shows without facts.
pub(crate) async fn failure_facts(assembly: &ProviderAssembly, blocks: &[Block]) -> Option<Failures> {
    let mut failures = Failures::new();
    let mut readers: HashMap<String, Option<Arc<dyn Provider>>> = HashMap::new();
    for block in blocks {
        let Block::Error(error) = block else {
            continue;
        };
        let Some(diagnostics) = error.diagnostics.as_ref().filter(|diagnostics| diagnostics.upstream.is_some()) else {
            continue;
        };
        let provider = &error.model.provider_id;
        if !readers.contains_key(provider) {
            let reader = reader(assembly, provider).await;
            readers.insert(provider.clone(), reader);
        }
        if let Some(Some(reader)) = readers.get(provider) {
            failures.insert(error.id.clone(), reader.read_failure(diagnostics, error.created_at));
        }
    }
    (!failures.is_empty()).then_some(failures)
}

/// The provider of the entry `provider`. Facts only add to what the browser
/// receives anyway, so an entry that is gone or cannot be read yields no
/// reader rather than holding back the transcript.
async fn reader(assembly: &ProviderAssembly, provider: &str) -> Option<Arc<dyn Provider>> {
    let id = ProviderId::try_from(provider).ok()?;
    let entry = match assembly.vault().entry(id).await {
        Ok(entry) => entry?,
        Err(error) => {
            tracing::warn!(provider, error = &error as &dyn std::error::Error, "a failure's provider entry was not read");
            return None;
        }
    };
    match assembly.provider_for(&entry).await {
        Ok(provider) => Some(provider),
        Err(error) => {
            tracing::warn!(provider = %entry.id, error = &error as &dyn std::error::Error, "a failure's provider was not built");
            None
        }
    }
}
