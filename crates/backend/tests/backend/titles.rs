//! Conversation titles (`product.md` § Conversation titles, `web-api.md`
//! § Resource index): the first message titles the conversation at once and
//! a request beside the first turn writes a better one, and nothing asks
//! again; a rename that lands while the request is in flight wins; an
//! archive ends the request; an answer without a title writes nothing. The
//! model is a scripted family that tells a title request from a turn by its
//! system prompt. No test calls a real model.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use demi_backend_providers::llm::families::{
    FamilyArgs, FamilyCredential, FamilyError, ProviderFamily,
};
use demi_provider_common::{
    Capabilities, CatalogError, InferenceItem, InferenceRequest, Provider, ProviderEvent,
    ProviderRun, ProviderRuntime, RequestLimits, RuntimeEnv, RuntimeError, UserPart,
};
use demi_shared_types::{
    AuthState, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState,
    Timestamp, TokenUsage,
};
use demi_web_api_protocol::conversations::{ConversationStatus, ConversationSummary};
use demi_web_api_protocol::state::SyncEvent;
use demi_web_api_protocol::providers::{CredentialKind, ProviderAnswer};
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{StreamExt as _, stream};
use reqwest::StatusCode;
use serde_json::json;
use tokio::sync::Semaphore;

use crate::conversations::{FIRST, SECOND, Socket, choose, configured, create, summaries};
use crate::support::{Harness, Session, SyncChannel, TestBackend, eventually};

/// The title the scripted model writes.
const GENERATED: &str = "TS2307 after package split";

/// What the scripted model was asked for titles, and the title requests it
/// may answer: each waits for a permit the test adds, and then writes
/// `answer`, or for its cancel, which it counts.
struct Script {
    asked: Mutex<Vec<InferenceRequest>>,
    answers: Semaphore,
    answer: String,
    cancelled: AtomicUsize,
}

impl Script {
    fn new(answer: &str) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            answers: Semaphore::new(0),
            answer: answer.to_owned(),
            cancelled: AtomicUsize::new(0),
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
            script: self.0.clone(),
        }))
    }
}

struct TitlingProvider {
    script: Arc<Script>,
}

impl Provider for TitlingProvider {
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
        Box::pin(async {
            // The model `m`, with no thinking effort to choose.
            let mut catalog = crate::families::catalog(&["m"]);
            catalog.models[0].supported_thinking_efforts = None;
            Ok(catalog)
        })
    }

    fn read_failure(&self, _: &ProviderErrorDiagnostics, _: Timestamp) -> ProviderFailureFacts {
        ProviderFailureFacts { retry_at: None, continue_with: None }
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(TitlingRuntime(self.script.clone())))
    }
}

struct TitlingRuntime(Arc<Script>);

fn usage() -> ProviderEvent {
    ProviderEvent::Response(Some(TokenUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }))
}

