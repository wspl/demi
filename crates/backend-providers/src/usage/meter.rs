//! The metered runtime
//! (`crates/backend-user-shard/src/conversation/providers.rs`): the backend
//! wraps each session's runtime so that accounting happens around every run.
//!
//! - The user's rate limit applies before every run.
//! - Every call the run makes to the vendor is a usage ledger row as it ends,
//!   the run's last before the run's response reaches the agent
//!   (`crates/backend-database/src/usage.rs`).
//!
//! A ledger write that fails is logged and the request goes on: inference never
//! fails because accounting did. Each run logs under a span that names its
//! request, its agent and its account, so a row finds its course in the server
//! log.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use demi_backend_database::control::ControlService;
use demi_backend_database::usage::{ApiPrice, ObservedWindow, UsageRow, merge_windows};
use demi_provider_common::{
    Ahead, CallEvent, CallOutcome, ErrorCode, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun,
    ProviderRuntime, RequestLimits, RequestPurpose, VendorCall,
};
use demi_shared_types::{Clock, Model, ModelCost, QuotaWindow, Timestamp, TokenUsage};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use futures_util::future::LocalBoxFuture;
use futures_util::{Stream, StreamExt, stream};
use sha2::{Digest, Sha256};

use super::rate_limit::RequestRateLimit;

/// Whose calls a runtime makes: the ledger rows' user, conversation, entry
/// and account.
#[derive(Clone)]
pub struct Ledger {
    pub control: ControlService,
    pub clock: Arc<dyn Clock>,
    pub user: UserId,
    pub conversation: ConversationId,
    pub provider: ProviderId,
    /// The subscription account the runtime calls with; none for an API key.
    pub account: Option<CredentialId>,
    /// Looks up the catalog's prices of a model as each of its rows is
    /// written, so a price the catalog learns or corrects applies from the
    /// next call (`crates/backend-database/src/usage.rs`).
    pub prices: ModelPrices,
}

/// The catalog's prices of the model named, at the moment it is asked;
/// none when the catalog has none.
pub type ModelPrices = Rc<dyn Fn(String) -> LocalBoxFuture<'static, Option<ModelCost>>>;

/// The first 16 hex digits of the SHA-256 of `text`: enough to tell two
/// prompts apart, as the ledger's hashes do.
fn short_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    hex::encode(&digest[..8])
}

/// The hashes of the system prompt and of the tool definitions `request`
/// sends; none for a request outside a session (`crates/backend-database/src/usage.rs`).
fn prompt_hashes(request: &InferenceRequest) -> (Option<String>, Option<String>) {
    if request.agent.is_none() {
        return (None, None);
    }
    let tools: Vec<serde_json::Value> = request
        .tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            })
        })
        .collect();
    (
        Some(short_hash(&request.system_prompt)),
        Some(short_hash(&serde_json::Value::Array(tools).to_string())),
    )
}

/// Writes `row` and answers its id; a failure is logged and the row is
/// lost.
async fn write(control: &ControlService, row: UsageRow) -> Option<String> {
    match control.append_usage(row).await {
        Ok(id) => Some(id),
        Err(error) => {
            // The row is lost; the request goes on (`crates/backend-providers/src/usage/mod.rs`).
            tracing::warn!(
                error = &error as &dyn std::error::Error,
                "a usage ledger row was not written"
            );
            None
        }
    }
}

/// A session's runtime with the user's rate limit and the ledger around it.
/// Its forks count against the same limit and write to the same ledger.
pub struct MeteredRuntime {
    inner: Box<dyn ProviderRuntime>,
    /// The user's window, which the user's shard holds.
    rate_limit: Rc<RefCell<RequestRateLimit>>,
    ledger: Rc<Ledger>,
}

