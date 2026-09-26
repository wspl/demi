//! Conversation titles (`product.md` § Conversation titles, `web-api.md`
//! § Resource index): the first message titles the conversation at once and
//! a request beside the first turn writes a better one; Detect title asks
//! again; a rename that lands while a request is in flight wins; an archive
//! ends the request; an answer without a title writes nothing. The model is
//! a scripted family that tells a title request from a turn by its system
//! prompt. No test calls a real model.

use std::sync::{Arc, Mutex};

use demi_agent::testing::model_of;
use demi_backend::{FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily};
use demi_core::{
    AuthState, ModelSelection, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState, Timestamp,
    TokenUsage, UserContentBlock,
};
use demi_provider::{
    Capabilities, CatalogError, InferenceItem, InferenceRequest, Provider, ProviderEvent, ProviderRun, ProviderRuntime,
    RuntimeEnv, RuntimeError,
};
use demi_web_api::conversations::ConversationSummary;
use demi_web_api::error::ErrorCode;
use demi_web_api::providers::{CredentialKind, ProviderAnswer};
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{StreamExt as _, stream};
use reqwest::StatusCode;
use serde_json::json;
use tokio::sync::Semaphore;

use crate::conversations::{FIRST, SECOND, Socket, create, summaries};
use crate::support::{Harness, Session, TestBackend, eventually};

/// The title the scripted model writes.
const GENERATED: &str = "TS2307 after package split";

/// What the scripted model was asked for titles, and the title requests it
/// may answer: each waits for a permit the test adds, or its cancel, and
/// then writes `answer`.
struct Script {
    asked: Mutex<Vec<InferenceRequest>>,
    answers: Semaphore,
    answer: String,
}

impl Script {
    fn new(answer: &str) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            answers: Semaphore::new(0),
            answer: answer.to_owned(),
        })
    }

    fn asked(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}

struct Titling(Arc<Script>);

impl ProviderFamily for Titling {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(_) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        Ok(Arc::new(TitlingProvider {
            id: args.entry_id,
            script: self.0.clone(),
        }))
    }
}

struct TitlingProvider {
    id: String,
    script: Arc<Script>,
}

impl Provider for TitlingProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        "Titling"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async { AuthState::Authenticated { account_label: None } })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready { message: None }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(async { Err(CatalogError::Unavailable("no directory".into())) })
    }

    fn read_failure(&self, _: &ProviderErrorDiagnostics, _: Timestamp) -> ProviderFailureFacts {
        ProviderFailureFacts { retry_at: None }
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(TitlingRuntime(self.script.clone())))
    }
}

struct TitlingRuntime(Arc<Script>);

fn usage() -> ProviderEvent {
    ProviderEvent::Response(TokenUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    })
}

impl ProviderRuntime for TitlingRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        if !request.system_prompt.starts_with("You are a title generator") {
            return stream::iter([ProviderEvent::TextDelta("Answer.".into()), usage()]).boxed_local();
        }
        let script = self.0.clone();
        let cancel = request.cancel.clone();
        script.asked.lock().unwrap().push(request);
        stream::once(async move {
            tokio::select! {
                permit = script.answers.acquire() => {
                    permit.unwrap().forget();
                    stream::iter([ProviderEvent::TextDelta(script.answer.clone()), usage()]).boxed_local()
                }
                // A cancelled run ends without an event.
                () = cancel.cancelled() => stream::empty().boxed_local(),
            }
        })
        .flatten()
        .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self(self.0.clone()))
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

async fn summary(backend: &TestBackend, session: &Session, id: &str) -> ConversationSummary {
    summaries(backend, session)
        .await
        .into_iter()
        .find(|summary| summary.id.as_str() == id)
        .expect("the conversation is listed")
}

