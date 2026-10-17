//! The provider **Test** (`web-api-settings.md` § Model configuration and provider
//! inspection): one real, minimal request to the model the user names, with
//! the account the user names or the active one. It runs on the acting
//! user's shard, which builds the runtime with its own HTTP client, or over
//! the acting user's Cloud for a provider that runs a process. A test that
//! ran and failed is a result carrying the provider's own reason.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;

use demi_backend_providers::llm::catalog::configured_selection;
use demi_backend_providers::vault::entries::ProviderEntry;
use demi_provider_common::{
    ErrorCode, InferenceItem, InferenceRequest, PromptCache, Provider, ProviderEvent, RequestPurpose, RuntimeEnv, UserPart,
};
use demi_provider_common::media::MediaFailure;
use demi_shared_types::MediaKind;
use demi_web_api_protocol::ids::CredentialId;
use demi_web_api_protocol::providers::TestResult;
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::claude_cli::{CrossPlacement, ProcessWork};
use crate::shard::exposed_ports::endpoint_device;
use crate::shard::Shard;

impl Shard {
    /// Tests the key of an entry of media alone, which makes no text, by
    /// reading its vendor's list of its media models, as ElevenLabs'
    /// `GET /v1/models` checks a key (`providers.md` § Media-only vendors).
    /// It needs only the key: a passed test names the first model the
    /// listing holds, or the entry when it holds none, and a refused key
    /// fails in the vendor's words.
    async fn test_media_key(&self, entry: &ProviderEntry, provider: &Arc<dyn Provider>, cancel: CancellationToken) -> TestResult {
        let services = self.services();
        let assembly = &services.assembly;
        let shapes = assembly.media_shapes(entry);
        let maker = MediaKind::ALL
            .into_iter()
            .filter(|kind| shapes.makes(*kind))
            .find_map(|kind| assembly.maker(entry, provider, kind));
        let Some(maker) = maker else {
            return TestResult::Failed { message: "This provider makes nothing yet".into(), model: None, not_exposed: None };
        };
        let http = assembly.http_for(entry, self.http().clone());
        let catalog = services.media.current();
        let listing = tokio::select! {
            () = cancel.cancelled() => {
                return TestResult::Failed { message: "The test was cancelled".into(), model: None, not_exposed: None };
            }
            listing = maker.media().models(&http, &catalog) => listing,
        };
        match listing {
            Ok(models) => {
                let first = models.into_iter().next().map(|model| model.name.unwrap_or(model.id));
                TestResult::Passed { model: first.unwrap_or_else(|| entry.label.clone()) }
            }
            Err(failure) => TestResult::Failed { message: failure_words(&failure), model: None, not_exposed: None },
        }
    }

    /// Tests `provider`, the entry's provider for `account` or for its
    /// active account without one, with the entry's model `model_id`. A
    /// provider that runs a process runs it on the Claude Code machine. `cancel`
    /// ends the test when the requester leaves.
    pub async fn test_provider(
        &self,
        entry: ProviderEntry,
        provider: Arc<dyn Provider>,
        account: Option<CredentialId>,
        model_id: Option<String>,
        cancel: CancellationToken,
    ) -> TestResult {
        let assembly = &self.services().assembly;
        if !assembly.makes_text(&entry) {
            return self.test_media_key(&entry, &provider, cancel).await;
        }
        let Some(model_id) = model_id else {
            return TestResult::Failed { message: "Name a text model to test".into(), model: None, not_exposed: None };
        };
        let catalog = assembly
            .entry_catalog(&entry, &Ok(provider.clone()), false)
            .await;
        let Some(model) = catalog
            .models
            .into_iter()
            .find(|model| model.id == model_id)
        else {
            return TestResult::Failed {
                message: format!("This provider lists no model {model_id}"),
                model: None,
                not_exposed: None,
            };
        };
        let name = model.display_name.clone();
        let failed = |message: String| TestResult::Failed {
            message,
            model: Some(name.clone()),
            not_exposed: None,
        };
        // The model the catalog lists is also the configured one, when the
        // entry configures its list.
        let selection =
            match configured_selection(&entry, model.selection(entry.id.as_str(), None, None)) {
                Ok(selection) => selection,
                Err(error) => return failed(error.to_string()),
            };
        let runtime = if provider.capabilities().process_host {
            let work = ProcessWork::Account(entry.id.clone());
            let placement = CrossPlacement::placement(Rc::downgrade(&self.this()), work);
            let account = account.as_ref().or(entry.active());
            let env = RuntimeEnv {
                http: assembly.http_for(&entry, self.http().clone()),
            };
            assembly
                .process_runtime(&entry, account, env, placement)
                .await
                .map_err(|error| error.to_string())
        } else {
            let env = RuntimeEnv {
                http: assembly.http_for(&entry, self.http().clone()),
            };
            provider.runtime(env).map_err(|error| error.to_string())
        };
        let mut runtime = match runtime {
            Ok(runtime) => runtime,
            Err(message) => return failed(message),
        };
        let request_cancel = cancel.child_token();
        let request = InferenceRequest {
            session_id: "provider-test".into(),
            turn_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            model_id: model.id,
            output_limit: selection.model.output_limit.and_then(NonZeroU32::new),
            output_cap: None,
            system_prompt: "Reply with the word ok.".into(),
            items: Arc::new([InferenceItem::UserMessage {
                content: vec![UserPart::Text("ping".into())],
            }]),
            blocks: Arc::new([]),
            tools: Arc::new([]),
            thinking: None,
            service_tier_id: None,
            // A connection test is no session's: nothing extends it.
            prompt_cache: PromptCache::Off,
            cancel: request_cancel.clone(),
            account: None,
            // A connection test is no conversation's, and no ledger meters
            // its runtime.
            purpose: RequestPurpose::Turn,
            agent: None,
        };
        // The first event that is not the usage ledger's accounting decides:
        // a failure is the provider's refusal, and anything else shows that
        // it answers.
        let first = {
            let mut run = runtime.run(request);
            let judged = async {
                loop {
                    match run.next().await {
                        Some(ProviderEvent::Call(_)) => {}
                        next => break next,
                    }
                }
            };
            let first = tokio::select! {
                () = cancel.cancelled() => None,
                first = judged => Some(first),
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
            Some(Some(ProviderEvent::Error(failure))) => {
                let not_exposed = match failure.code {
                    // Only the owner's device can be exposed by the one who tests.
                    Some(ErrorCode::PortNotExposed) if entry.owner == *self.user() => {
                        endpoint_device(&self.services().control, &entry).await
                    }
                    _ => None,
                };
                TestResult::Failed {
                    message: failure.message,
                    model: Some(name),
                    not_exposed,
                }
            }
            Some(Some(_)) => TestResult::Passed { model: name },
        }
    }
}

/// What a media request's failure says, as a test shows it: the vendor's
/// own words where it gave some.
fn failure_words(failure: &MediaFailure) -> String {
    match failure {
        MediaFailure::UsageLimit { .. } => "The usage limit is reached".to_owned(),
        MediaFailure::SignIn(failure) => failure.message(),
        MediaFailure::KeyRefused { message, .. } | MediaFailure::Refused { message, .. } => message.clone(),
        MediaFailure::Shape { takes, .. } => format!("It takes only {}", takes.join(", ")),
        MediaFailure::Unanswered { cause, .. } => cause.clone(),
    }
}