impl MeteredRuntime {
    pub fn new(
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

/// How a call ended, as its row says.
enum Outcome {
    Answered,
    Refused(Option<ErrorCode>),
    Cancelled,
}

impl Outcome {
    fn text(&self) -> String {
        match self {
            Self::Answered => "answered".to_owned(),
            Self::Cancelled => "cancelled".to_owned(),
            Self::Refused(Some(code)) => code.as_str().to_owned(),
            // A failure without a code, such as a stream that broke off.
            Self::Refused(None) => "failed".to_owned(),
        }
    }
}

/// The calls of one run: what each row says of the request, and the call
/// open now, which started with the run or at its `Started` and which its
/// `Ended` or the run's end ends. A run dropped with a call open was
/// cancelled, and that call's row is written then.
struct Calls {
    ledger: Rc<Ledger>,
    agent: Option<u64>,
    turn: Option<String>,
    purpose: RequestPurpose,
    model: String,
    request: String,
    system_hash: Option<String>,
    tools_hash: Option<String>,
    /// When the open call started; none between a call's end and the next
    /// one's start.
    open: Option<Timestamp>,
    /// The windows the open call's response reported.
    observed: Vec<ObservedWindow>,
    /// The row of the call that ended last, which a window the vendor
    /// reports before the next call starts is added to.
    last_row: Option<String>,
    /// The run's span, which a dropped run's last line logs under.
    span: tracing::Span,
}

impl Calls {
    fn new(ledger: Rc<Ledger>, request: &InferenceRequest, span: tracing::Span) -> Self {
        let open = Some(ledger.clock.now());
        let (system_hash, tools_hash) = prompt_hashes(request);
        Self {
            system_hash,
            tools_hash,
            agent: request.agent,
            // A title request belongs to no turn.
            turn: (request.purpose != RequestPurpose::Title).then(|| request.turn_id.clone()),
            purpose: request.purpose,
            model: request.model_id.clone(),
            request: request.request_id.clone(),
            ledger,
            open,
            observed: Vec::new(),
            last_row: None,
            span,
        }
    }

    fn start(&mut self) {
        if self.open.is_none() {
            self.open = Some(self.ledger.clock.now());
            self.last_row = None;
        }
    }

    /// The vendor reported `windows`: the open call's, else the one that
    /// ended last, whose row gains them.
    async fn observe(&mut self, windows: Vec<QuotaWindow>) {
        if self.open.is_some() {
            merge_windows(&mut self.observed, &windows);
            return;
        }
        let Some(id) = self.last_row.clone() else {
            return;
        };
        if let Err(error) = self.ledger.control.add_usage_quota(id, windows).await {
            // The windows are lost from the row; the request goes on.
            tracing::warn!(
                error = &error as &dyn std::error::Error,
                "a usage ledger row's quota was not written"
            );
        }
    }

    /// Writes `row` and keeps its id for windows reported after it.
    async fn record(&mut self, mut row: UsageRow) {
        // A call without counts has nothing to price.
        if let Some(usage) = row.usage {
            let prices = (self.ledger.prices)(row.model.clone()).await;
            row.price = prices.and_then(|prices| ApiPrice::of(&prices, &usage, row.cache_write_1h_tokens));
        }
        self.last_row = write(&self.ledger.control, row).await;
    }

    /// The row of the call open now, ended as `outcome` says with the
    /// vendor's counts and id for it, none when it reported none; a call
    /// the run reports without its start, such as a refusal after the last
    /// call ended, starts as it ends.
    fn row(&mut self, call: VendorCall, outcome: Outcome) -> UsageRow {
        let ended_at = self.ledger.clock.now();
        let started_at = self.open.take().unwrap_or(ended_at);
        let VendorCall {
            vendor_id,
            usage,
            cache_write_1h_tokens,
            ..
        } = call;
        let outcome = outcome.text();
        tracing::info!(
            outcome,
            vendor_id = vendor_id.as_deref(),
            input_tokens = usage.map(|usage| usage.input_tokens),
            output_tokens = usage.map(|usage| usage.output_tokens),
            cache_read_tokens = usage.map(|usage| usage.cache_read_tokens),
            cache_write_tokens = usage.map(|usage| usage.cache_write_tokens),
            "a vendor call ended"
        );
        let observed = std::mem::take(&mut self.observed);
        UsageRow {
            user: self.ledger.user.clone(),
            conversation: self.ledger.conversation.clone(),
            agent: self.agent,
            turn: self.turn.clone(),
            purpose: self.purpose.as_str(),
            provider: self.ledger.provider.clone(),
            account: self.ledger.account.clone(),
            model: self.model.clone(),
            request: self.request.clone(),
            vendor_id,
            usage,
            cache_write_1h_tokens,
            outcome,
            quota: (!observed.is_empty()).then_some(observed),
            started_at,
            ended_at,
            media_seconds: None,
            cost: None,
            // Priced as it is written.
            price: None,
            system_hash: self.system_hash.clone(),
            tools_hash: self.tools_hash.clone(),
        }
    }

    /// The run reported the end of its latest call.
    async fn ended(&mut self, call: VendorCall) {
        let outcome = match &call.outcome {
            CallOutcome::Answered => Outcome::Answered,
            CallOutcome::Refused(code) => Outcome::Refused(code.clone()),
        };
        let row = self.row(call, outcome);
        self.record(row).await;
    }

    /// The run's end ends the call open, if any: with the response's usage,
    /// the failure's code, or, cancelled, with none.
    async fn close(&mut self, usage: Option<TokenUsage>, outcome: Outcome) {
        if self.open.is_none() {
            return;
        }
        let row = self.row(unreported(usage), outcome);
        self.record(row).await;
    }
}

/// A call the run did not report on its own, with the usage its end
/// carries: no vendor id and no hour-long cache write.
fn unreported(usage: Option<TokenUsage>) -> VendorCall {
    VendorCall {
        vendor_id: None,
        usage,
        cache_write_1h_tokens: 0,
        // The row's outcome is the run end's.
        outcome: CallOutcome::Answered,
    }
}

impl Drop for Calls {
    /// A run dropped with a call open was cancelled before the vendor
    /// answered; its row is written on its own, since nothing awaits the
    /// run any more.
    fn drop(&mut self) {
        if self.open.is_none() {
            return;
        }
        let span = self.span.clone();
        let row = span.in_scope(|| self.row(unreported(None), Outcome::Cancelled));
        let control = self.ledger.control.clone();
        tokio::spawn(async move {
            write(&control, row).await;
        });
    }
}

/// `run` polled inside `span`, so every line its provider logs carries the
/// span's fields.
fn within<'a>(span: tracing::Span, run: impl Stream<Item = ProviderEvent> + 'a) -> ProviderRun<'a> {
    let mut run = Box::pin(run);
    stream::poll_fn(move |context| {
        let _entered = span.enter();
        run.as_mut().poll_next(context)
    })
    .boxed_local()
}

