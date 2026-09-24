//! The metered runtime (`providers.md` § Inference admission and runtime
//! ownership): the backend wraps each session's runtime so that the user's
//! rate limit applies before every run and the usage of every response
//! reaches the ledger before the agent sees the response
//! (`usage-and-quota.md` § Usage ledger). A ledger write that fails is
//! logged and the request goes on: inference never fails because
//! accounting did.

use std::cell::RefCell;
use std::rc::Rc;

use demi_core::TokenUsage;
use demi_provider::{ErrorCode, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun, ProviderRuntime};
use demi_web_api::ids::{ConversationId, ProviderId, UserId};
use futures_util::future::LocalBoxFuture;
use futures_util::{StreamExt, stream};

use super::rate_limit::RequestRateLimit;
use crate::storage::control::ControlService;
use crate::storage::usage::UsageRow;

/// Whose requests a runtime makes: the ledger rows' user, conversation and
/// entry.
#[derive(Clone)]
pub(crate) struct Ledger {
    pub(crate) control: ControlService,
    pub(crate) user: UserId,
    pub(crate) conversation: ConversationId,
    pub(crate) provider: ProviderId,
}

impl Ledger {
    /// Writes the row of one response and waits for it.
    async fn record(&self, model: String, usage: TokenUsage) {
        let row = UsageRow {
            user: self.user.clone(),
            conversation: self.conversation.clone(),
            provider: self.provider.clone(),
            model,
            usage,
        };
        if let Err(error) = self.control.append_usage(row).await {
            // The row is lost; the request goes on (`usage-and-quota.md`).
            tracing::warn!(
                error = &error as &dyn std::error::Error,
                "a usage ledger row was not written"
            );
        }
    }
}

/// A session's runtime with the user's rate limit and the ledger around it.
/// Its forks count against the same limit and write to the same ledger.
pub(crate) struct MeteredRuntime {
    inner: Box<dyn ProviderRuntime>,
    /// The user's window, which the user's shard holds.
    rate_limit: Rc<RefCell<RequestRateLimit>>,
    ledger: Rc<Ledger>,
}

impl MeteredRuntime {
    pub(crate) fn new(
        inner: Box<dyn ProviderRuntime>,
        rate_limit: Rc<RefCell<RequestRateLimit>>,
        ledger: Ledger,
    ) -> Self {
        Self {
            inner,
            rate_limit,
            ledger: Rc::new(ledger),
        }
    }
}

impl ProviderRuntime for MeteredRuntime {
    /// A request over the limit never reaches the vendor: its run fails with
    /// `rate_limited`, which the agent does not retry by itself.
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let admitted = self.rate_limit.borrow_mut().take();
        if let Err(limited) = admitted {
            let refusal = ProviderEvent::Error(ProviderFailure {
                message: limited.to_string(),
                code: Some(ErrorCode::RateLimited),
                diagnostics: None,
                retry_after: None,
            });
            return stream::iter([refusal]).boxed_local();
        }
        let model = request.model_id.clone();
        let ledger = self.ledger.clone();
        self.inner
            .run(request)
            .then(move |event| {
                let ledger = ledger.clone();
                let model = model.clone();
                async move {
                    if let ProviderEvent::Response(usage) = &event {
                        ledger.record(model, *usage).await;
                    }
                    event
                }
            })
            .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self {
            inner: self.inner.fresh(),
            rate_limit: self.rate_limit.clone(),
            ledger: self.ledger.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        self.inner.close()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use demi_provider::testing::{ScriptedRuntime, Turn, event, inference_request};
    use futures_util::future::join_all;

    use super::*;
    use crate::storage::control::testing;

    #[tokio::test(start_paused = true)]
    async fn answered_requests_are_ledger_rows_before_their_response_and_the_limit_refuses_the_rest() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
            .await
            .unwrap();
        let master = testing::master(&control).await;
        let ledger = Ledger {
            control: control.clone(),
            user: master.id.clone(),
            conversation: ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap(),
            provider: ProviderId::try_from("entry-1").unwrap(),
        };
        let script = ScriptedRuntime::new([
            Turn::Events(vec![event::text("a"), event::response(100, 10)]),
            Turn::Events(vec![event::response(101, 10)]),
            Turn::Events(vec![event::response(102, 10)]),
            Turn::Events(vec![event::response(103, 10)]),
        ]);
        let limit = Rc::new(RefCell::new(RequestRateLimit::new(3)));
        let mut first = MeteredRuntime::new(Box::new(script.clone()), limit, ledger);
        let requests = || async { control.usage_totals(master.id.clone()).await.unwrap() };

        // The row exists by the time the response reaches the caller.
        let mut run = first.run(inference_request());
        assert_eq!(run.next().await, Some(event::text("a")));
        assert!(requests().await.is_empty());
        assert_eq!(run.next().await, Some(event::response(100, 10)));
        assert_eq!(requests().await[0].requests, 1);
        drop(run);

        // Three runs of the runtime and its forks start together: two fit the
        // window, and the third is refused before it reaches the vendor.
        let mut forks: Vec<Box<dyn ProviderRuntime>> = (0..3).map(|_| first.fresh()).collect();
        let outcomes = join_all(
            forks
                .iter_mut()
                .map(|runtime| runtime.run(inference_request()).collect::<Vec<_>>()),
        )
        .await;
        let refused: Vec<&Vec<ProviderEvent>> = outcomes
            .iter()
            .filter(|events| matches!(events.as_slice(), [ProviderEvent::Error(failure)] if failure.code == Some(ErrorCode::RateLimited)))
            .collect();
        assert_eq!(refused.len(), 1, "{outcomes:?}");
        assert_eq!(script.remaining(), 1, "a refused request never reaches the vendor");
        let totals = requests().await;
        assert_eq!(
            (totals[0].requests, totals[0].input_tokens, totals[0].output_tokens),
            (3, 303, 30)
        );
        assert_eq!(
            (totals[0].provider_id.as_str(), totals[0].model_id.as_str()),
            ("entry-1", "model-1")
        );
        control.close().await.unwrap();
    }
}
