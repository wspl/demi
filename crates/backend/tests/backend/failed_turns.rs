//! A root's turn that failed with a known time to try again
//! (`failures-and-recovery.md` § The unfinished turn): it is resumed at that
//! time, as the user's Resume would, only when the user's `resumeAtRetry`
//! preference says so at that moment, and once. The model is a scripted
//! family whose failures name a retry time an hour after they arrive; the
//! shard's timer runs on the paused clock. No test calls a real model.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demi_backend_user_shard::shard::ShardPlacement;
use demi_backend_providers::llm::families::{
    FamilyArgs, FamilyCredential, FamilyError, ProviderFamily,
};
use demi_provider_common::testing::event;
use demi_provider_common::{
    Capabilities, CatalogError, InferenceItem, InferenceRequest, Provider, ProviderEvent,
    ProviderFailure, ProviderRun, ProviderRuntime, RequestLimits, RuntimeEnv, RuntimeError,
};
use demi_shared_types::{
    AuthState, FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use demi_web_api_protocol::providers::CredentialKind;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{StreamExt as _, stream};
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, Socket, choose, configured, create, entry};
use crate::support::{Harness, elapse, eventually};

/// The answers of the family's requests, in order, and what each request
/// carried.
#[derive(Default)]
struct Script {
    answers: Mutex<VecDeque<Vec<ProviderEvent>>>,
    asked: Mutex<Vec<Vec<InferenceItem>>>,
}

impl Script {
    fn push(&self, answer: Vec<ProviderEvent>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    fn asked(&self) -> Vec<Vec<InferenceItem>> {
        self.asked.lock().unwrap().clone()
    }
}

/// A usage limit as the vendor words it, with its record.
fn limited() -> Vec<ProviderEvent> {
    vec![ProviderEvent::Error(ProviderFailure {
        message: "You've hit your session limit".into(),
        code: None,
        diagnostics: Some(Box::new(ProviderErrorDiagnostics {
            source: FailureSource::Stream,
            client_request_id: None,
            provider_request_id: None,
            provider_response_id: None,
            provider_code: None,
            http_status: Some(429),
            upstream: Some(r#"{"type":"error","error":"usage_limit"}"#.into()),
        })),
        retry_after: None,
    })]
}

/// An API-key family whose failures can succeed again an hour after they
/// arrive.
struct Limited(Arc<Script>);

impl ProviderFamily for Limited {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(_) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        Ok(Arc::new(LimitedProvider(self.0.clone())))
    }
}

struct LimitedProvider(Arc<Script>);

impl Provider for LimitedProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            AuthState::Authenticated {
                account_label: None,
            }
        })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready { message: None }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(async { Err(CatalogError::Unavailable("no directory".into())) })
    }

    fn read_failure(&self, _: &ProviderErrorDiagnostics, received_at: Timestamp) -> ProviderFailureFacts {
        ProviderFailureFacts {
            retry_at: Some(Timestamp::truncate(received_at.to_jiff() + SignedDuration::from_hours(1))),
        }
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(LimitedRuntime(self.0.clone())))
    }
}

struct LimitedRuntime(Arc<Script>);

impl ProviderRuntime for LimitedRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        self.0.asked.lock().unwrap().push(request.items.to_vec());
        let answer = self
            .0
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("an answer is scripted for each request");
        stream::iter(answer).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self(self.0.clone()))
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits::default()
    }
}

/// Off, as by default, the failure's time comes and the turn waits for the
/// user; on, a failed turn resumes at its time, as the user's Resume would,
/// and once: the turn it resumed answered, and the next request is the
/// user's next message.
// One conversation, four scripted requests and three hours of paused time:
// well under a second.
#[tokio::test(flavor = "local")]
async fn a_failed_turn_resumes_at_its_retry_time_once_and_only_when_the_user_prefers_it() {
    let logs = tempfile::tempdir().unwrap();
    let path = logs.path().join("backend.log");
    let file = Arc::new(std::fs::File::create(&path).unwrap());
    // The shards run on this test's thread, which this log hears.
    let _log =
        tracing::subscriber::set_default(tracing_subscriber::fmt().with_writer(file).finish());
    let log = || std::fs::read_to_string(&path).unwrap();
    let script = Arc::new(Script::default());
    let mut harness = Harness::new()
        .with_families(demi_backend::families::builtin().with("limited", Limited(script.clone())));
    harness.shards = Some(ShardPlacement::Inline);
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "limited", "label": "Limited", "apiKey": "k",
        "models": [configured(4_000)]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // Off: at 09:00 the failure of 08:00 can succeed again, and nothing
    // starts.
    script.push(limited());
    socket.chat("m1", "first").await;
    let waiting = || async { log().matches("waits for its retry time").count() };
    eventually("the timer is armed", || async { waiting().await == 1 }).await;
    harness.clock.advance(SignedDuration::from_hours(1));
    elapse(Duration::from_secs(3600)).await;
    eventually("the retry time came", || async { log().contains("it waits for the user") }).await;
    assert_eq!(script.asked().len(), 1);

    // On: the user's next message fails at 09:00 too, and at 10:00 the turn
    // resumes by itself.
    let turned = backend
        .patch("/api/settings/preferences", &master, json!({ "resumeAtRetry": true }))
        .await;
    assert_eq!(turned.status, StatusCode::OK);
    script.push(limited());
    script.push(vec![event::text("resumed"), event::response(1, 1)]);
    socket.chat("m2", "second").await;
    assert_eq!(script.asked().len(), 2);
    eventually("the timer is armed", || async { waiting().await == 2 }).await;
    harness.clock.advance(SignedDuration::from_hours(1));
    elapse(Duration::from_secs(3600)).await;
    socket.until_idle().await;
    let asked = script.asked();
    assert_eq!(asked.len(), 3);
    // The failure came before any output, so the resume runs the turn
    // again from its message.
    assert_eq!(asked[2], asked[1]);
    assert!(log().contains("a failed turn resumed at its retry time"));

    // Once: an hour later, the next request is the user's message.
    harness.clock.advance(SignedDuration::from_hours(1));
    elapse(Duration::from_secs(3600)).await;
    script.push(vec![event::text("third"), event::response(1, 1)]);
    socket.chat("m3", "third").await;
    let asked = script.asked();
    assert_eq!(asked.len(), 4);
    let sent = format!("{:?}", asked[3].last());
    assert!(sent.contains("third"), "{sent}");
    backend.close().await;
}