impl ProviderRuntime for MeteredRuntime {
    /// A request over the limit never reaches the vendor: its run fails with
    /// `rate_limited`, which the agent does not retry by itself, and it has
    /// no row.
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
        let span = tracing::info_span!(
            "provider_call",
            request = %request.request_id,
            agent = tracing::field::Empty,
            account = tracing::field::Empty,
            purpose = request.purpose.as_str(),
            provider = %self.ledger.provider,
            model = %request.model_id,
        );
        if let Some(agent) = request.agent {
            span.record("agent", agent);
        }
        if let Some(account) = &self.ledger.account {
            span.record("account", account.as_str());
        }
        let cancel = request.cancel.clone();
        let mut calls = Calls::new(self.ledger.clone(), &request, span.clone());
        let mut events = span.in_scope(|| self.inner.run(request));
        let run = async_stream::stream! {
            while let Some(event) = events.next().await {
                match event {
                    ProviderEvent::Call(CallEvent::Started) => calls.start(),
                    ProviderEvent::Call(CallEvent::Ended(call)) => calls.ended(call).await,
                    ProviderEvent::Call(CallEvent::Observed(windows)) => calls.observe(windows).await,
                    event => {
                        match &event {
                            ProviderEvent::Response(usage) => {
                                calls.close(*usage, Outcome::Answered).await;
                            }
                            ProviderEvent::Error(failure) => {
                                calls.close(None, Outcome::Refused(failure.code.clone())).await;
                            }
                            _ => {}
                        }
                        yield event;
                    }
                }
            }
            // A run that ends without its response was cancelled, or ended
            // at a batch of tool calls whose results it waits for.
            let outcome = if cancel.is_cancelled() { Outcome::Cancelled } else { Outcome::Answered };
            calls.close(None, outcome).await;
        };
        within(span, run)
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

