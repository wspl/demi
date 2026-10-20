//! A small request (`crates/backend-providers/src/llm/small_model.rs`): one request with a fixed
//! instruction as its system prompt and one user message, for a small job
//! that serves a conversation rather than answers it, such as its title. It
//! goes to the runtime the caller supplies, the metered one a turn uses, and
//! is no session turn: nothing is added to a transcript. The caller resolves
//! the small model and its thinking; the job gives the instruction, the
//! message and its output limit.

use std::{num::NonZeroU32, sync::Arc};

use demi_provider_common::{
    InferenceItem, InferenceRequest, PromptCache, ProviderEvent, ProviderRuntime, RequestPurpose, UserPart,
};
use demi_shared_types::ModelSelection;
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

/// What a small job asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallRequest {
    /// The whole system prompt.
    pub instruction: &'static str,
    /// The one user message.
    pub message: String,
    /// The request's cap on its output; a model whose own output limit is
    /// lower sends that one (`crates/provider-common/src/contract.rs`).
    pub output_cap: NonZeroU32,
    /// What the usage ledger counts the request under.
    pub purpose: RequestPurpose,
}

/// Why a small request answered nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SmallRequestError {
    /// The provider refused or failed the request.
    #[error("{0}")]
    Failed(String),
}

/// Runs `request` for the conversation `session` on `runtime` with the
/// small model `selection` and its thinking, and answers the text of the
/// response; thinking output is ignored. A request that `cancel` stops is
/// none.
pub async fn run_small_request(
    runtime: &mut dyn ProviderRuntime,
    session: &str,
    request_id: String,
    selection: &ModelSelection,
    request: SmallRequest,
    cancel: CancellationToken,
) -> Result<Option<String>, SmallRequestError> {
    let inference = InferenceRequest {
        session_id: session.to_owned(),
        affinity: None,
        turn_id: format!("{}:{request_id}", request.purpose.as_str()),
        request_id,
        model_id: selection.model.id.clone(),
        output_limit: selection.model.output_limit.and_then(NonZeroU32::new),
        output_cap: Some(request.output_cap),
        system_prompt: request.instruction.to_owned(),
        items: Arc::from([InferenceItem::UserMessage {
            content: vec![UserPart::Text(request.message)],
        }]),
        blocks: Arc::from([]),
        tools: Arc::from([]),
        thinking: selection.thinking.clone(),
        service_tier_id: selection.service_tier_id.clone(),
        // A small request is no session's: nothing extends it.
        prompt_cache: PromptCache::Off,
        cancel: cancel.clone(),
        // No agent's: the entry's rotation picks one for this request alone.
        account: None,
        purpose: request.purpose,
        agent: None,
    };
    let mut events = runtime.run(inference);
    let mut answer = String::new();
    while let Some(event) = events.next().await {
        match event {
            ProviderEvent::TextDelta(text) => answer.push_str(&text),
            ProviderEvent::Error(failure) => return Err(SmallRequestError::Failed(failure.message)),
            _ => {}
        }
    }
    if cancel.is_cancelled() {
        return Ok(None);
    }
    Ok(Some(answer))
}