/// The text of the user message a title request read.
fn input(request: &InferenceRequest) -> String {
    match &request.items[..] {
        [InferenceItem::UserMessage { content }] => match &content[..] {
            [UserContentBlock::Text { text }] => text.clone(),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

/// A backend with titles on, whose master has an entry of the scripted
/// family, and the model of that entry; the harness holds the backend's
/// data.
async fn titling(script: &Arc<Script>) -> (Harness, TestBackend, Session, ModelSelection) {
    let mut harness = Harness::new().with_families(FamilyRegistry::builtin().with("titling", Titling(script.clone())));
    harness.conversations.titles = true;
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "custom", "providerType": "titling", "label": "T", "apiKey": "k" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let provider = created.json::<ProviderAnswer>().provider.id.as_str().to_owned();
    (harness, backend, master, model_of(&provider, "m"))
}

#[tokio::test]
async fn a_title_follows_the_first_message_and_a_rename_or_an_archive_while_one_is_asked_wins() {
    let script = Script::new(&format!("\"{GENERATED}\"\n"));
    let (_harness, backend, master, model) = titling(&script).await;
    create(&backend, &master, FIRST).await;
    // The browser's record creation repeats the placeholder, which settles
    // nothing.
    let repeated = backend
        .patch(&format!("/api/conversations/{FIRST}"), &master, json!({ "title": "New conversation" }))
        .await;
    assert_eq!(repeated.status, StatusCode::OK);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model).await;

    // The first message is the title at once, and a request beside the
    // first turn writes a better one.
    let message = "why does pnpm build fail with TS2307 after I moved auth into its own package";
    socket.chat("m1", message).await;
    eventually("the title request is asked", || async { script.asked() == 1 }).await;
    let asked = summary(&backend, &master, FIRST).await;
    assert_eq!((asked.title.as_str(), asked.title_generating), (message, true));
    script.answers.add_permits(1);
    eventually("the generated title is written", || async {
        summary(&backend, &master, FIRST).await.title == GENERATED
    })
    .await;
    let titled = summary(&backend, &master, FIRST).await;
    assert_eq!((titled.title_generating, titled.title_current), (false, true));
    {
        let asked = script.asked.lock().unwrap();
        assert_eq!(input(&asked[0]), format!("1. {message}"));
        assert_eq!(asked[0].output_limit.map(|limit| limit.get()), Some(1_024));
        assert!(asked[0].tools.is_empty() && asked[0].thinking.is_none());
    }
    // The request is metered like a turn.
    let totals = backend.get("/api/usage", Some(&master)).await.json::<demi_web_api::usage::UsageTotals>();
    assert_eq!(totals.totals[0].requests, 2);

    // A later message titles nothing by itself, and the title is no longer
    // current; a rename that lands while Detect title's request is in
    // flight wins.
    socket.chat("m2", "and the login test").await;
    assert!(!summary(&backend, &master, FIRST).await.title_current);
    let path = format!("/api/conversations/{FIRST}/title");
    let detect = backend.post(&path, Some(&master), json!({ "model": model })).await;
    assert_eq!(detect.status, StatusCode::ACCEPTED, "{}", String::from_utf8_lossy(&detect.body));
    eventually("Detect title is asked", || async { script.asked() == 2 }).await;
    assert_eq!(input(&script.asked.lock().unwrap()[1]), format!("1. {message}\n2. and the login test"));
    let renamed = backend
        .patch(&format!("/api/conversations/{FIRST}"), &master, json!({ "title": "Kept" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    script.answers.add_permits(1);
    eventually("the request ends", || async {
        !summary(&backend, &master, FIRST).await.title_generating
    })
    .await;
    let kept = summary(&backend, &master, FIRST).await;
    assert_eq!((kept.title.as_str(), kept.title_current), ("Kept", true));

    // An archive ends a request in flight, which writes nothing.
    backend.post(&path, Some(&master), json!({ "model": model })).await;
    eventually("the third request is asked", || async { script.asked() == 3 }).await;
    let archived = backend
        .patch(&format!("/api/conversations/{FIRST}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK);
    eventually("the aborted request ends", || async {
        let listed = backend.get("/api/conversations?archived=true", Some(&master)).await;
        let conversations = listed.json::<demi_web_api::conversations::Conversations>().conversations;
        conversations.first().is_some_and(|summary| !summary.title_generating)
    })
    .await;
    assert_eq!(script.answers.available_permits(), 0);
    let refused = backend.post(&path, Some(&master), json!({ "model": model })).await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));

    // What there is nothing to title with is refused.
    create(&backend, &master, SECOND).await;
    let empty = backend
        .post(&format!("/api/conversations/{SECOND}/title"), Some(&master), json!({ "model": model }))
        .await;
    assert_eq!(empty.refusal(), (StatusCode::CONFLICT, ErrorCode::NoMessages));
    let foreign = backend
        .post(
            &format!("/api/conversations/{SECOND}/title"),
            Some(&master),
            json!({ "model": model_of("someone-elses", "m") }),
        )
        .await;
    assert_eq!(foreign.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound));
    backend.close().await;
}

#[tokio::test]
async fn an_answer_without_a_title_leaves_the_messages_title_and_detect_title_available() {
    let script = Script::new(" \n\n");
    let (_harness, backend, master, model) = titling(&script).await;
    create(&backend, &master, FIRST).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model).await;
    socket.chat("m1", "hello   there").await;
    eventually("the title request is asked", || async { script.asked() == 1 }).await;
    script.answers.add_permits(1);
    eventually("the request ends", || async {
        !summary(&backend, &master, FIRST).await.title_generating
    })
    .await;
    // The title in place stays, and asking again is the retry.
    let left = summary(&backend, &master, FIRST).await;
    assert_eq!((left.title.as_str(), left.title_current), ("hello there", false));
    backend.close().await;
}