    fn idle(&mut self) {
        self.inner.idle();
    }

    fn prepares_ahead(&self) -> bool {
        self.inner.prepares_ahead()
    }

    /// Readying ahead is no request: it is neither limited nor recorded.
    fn prepare(&mut self, request: InferenceRequest, ahead: Ahead) -> LocalBoxFuture<'_, ()> {
        self.inner.prepare(request, ahead)
    }

    fn request_limits(&self, model: &Model) -> RequestLimits {
        self.inner.request_limits(model)
    }
}

#[cfg(test)]
mod tests {
    use demi_backend_database::control::testing;
    use demi_provider_common::testing::{ScriptedRuntime, Turn, event, inference_request};
    use futures_util::future::join_all;

    use super::*;

    /// A call ending with `usage`, `hour` of its cache write kept for an
    /// hour.
    fn call(usage: TokenUsage, hour: u64) -> Turn {
        Turn::Events(vec![
            ProviderEvent::Call(CallEvent::Ended(VendorCall {
                vendor_id: None,
                usage: Some(usage),
                cache_write_1h_tokens: hour,
                outcome: CallOutcome::Answered,
            })),
            event::response(0, 0),
        ])
    }

    fn tokens(input_tokens: u64, cache_write_tokens: u64, cache_read_tokens: u64, output_tokens: u64) -> TokenUsage {
        TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        }
    }

    /// Each row is priced as it is written, at the catalog's prices of its
    /// model at that moment:
    ///
    /// - A call before the catalog had a price, as when its lookup failed,
    ///   stays unpriced, and the next is priced once it has one.
    /// - The part of a write kept for an hour is at twice the input rate.
    /// - A call that used a kind its model has no price for has no price, while
    ///   the model's other calls do.
    /// - Each row keeps the hashes of the prompt it sent, so a request whose
    ///   system prompt changed is the miss's cause
    ///   (`crates/backend-database/src/usage.rs`).
    ///
    /// Planted defects:
    ///
    /// - The prices looked up once as the runtime is built.
    /// - The hour-long write at the write rate.
    /// - An unpriced kind priced at zero.
    /// - One hash for every prompt.
    ///
    /// Cost: five scripted calls on an in-memory ledger, milliseconds.
    #[tokio::test]
    async fn rows_are_priced_at_the_catalog_as_written_and_keep_their_prompts_hashes() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            Arc::new(demi_backend_database::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await;
        let conversation = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap();
        // The catalog's prices of the runtime's model as each row is written.
        let ledger = |catalog: Rc<RefCell<Option<ModelCost>>>| Ledger {
            control: control.clone(),
            clock: Arc::new(demi_shared_types::SystemClock),
            user: master.id.clone(),
            conversation: conversation.clone(),
            provider: ProviderId::try_from("entry-1").unwrap(),
            account: None,
            prices: Rc::new(move |_| {
                let prices = *catalog.borrow();
                Box::pin(async move { prices })
            }),
        };
        let limit = Rc::new(RefCell::new(RequestRateLimit::new(10)));
        let catalog = Rc::new(RefCell::new(None));
        let priced = ScriptedRuntime::new([
            call(tokens(1_000, 3_000, 10_000, 200), 1_000),
            call(tokens(100, 20_000, 0, 10), 0),
            call(tokens(100, 1_500, 30_000, 10), 1_000),
        ]);
        let mut runtime = MeteredRuntime::new(Box::new(priced), limit.clone(), ledger(catalog.clone()));
        let mut request = inference_request();
        request.system_prompt = "You are Demi.".into();
        runtime.run(request.clone()).collect::<Vec<_>>().await;
        *catalog.borrow_mut() = Some(ModelCost {
            input: Some(5.0),
            output: Some(25.0),
            cache_read: Some(0.5),
            cache_write: Some(6.25),
        });
        request.system_prompt = "You are Demi. Read AGENTS.md.".into();
        runtime.run(request.clone()).collect::<Vec<_>>().await;
        runtime.run(request.clone()).collect::<Vec<_>>().await;
        let unpriced = ScriptedRuntime::new([
            call(tokens(100, 500, 30_000, 10), 0),
            call(tokens(1_000, 0, 2_000, 100), 0),
        ]);
        let without_write = Rc::new(RefCell::new(Some(ModelCost {
            input: Some(3.0),
            output: Some(15.0),
            cache_read: Some(0.3),
            cache_write: None,
        })));
        let mut runtime = MeteredRuntime::new(Box::new(unpriced), limit, ledger(without_write));
        runtime.run(request.clone()).collect::<Vec<_>>().await;
        runtime.run(request).collect::<Vec<_>>().await;

        let cost = control.conversation_cost(conversation, Vec::new(), None).await.unwrap();
        let close = |found: Option<f64>, expected: f64| {
            assert!(found.is_some_and(|found| (found - expected).abs() < 1e-9), "{found:?} ≠ {expected}");
        };
        let prices: Vec<Option<f64>> = cost.calls.iter().map(|call| call.price).collect();
        assert_eq!(prices[0], None, "{prices:?}");
        close(prices[1], 0.12575);
        // 100 × $5 + 500 × $6.25 + 1,000 × $10 + 30,000 × $0.5 + 10 × $25,
        // per million.
        close(prices[2], 0.028875);
        assert_eq!(prices[3], None, "{prices:?}");
        close(prices[4], 0.0051);
        close(cost.totals.price.total, 0.12575 + 0.028875 + 0.0051);
        close(cost.totals.price.cache_write, 0.125 + 0.013125);
        // As the page reads the causes.
        let misses: Vec<serde_json::Value> =
            cost.calls.iter().map(|call| serde_json::to_value(call.miss).unwrap()).collect();
        let none = serde_json::Value::Null;
        assert_eq!(misses, [none.clone(), "system_prompt_changed".into(), none.clone(), none.clone(), none]);
        control.close().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn answered_requests_are_ledger_rows_before_their_response_and_the_limit_refuses_the_rest()
     {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(demi_backend_database::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await;
        let ledger = Ledger {
            control: control.clone(),
            clock: Arc::new(demi_shared_types::SystemClock),
            user: master.id.clone(),
            conversation: ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap(),
            provider: ProviderId::try_from("entry-1").unwrap(),
            account: None,
            prices: Rc::new(|_| Box::pin(async { None })),
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
        assert_eq!(
            script.remaining(),
            1,
            "a refused request never reaches the vendor"
        );
        let totals = requests().await;
        assert_eq!(
            (
                totals[0].requests,
                totals[0].input_tokens,
                totals[0].output_tokens
            ),
            (3, 303, 30)
        );
        assert_eq!(
            (totals[0].provider_id.as_str(), totals[0].model_id.as_str()),
            ("entry-1", "model-1")
        );
        control.close().await.unwrap();
    }
}
