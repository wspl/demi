//! A root's turn that failed with a known time to try again
//! (`failures-and-recovery.md` § The unfinished turn): it is resumed at that
//! time, as the user's Resume would, only when the user's `resumeAtRetry`
//! preference says so at that moment, and once; the wait outlives the
//! tree's eviction, which the moment opens again, and a restart, after which
//! opening the conversation waits again for a moment still to come; a newer
//! failure replaces it, and the shard's close stops a resume under way. The
//! model is a scripted family whose failures name a retry time an hour after
//! they arrive; the shard's timer runs on the paused clock. No test calls a
//! real model.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
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

use crate::conversations::{FIRST, SECOND, Socket, choose, configured, create, entry};
use crate::support::{Harness, Session, TestBackend, elapse, eventually};

/// The answers of the family's requests, in order, and what each request
/// carried.
#[derive(Default)]
struct Script {
    answers: Mutex<VecDeque<Vec<ProviderEvent>>>,
    asked: Mutex<Vec<Vec<InferenceItem>>>,
    /// How many runtimes were built and closed: a tree opens with one and
    /// closes it as it goes.
    built: AtomicUsize,
    closed: AtomicUsize,
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
            continue_with: None,
        }
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        self.0.built.fetch_add(1, Ordering::SeqCst);
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
        self.0.closed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits::default()
    }
}

/// A backend on the paused clock whose conversation `FIRST` infers with the
/// scripted family, and the log its shards write.
struct World {
    harness: Harness,
    backend: TestBackend,
    master: Session,
    script: Arc<Script>,
    log: std::path::PathBuf,
    _logs: tempfile::TempDir,
    _subscriber: tracing::subscriber::DefaultGuard,
}