impl ProviderRuntime for TitlingRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        if !request
            .system_prompt
            .starts_with("You are a title generator")
        {
            return stream::iter([ProviderEvent::TextDelta("Answer.".into()), usage()])
                .boxed_local();
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
                () = cancel.cancelled() => {
                    script.cancelled.fetch_add(1, Ordering::SeqCst);
                    stream::empty().boxed_local()
                }
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

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits::default()
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
            [UserPart::Text(text)] => text.clone(),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

/// A backend with titles on, whose master has an entry of the scripted
/// family that configures the model `m` with an output limit of 8,000
/// tokens, and that entry; the harness holds the backend's data.
async fn titling(script: &Arc<Script>) -> (Harness, TestBackend, Session, String) {
    let mut harness = Harness::new()
        .with_families(demi_backend::families::builtin().with("titling", Titling(script.clone())));
    harness.conversations.titles = true;
    let (backend, master) = harness.start_set_up().await;
    let entry = json!({
        "source": "family", "providerType": "titling", "label": "T", "apiKey": "k", "models": [configured(8_000)]
    });
    let created = backend.post("/api/providers", Some(&master), entry).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let provider = created
        .json::<ProviderAnswer>()
        .provider
        .id
        .as_str()
        .to_owned();
    (harness, backend, master, provider)
}

/// Waits on `page` for the summary of the conversation `id` that `done`
/// accepts, and answers it.
async fn pushed(
    page: &mut SyncChannel,
    id: &str,
    done: impl Fn(&ConversationSummary) -> bool,
) -> ConversationSummary {
    let received = page
        .until(|event| {
            matches!(event, SyncEvent::Conversation { conversation }
                if conversation.id.as_str() == id && done(conversation))
        })
        .await;
    match received.into_iter().last() {
        Some(SyncEvent::Conversation { conversation }) => *conversation,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_first_message_titles_the_conversation_once_and_an_archive_ends_the_request() {
    let script = Script::new(&format!("\"{GENERATED}\"\n"));
    let (harness, backend, master, provider) = titling(&script).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The first message is the title at once, and a request beside the
    // first turn writes a better one.
    let message = "why does pnpm build fail with TS2307 after I moved auth into its own package";
    socket.chat("m1", message).await;
    eventually("the title request is asked", || async {
        script.asked() == 1
    })
    .await;
    assert_eq!(summary(&backend, &master, FIRST).await.title.as_deref(), Some(message));
    script.answers.add_permits(1);
    eventually("the generated title is written", || async {
        summary(&backend, &master, FIRST).await.title.as_deref() == Some(GENERATED)
    })
    .await;
    {
        let asked = script.asked.lock().unwrap();
        assert!(input(&asked[0]).contains(message), "{}", input(&asked[0]));
        // The request's own cap, although the configured model allows more
        // (`models.md` § Output limit).
        assert_eq!(
            asked[0].max_output_tokens().map(|limit| limit.get()),
            Some(4_096)
        );
        assert!(asked[0].tools.is_empty() && asked[0].thinking.is_none());
    }
    // The request is metered like a turn.
    let totals = backend
        .get("/api/usage", Some(&master))
        .await
        .json::<demi_web_api_protocol::usage::UsageTotals>();
    assert_eq!(totals.totals[0].requests, 2);
    // Its row is no agent's and no turn's, and says it was a title's.
    let rows = crate::usage::rows(&harness);
    let rows: Vec<_> = rows
        .iter()
        .map(|row| (row.agent, row.turn_id.is_some(), row.purpose.as_deref()))
        .collect();
    assert!(rows.contains(&(None, false, Some("title"))), "{rows:?}");
    assert!(rows.contains(&(Some(0), true, Some("turn"))), "{rows:?}");

    // A later message titles nothing, nothing asks for a title again, and
    // the summary says nothing of a request.
    socket.chat("m2", "and the login test").await;
    assert_eq!(script.asked(), 1);
    let asked_again = backend
        .post(&format!("/api/conversations/{FIRST}/title"), Some(&master), json!({}))
        .await;
    assert_eq!(asked_again.status, StatusCode::NOT_FOUND);
    let listed = backend
        .get("/api/conversations?archived=false", Some(&master))
        .await
        .json::<serde_json::Value>();
    let listed = &listed["conversations"][0];
    assert_eq!(listed["title"], GENERATED);
    assert!(
        listed.get("titleCurrent").is_none() && listed.get("titleGenerating").is_none(),
        "{listed}"
    );

    // An archive ends a request in flight, which writes nothing.
    create(&backend, &master, SECOND).await;
    choose(&backend, &master, SECOND, &provider, "m").await;
    let mut second = Socket::connect(&backend, &master, SECOND).await;
    second.open().await;
    second.chat("m3", "rename the login test").await;
    eventually("the second request is asked", || async {
        script.asked() == 2
    })
    .await;
    let archived = backend
        .patch(
            &format!("/api/conversations/{SECOND}"),
            &master,
            json!({ "archived": true }),
        )
        .await;
    assert_eq!(archived.status, StatusCode::OK);
    eventually("the archive cancels the request", || async {
        script.cancelled.load(Ordering::SeqCst) == 1
    })
    .await;
    let kept = backend
        .get("/api/conversations?archived=true", Some(&master))
        .await
        .json::<demi_web_api_protocol::conversations::Conversations>()
        .conversations;
    assert_eq!(kept[0].title.as_deref(), Some("rename the login test"));
    backend.close().await;
}

#[tokio::test]
async fn a_rename_while_the_title_is_asked_wins() {
    let script = Script::new(GENERATED);
    let (_harness, backend, master, provider) = titling(&script).await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "why does pnpm build fail").await;
    eventually("the title request is asked", || async {
        script.asked() == 1
    })
    .await;
    // The turn's changes reach the page before the rename, so the next
    // summary after it is the one the request's end pushes.
    pushed(&mut page, FIRST, |summary| {
        summary.status == ConversationStatus::Completed
    })
    .await;
    let renamed = backend
        .patch(
            &format!("/api/conversations/{FIRST}"),
            &master,
            json!({ "title": "Kept" }),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    pushed(&mut page, FIRST, |summary| summary.title.as_deref() == Some("Kept")).await;
    script.answers.add_permits(1);
    let ended = pushed(&mut page, FIRST, |_| true).await;
    assert_eq!(ended.title.as_deref(), Some("Kept"));
    backend.close().await;
}

#[tokio::test]
async fn an_answer_without_a_title_leaves_the_messages_title() {
    let script = Script::new(" \n\n");
    let (_harness, backend, master, provider) = titling(&script).await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "hello   there").await;
    eventually("the title request is asked", || async {
        script.asked() == 1
    })
    .await;
    pushed(&mut page, FIRST, |summary| {
        summary.status == ConversationStatus::Completed
    })
    .await;
    script.answers.add_permits(1);
    // The title in place stays, and nothing retries.
    let ended = pushed(&mut page, FIRST, |_| true).await;
    assert_eq!(ended.title.as_deref(), Some("hello there"));
    assert_eq!(script.asked(), 1);
    backend.close().await;
}
