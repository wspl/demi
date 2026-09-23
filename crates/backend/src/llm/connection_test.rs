//! The provider **Test** (`web-api.md` § Model configuration and provider
//! inspection): one real, minimal request to the model the user names, with
//! the account the user names or the active one. It runs on the acting
//! user's shard, which builds the runtime with its own HTTP client. A test
//! that ran and failed is a result carrying the provider's own reason.

use std::num::NonZeroU32;
use std::sync::Arc;

use demi_core::UserContentBlock;
use demi_provider::{InferenceItem, InferenceRequest, Provider, ProviderEvent, RuntimeEnv};
use demi_web_api::providers::TestResult;
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::catalog::configured_selection;
use crate::shard::Shard;
use crate::vault::entries::ProviderEntry;

impl Shard {
    /// Tests `provider`, the entry's provider for the account under test,
    /// with the entry's model `model_id`. `cancel` ends the test when the
    /// requester leaves.
    pub(crate) async fn test_provider(
        &self,
        entry: ProviderEntry,
        provider: Arc<dyn Provider>,
        model_id: String,
        cancel: CancellationToken,
    ) -> TestResult {
        let assembly = &self.services().assembly;
        let catalog = assembly.entry_catalog(&entry, &Ok(provider.clone()), false).await;
        let Some(model) = catalog.models.into_iter().find(|model| model.id == model_id) else {
            return TestResult::Failed {
                message: format!("This provider lists no model {model_id}"),
                model: None,
            };
        };
        let name = model.display_name.clone();
        let failed = |message: String| TestResult::Failed {
            message,
            model: Some(name.clone()),
        };
        // The model the catalog lists is also the configured one, when the
        // entry configures its list.
        let selection = match configured_selection(&entry, model.selection(entry.id.as_str(), None, None)) {
            Ok(selection) => selection,
            Err(error) => return failed(error.to_string()),
        };
        if provider.capabilities().process_host {
            return failed("This provider runs a process on the user's Cloud, which the test cannot reach yet".into());
        }
        let mut runtime = match provider.runtime(RuntimeEnv {
            http: self.http().clone(),
        }) {
            Ok(runtime) => runtime,
            Err(error) => return failed(error.to_string()),
        };
        let request_cancel = cancel.child_token();
        let request = InferenceRequest {
            session_id: "provider-test".into(),
            turn_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            model_id: model.id,
            output_limit: selection.model.output_limit.and_then(NonZeroU32::new),
            system_prompt: "Reply with the word ok.".into(),
            items: Arc::new([InferenceItem::UserMessage {
                content: vec![UserContentBlock::Text { text: "ping".into() }],
            }]),
            tools: Arc::new([]),
            thinking: None,
            service_tier_id: None,
            cancel: request_cancel.clone(),
        };
        // The first event decides: a failure is the provider's refusal, and
        // anything else shows that it answers.
        let first = {
            let mut run = runtime.run(request);
            let first = tokio::select! {
                () = cancel.cancelled() => None,
                first = run.next() => Some(first),
            };
            // Ends the run however it stands; dropping its stream is its
            // cancellation.
            request_cancel.cancel();
            first
        };
        runtime.close().await;
        match first {
            None => failed("The test was cancelled".into()),
            Some(None) => failed("The provider returned no events".into()),
            Some(Some(ProviderEvent::Error(failure))) => failed(failure.message),
            Some(Some(_)) => TestResult::Passed { model: name },
        }
    }
}