impl World {
    async fn start() -> Self {
        let logs = tempfile::tempdir().unwrap();
        let log = logs.path().join("backend.log");
        let file = Arc::new(std::fs::File::create(&log).unwrap());
        // The shards run on this test's thread, which this log hears.
        let subscriber =
            tracing::subscriber::set_default(tracing_subscriber::fmt().with_writer(file).finish());
        let script = Arc::new(Script::default());
        let mut harness = Harness::new().with_families(
            demi_backend::families::builtin().with("limited", Limited(script.clone())),
        );
        harness.shards = Some(ShardPlacement::Inline);
        let (backend, master) = harness.start_set_up().await;
        let body = json!({
            "source": "family", "providerType": "limited", "label": "Limited", "apiKey": "k",
            "models": [configured(4_000)]
        });
        let provider = entry(&backend, &master, body).await;
        create(&backend, &master, FIRST).await;
        choose(&backend, &master, FIRST, &provider, "m").await;
        Self {
            harness,
            backend,
            master,
            script,
            log,
            _logs: logs,
            _subscriber: subscriber,
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap()
    }

    /// How often the log says `line`.
    fn said(&self, line: &str) -> usize {
        self.log().matches(line).count()
    }

    async fn socket(&self) -> Socket {
        let mut socket = Socket::connect(&self.backend, &self.master, FIRST).await;
        socket.open().await;
        socket
    }

    async fn prefer(&self, on: bool) {
        let turned = self
            .backend
            .patch("/api/settings/preferences", &self.master, json!({ "resumeAtRetry": on }))
            .await;
        assert_eq!(turned.status, StatusCode::OK);
    }

    /// Sends `text`, whose turn fails, and waits until its timer is the
    /// `armed`th that waited.
    async fn fail(&self, socket: &mut Socket, id: &str, text: &str, armed: usize) {
        self.script.push(limited());
        socket.chat(id, text).await;
        eventually("the timer is armed", || async { self.said(WAITS) == armed }).await;
    }

    /// Closes the page's `socket`, the `ended`th to end, and waits until the
    /// backend has seen it go.
    async fn leave(&self, socket: Socket, ended: usize) {
        drop(socket);
        eventually("the page left", || async { self.said("a page socket ended") == ended }).await;
    }

    /// Moves both clocks on by `minutes`.
    async fn pass(&self, minutes: i64) {
        self.harness.clock.advance(SignedDuration::from_mins(minutes));
        elapse(Duration::from_secs(60 * minutes.unsigned_abs())).await;
    }

    fn asked(&self) -> usize {
        self.script.asked().len()
    }
}

const WAITS: &str = "a failed turn waits for its retry time";
const DECLINED: &str = "it waits for the user";
const RESUMED: &str = "a failed turn resumed at its retry time";

fn answered() -> Vec<ProviderEvent> {
    vec![event::text("resumed"), event::response(1, 1)]
}

/// Off, as by default, the failure's time comes and the turn waits for the
/// user; on, a failed turn resumes at its time, as the user's Resume would,
/// and once: the turn it resumed answered, and the next request is the
/// user's next message.
// One conversation, four scripted requests and three hours of paused time:
// well under a second.
#[tokio::test(flavor = "local")]
async fn a_failed_turn_resumes_at_its_retry_time_once_and_only_when_the_user_prefers_it() {
    let world = World::start().await;
    let mut socket = world.socket().await;

    // Off: at 09:00 the failure of 08:00 can succeed again, and nothing
    // starts.
    world.fail(&mut socket, "m1", "first", 1).await;
    world.pass(60).await;
    eventually("the retry time came", || async { world.said(DECLINED) == 1 }).await;
    assert_eq!(world.asked(), 1);

    // On: the user's next message fails at 09:00 too, and at 10:00 the turn
    // resumes by itself.
    world.prefer(true).await;
    world.fail(&mut socket, "m2", "second", 2).await;
    world.script.push(answered());
    world.pass(60).await;
    socket.until_idle().await;
    let asked = world.script.asked();
    assert_eq!(asked.len(), 3);
    // The failure came before any output, so the resume runs the turn
    // again from its message.
    assert_eq!(asked[2], asked[1]);
    assert_eq!(world.said(RESUMED), 1);

    // Once: an hour later, the next request is the user's message.
    world.pass(60).await;
    world.script.push(vec![event::text("third"), event::response(1, 1)]);
    socket.chat("m3", "third").await;
    let asked = world.script.asked();
    assert_eq!(asked.len(), 4);
    let sent = format!("{:?}", asked[3].last());
    assert!(sent.contains("third"), "{sent}");
    world.backend.close().await;
}

/// The page leaves, and the idle tree is put away to save memory: the wait
/// goes on, and its moment opens the conversation again and resumes the
/// turn. A newer failure replaces the wait of the one before, and the
/// preference turned off during a wait keeps the turn waiting.
// One conversation, five scripted requests and four hours of paused time:
// well under a second.
#[tokio::test(flavor = "local")]
async fn the_wait_outlives_the_trees_eviction_a_newer_failure_replaces_it_and_off_keeps_it() {
    let world = World::start().await;
    world.prefer(true).await;
    let mut socket = world.socket().await;
    world.fail(&mut socket, "m1", "first", 1).await;
    world.leave(socket, 1).await;
    // Ten minutes after the page left, the tree goes, its runtime closed.
    world.pass(11).await;
    eventually("the tree is put away", || async {
        world.script.closed.load(Ordering::SeqCst) == 1
    })
    .await;
    world.script.push(answered());
    world.pass(49).await;
    eventually("the turn resumed", || async { world.asked() == 2 }).await;
    assert_eq!(world.script.built.load(Ordering::SeqCst), 2, "the moment opened the tree again");
    assert_eq!(world.said(RESUMED), 1);

    // At 09:00 a failure that can succeed at 10:00; at 09:30 another, at
    // 10:30: only the newer waits, and it is turned off before its moment.
    let mut socket = world.socket().await;
    world.fail(&mut socket, "m2", "second", 2).await;
    world.pass(30).await;
    world.fail(&mut socket, "m3", "third", 3).await;
    world.pass(20).await;
    world.prefer(false).await;
    world.pass(40).await;
    eventually("the newer failure's time came", || async { world.said(DECLINED) == 1 }).await;
    // The replaced wait never came: one moment came in all, and nothing
    // started.
    assert_eq!(world.said(DECLINED) + world.said(RESUMED), 2);
    assert_eq!(world.asked(), 4);
    world.backend.close().await;
}

/// The wait is the running backend's: after a restart, opening the
/// conversation waits again for a moment still to come, and resumes then;
/// one whose moment passed while the backend was down waits for nothing.
// Two conversations' turns across two restarts on the paused clock: about a
// second, for the backends' starts.
#[tokio::test(flavor = "local")]
async fn after_a_restart_opening_the_conversation_waits_again_only_for_a_moment_to_come() {
    let mut world = World::start().await;
    world.prefer(true).await;
    let mut socket = world.socket().await;
    world.fail(&mut socket, "m1", "first", 1).await;
    drop(socket);
    let address = world.backend.address();
    world.backend.close().await;

    // Restarted at 08:30: the moment, 09:00, is still to come.
    world.harness.clock.advance(SignedDuration::from_mins(30));
    world.backend = world.harness.start_at(address).await;
    let _socket = world.socket().await;
    eventually("the timer is armed again", || async { world.said(WAITS) == 2 }).await;
    world.script.push(answered());
    world.pass(30).await;
    eventually("the turn resumed", || async { world.asked() == 2 }).await;

    // A failure at 09:00, then a restart at 11:00, after its moment.
    let mut socket = world.socket().await;
    world.fail(&mut socket, "m2", "second", 3).await;
    drop((socket, _socket));
    world.backend.close().await;
    world.harness.clock.advance(SignedDuration::from_hours(2));
    world.backend = world.harness.start_at(address).await;
    let mut socket = world.socket().await;
    world.pass(120).await;
    // Nothing waited, and the next request is the user's.
    world.script.push(vec![event::text("fourth"), event::response(1, 1)]);
    socket.chat("m3", "fourth").await;
    assert_eq!(world.said(WAITS), 3);
    assert_eq!(world.asked(), 4);
    world.backend.close().await;
}

/// The moment comes while the backend shuts down: the resume opens nothing
/// once the close began. That the close waits for a resume already past this
/// check is the next test's.
// One conversation and a close on the paused clock: well under a second.
#[tokio::test(flavor = "local")]
async fn a_resume_whose_moment_comes_during_the_shutdown_opens_nothing() {
    let world = World::start().await;
    world.prefer(true).await;
    let mut socket = world.socket().await;
    world.fail(&mut socket, "m1", "first", 1).await;
    world.leave(socket, 1).await;
    world.pass(11).await;
    eventually("the tree is put away", || async {
        world.script.closed.load(Ordering::SeqCst) == 1
    })
    .await;
    create(&world.backend, &world.master, SECOND).await;
    let mut watching = Socket::connect(&world.backend, &world.master, SECOND).await;

    let firing = world.backend.hold_retries(demi_backend_user_shard::holds::RetryStep::Firing);
    world.script.push(answered());
    world.pass(49).await;
    firing.until_arrived(1).await;
    tokio::join!(world.backend.close(), async {
        // The close began: the pages' sockets end first.
        watching.closed().await;
        firing.release();
    });
    // The backend is gone; what it did is in the script and the log.
    assert_eq!(world.script.asked().len(), 1);
    assert_eq!(world.script.built.load(Ordering::SeqCst), 1, "no tree opened again");
    assert!(!std::fs::read_to_string(&world.log).unwrap().contains(RESUMED));
}

/// The shutdown begins while a resume that found the backend open is under
/// way: the close waits for it, so the tree it opens is one the close puts
/// away, and no tree outlives the backend.
// One conversation and a close on the paused clock: well under a second.
#[tokio::test(flavor = "local")]
async fn a_shutdown_waits_for_a_resume_under_way() {
    let world = World::start().await;
    world.prefer(true).await;
    let mut socket = world.socket().await;
    world.fail(&mut socket, "m1", "first", 1).await;
    world.leave(socket, 1).await;
    world.pass(11).await;
    eventually("the tree is put away", || async {
        world.script.closed.load(Ordering::SeqCst) == 1
    })
    .await;
    create(&world.backend, &world.master, SECOND).await;
    let mut watching = Socket::connect(&world.backend, &world.master, SECOND).await;

    let started = world.backend.hold_retries(demi_backend_user_shard::holds::RetryStep::Started);
    // The resumed turn hangs until the shutdown stops it.
    world.script.push(Vec::new());
    world.pass(49).await;
    started.until_arrived(1).await;
    tokio::join!(world.backend.close(), async {
        watching.closed().await;
        started.release();
    });
    let built = world.script.built.load(Ordering::SeqCst);
    assert_eq!(built, 2, "the resume opened the tree again");
    assert_eq!(
        world.script.closed.load(Ordering::SeqCst),
        built,
        "every tree the backend opened was put away by its close"
    );
}
