//! Conversations (`web-api.md` § Conversation creation and Fork, § Sidebar
//! mutations, read state and page synchronization; `runtime.md` § Frame
//! protocol): creation under the browser's id, the list and the product
//! state, a chat over the socket with an Anthropic endpoint the test
//! scripts, a reload whose history is what the database holds, a client that
//! falls behind, a takeover, the frames the backend refuses, provider edits
//! and deletion at the inference boundary, the rate limit, and a shutdown in
//! the middle of a turn. No test calls a real model.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demi_agent::testing::{client_text, model_of};
use demi_agent_protocol::{ClientFrame, ClientFrameKind, EditOutcome, EditRequest, ServerFrame, TranscriptVersion};
use demi_backend::{FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily};
use demi_core::{
    AuthState, Block, ModelSelection, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState,
    SessionPhase, Timestamp, TokenUsage, TurnId,
};
use demi_provider::testing::{MockResponse, MockVendor};
use demi_provider::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderEvent, ProviderRun, ProviderRuntime, RuntimeEnv,
    RuntimeError,
};
use demi_web_api::conversations::{ConversationAnswer, ConversationStatus, ConversationSummary, Conversations, Transcript};
use demi_web_api::error::ErrorCode;
use demi_web_api::providers::{CredentialKind, ProviderAnswer};
use demi_web_api::state::ProductState;
use demi_web_api::usage::UsageTotals;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{SinkExt as _, StreamExt as _, stream};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};

use crate::support::{Harness, MASTER_EMAIL, Session, TestBackend};

/// The conversation ids the tests create.
const FIRST: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b";
const SECOND: &str = "7d1c2e3f-4a5b-4c1e-9d2b-0b6f7f3e8f3a";

/// The system prompt the backend's conversations run with.
const SYSTEM_PROMPT: &str = "You are a coding agent. Answer the user's questions about their code.";

/// What a conversation socket delivered next.
#[derive(Debug)]
enum Received {
    Frame(ServerFrame),
    /// The backend closed the socket, with its close code.
    Closed(Option<u16>),
}

/// A page's conversation socket.
struct Socket {
    socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
}

impl Socket {
    async fn connect(backend: &TestBackend, session: &Session, conversation: &str) -> Self {
        match Self::try_connect(backend, session, conversation).await {
            Ok(socket) => socket,
            Err(status) => panic!("the stream refused the upgrade with {status}"),
        }
    }

    /// The socket, or the status the route answered instead of upgrading.
    async fn try_connect(backend: &TestBackend, session: &Session, conversation: &str) -> Result<Self, u16> {
        let url = backend.ws_url(&format!("/api/conversations/{conversation}/stream"));
        let mut request = url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("cookie", session.cookie.parse().unwrap());
        match tokio_tungstenite::connect_async(request).await {
            Ok((socket, _)) => Ok(Self { socket }),
            Err(tungstenite::Error::Http(response)) => Err(response.status().as_u16()),
            Err(error) => panic!("the socket did not connect: {error}"),
        }
    }

    async fn send(&mut self, frame: &ClientFrame) {
        self.send_text(serde_json::to_string(frame).unwrap()).await;
    }

    async fn send_text(&mut self, text: String) {
        self.socket.send(Message::Text(text.into())).await.unwrap();
    }

    async fn next(&mut self) -> Received {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), self.socket.next())
                .await
                .expect("the socket delivers within ten seconds");
            match message {
                Some(Ok(Message::Text(text))) => {
                    let frame = serde_json::from_str(text.as_str()).unwrap_or_else(|error| panic!("{error}: {text}"));
                    return Received::Frame(frame);
                }
                Some(Ok(Message::Close(close))) => return Received::Closed(close.map(|close| u16::from(close.code))),
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return Received::Closed(None),
            }
        }
    }

    async fn frame(&mut self) -> ServerFrame {
        match self.next().await {
            Received::Frame(frame) => frame,
            Received::Closed(code) => panic!("the socket closed with {code:?}"),
        }
    }

    /// The frames up to and including the first `done` accepts.
    async fn until(&mut self, done: impl Fn(&ServerFrame) -> bool) -> Vec<ServerFrame> {
        let mut frames = Vec::new();
        loop {
            let frame = self.frame().await;
            let last = done(&frame);
            frames.push(frame);
            if last {
                return frames;
            }
        }
    }

    /// Opens the conversation with `model` and reads the handshake.
    async fn open(&mut self, model: &ModelSelection) -> Vec<ServerFrame> {
        self.send(&ClientFrame::Open { model: model.clone() }).await;
        let handshake = self.until(|frame| matches!(frame, ServerFrame::PendingSteers { .. })).await;
        assert_eq!(handshake.first(), Some(&ServerFrame::Opened), "{handshake:?}");
        handshake
    }

    /// Sends a message and reads the frames of its turn, to the phase that
    /// says it ended.
    async fn chat(&mut self, id: &str, text: &str) -> Vec<ServerFrame> {
        self.send(&send(id, text)).await;
        self.until_idle().await
    }

    /// The frames up to the idle phase that follows a running one.
    async fn until_idle(&mut self) -> Vec<ServerFrame> {
        let mut ran = false;
        let mut frames = Vec::new();
        loop {
            let frame = self.frame().await;
            let idle = match &frame {
                ServerFrame::Phase { phase: SessionPhase::Running } => {
                    ran = true;
                    false
                }
                ServerFrame::Phase { phase: SessionPhase::Idle } => ran,
                _ => false,
            };
            frames.push(frame);
            if idle {
                return frames;
            }
        }
    }

    /// The live transcript, as a fresh reset sends it.
    async fn live(&mut self) -> Vec<Block> {
        self.send(&ClientFrame::SyncTranscript {}).await;
        let frames = self.until(|frame| matches!(frame, ServerFrame::TranscriptReset { .. })).await;
        match frames.into_iter().last() {
            Some(ServerFrame::TranscriptReset { blocks, .. }) => blocks,
            other => unreachable!("{other:?}"),
        }
    }

    /// The close code, skipping the frames before it.
    async fn closed(&mut self) -> Option<u16> {
        loop {
            if let Received::Closed(code) = self.next().await {
                return code;
            }
        }
    }
}

fn send(id: &str, text: &str) -> ClientFrame {
    ClientFrame::Send {
        message_id: TurnId::try_from(id).unwrap(),
        content: client_text(text),
    }
}

/// A Messages API stream that answers `deltas`, in that many pieces, and
/// reports `input` and `output` tokens.
fn answer(deltas: &[&str], input: u64, output: u64) -> MockResponse {
    let mut frames = vec![
        json!({ "type": "message_start", "message": {
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-4-8", "content": [],
            "usage": { "input_tokens": input, "output_tokens": 0 } } }),
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
    ];
    for delta in deltas {
        frames.push(json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": delta } }));
    }
    frames.push(json!({ "type": "content_block_stop", "index": 0 }));
    frames.push(json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": output } }));
    frames.push(json!({ "type": "message_stop" }));
    let text: String = frames
        .iter()
        .map(|frame| format!("event: {}\ndata: {frame}\n\n", frame["type"].as_str().unwrap()))
        .collect();
    MockResponse::event_stream(text)
}

/// A new API-key entry of the master's.
async fn entry(backend: &TestBackend, master: &Session, body: Value) -> String {
    let created = backend.post("/api/providers", Some(master), body).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    created.json::<ProviderAnswer>().provider.id.as_str().to_owned()
}

/// An Anthropic entry whose endpoint is `vendor`.
async fn anthropic(backend: &TestBackend, master: &Session, vendor: &MockVendor) -> String {
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url("/v1")
    });
    entry(backend, master, body).await
}

async fn create(backend: &TestBackend, session: &Session, id: &str) -> ConversationSummary {
    let created = backend.post("/api/conversations", Some(session), json!({ "id": id })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    created.json::<ConversationAnswer>().conversation
}

async fn summaries(backend: &TestBackend, session: &Session) -> Vec<ConversationSummary> {
    let listed = backend.get("/api/conversations", Some(session)).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    listed.json::<Conversations>().conversations
}

async fn transcript(backend: &TestBackend, session: &Session, id: &str) -> Transcript {
    let read = backend.get(&format!("/api/conversations/{id}/transcript"), Some(session)).await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

async fn usage(backend: &TestBackend, session: &Session) -> UsageTotals {
    backend.get("/api/usage", Some(session)).await.json()
}

/// Each block's type.
fn kinds(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .map(|block| serde_json::to_value(block).unwrap()["type"].as_str().unwrap().to_owned())
        .collect()
}

/// The text of the last text block.
fn last_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .rev()
        .find_map(|block| match block {
            Block::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .expect("the transcript holds text")
}

#[tokio::test]
async fn a_conversation_is_created_once_under_the_id_the_browser_chose_and_listed_for_its_owner() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("ana@example.test", "ana-pass-1", demi_web_api::auth::Role::User);

    let created = create(&backend, &master, FIRST).await;
    assert_eq!(
        (created.id.as_str(), created.title.as_str(), created.status, created.revision, created.unread),
        (FIRST, "New conversation", ConversationStatus::Idle, 0, false)
    );
    assert_eq!(created.cwd, format!("/home/demi/sessions/{FIRST}"));
    assert_eq!(serde_json::to_value(&created.target).unwrap(), json!({ "kind": "cloud" }));

    // A retry, in any spelling, finds the one it created.
    for spelling in [FIRST.to_owned(), FIRST.to_uppercase()] {
        let again = backend.post("/api/conversations", Some(&master), json!({ "id": spelling })).await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(again.json::<ConversationAnswer>().conversation, created);
    }
    let second = create(&backend, &master, SECOND).await;
    let listed: Vec<String> = summaries(&backend, &master)
        .await
        .into_iter()
        .map(|summary| summary.id.as_str().to_owned())
        .collect();
    assert_eq!(listed, [SECOND, FIRST], "the newest first");
    let archived = backend.get("/api/conversations?archived=true", Some(&master)).await;
    assert!(archived.json::<Conversations>().conversations.is_empty());
    let malformed = backend.get("/api/conversations?archived=1", Some(&master)).await;
    assert_eq!(malformed.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery));
    let state = backend.get("/api/state", Some(&master)).await.json::<ProductState>();
    assert_eq!(state.conversations, [second, created]);
    assert!(transcript(&backend, &master, FIRST).await.blocks.is_empty());

    // Another user can neither take the id nor reach the conversation.
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let taken = backend.post("/api/conversations", Some(&ana), json!({ "id": FIRST.to_uppercase() })).await;
    assert_eq!(taken.refusal(), (StatusCode::CONFLICT, ErrorCode::IdUnavailable));
    for path in [format!("/api/conversations/{FIRST}/transcript"), "/api/conversations/not-a-uuid/transcript".into()] {
        let foreign = backend.get(&path, Some(&ana)).await;
        assert_eq!(foreign.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound), "{path}");
    }
    assert!(summaries(&backend, &ana).await.is_empty());
    assert_eq!(Socket::try_connect(&backend, &ana, FIRST).await.err(), Some(404));
    for body in [json!({ "id": "conversation-1" }), json!({ "id": FIRST, "title": "x" }), json!({})] {
        let refused = backend.post("/api/conversations", Some(&ana), body.clone()).await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{body}");
    }
    backend.close().await;
}

#[tokio::test]
async fn a_message_runs_over_the_socket_and_a_reload_shows_what_the_database_holds() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let model = model_of(&provider, "claude-opus-4-8");

    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    let handshake = socket.open(&model).await;
    let kinds_of: Vec<String> = handshake
        .iter()
        .map(|frame| serde_json::to_value(frame).unwrap()["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds_of, ["opened", "transcript_reset", "phase", "queue", "pending_steers"]);
    vendor.respond(answer(&["Hello", " there."], 12, 3));
    let turn = socket.chat("m1", "Say hello").await;
    assert!(turn.iter().any(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. })), "{turn:?}");

    let sent = &vendor.requests()[0];
    assert_eq!((sent.uri.path(), sent.header("x-api-key")), ("/v1/messages", Some("sk-ant-test")));
    let body = sent.json();
    assert_eq!(body["system"], SYSTEM_PROMPT);
    assert_eq!(body["model"], "claude-opus-4-8");
    assert!(body["messages"][0].to_string().contains("Say hello"), "{body}");

    // Live equals cold: the live tree's transcript is what the database
    // holds, which the history route reads without a session.
    let live = socket.live().await;
    assert_eq!(kinds(&live), ["user", "text", "response"]);
    assert_eq!(last_text(&live), "Hello there.");
    let cold = transcript(&backend, &master, FIRST).await;
    assert_eq!(cold.blocks, live);
    assert!(cold.failures.is_none() && cold.subagents.is_empty());

    let summary = summaries(&backend, &master).await.remove(0);
    assert_eq!((summary.status, summary.unread), (ConversationStatus::Completed, true));
    assert!(summary.revision > 0);
    assert_eq!(
        (summary.provider_id.as_ref().map(|id| id.as_str()), summary.model_id.as_deref()),
        (Some(provider.as_str()), Some("claude-opus-4-8"))
    );
    let beyond = backend
        .post(&format!("/api/conversations/{FIRST}/read"), Some(&master), json!({ "revision": summary.revision + 1 }))
        .await;
    assert_eq!(beyond.refusal(), (StatusCode::CONFLICT, ErrorCode::InvalidRevision));
    let read = backend
        .post(&format!("/api/conversations/{FIRST}/read"), Some(&master), json!({ "revision": summary.revision }))
        .await;
    assert_eq!(read.status, StatusCode::NO_CONTENT);
    let older = backend.post(&format!("/api/conversations/{FIRST}/read"), Some(&master), json!({ "revision": 0 })).await;
    assert_eq!(older.status, StatusCode::NO_CONTENT);
    let summary = summaries(&backend, &master).await.remove(0);
    assert_eq!((summary.read_revision, summary.unread), (summary.revision, false));

    // The request is metered: its usage is a ledger row.
    let totals = usage(&backend, &master).await.totals;
    assert_eq!(
        (totals.len(), totals[0].requests, totals[0].input_tokens, totals[0].output_tokens),
        (1, 1, 12, 3)
    );
    assert_eq!((totals[0].provider_id.as_str(), totals[0].model_id.as_str()), (provider.as_str(), "claude-opus-4-8"));

    // A reload opens the same history, and a later message continues it.
    drop(socket);
    let mut reloaded = Socket::connect(&backend, &master, FIRST).await;
    let handshake = reloaded.open(&model).await;
    let ServerFrame::TranscriptReset { blocks, .. } = &handshake[1] else {
        panic!("{handshake:?}");
    };
    assert_eq!(*blocks, live);
    vendor.respond(answer(&["Again."], 20, 2));
    reloaded.chat("m2", "Once more").await;
    let replayed = vendor.requests()[1].json();
    assert_eq!(replayed["messages"].as_array().unwrap().len(), 3, "{replayed}");
    assert_eq!(kinds(&transcript(&backend, &master, FIRST).await.blocks).len(), 6);
    backend.close().await;
}

#[tokio::test]
async fn a_client_that_falls_behind_is_closed_as_lagging_and_a_reopen_adopts_the_running_tree() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new();
    harness.conversations.outbox_frames = 16;
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let model = model_of(&provider, "claude-opus-4-8");
    let deltas: Vec<String> = (0..200).map(|index| format!("{index} ")).collect();
    let deltas: Vec<&str> = deltas.iter().map(String::as_str).collect();
    vendor.respond(answer(&deltas, 5, 200));

    let mut slow = Socket::connect(&backend, &master, FIRST).await;
    slow.open(&model).await;
    slow.send(&send("m1", "Count")).await;
    assert_eq!(slow.closed().await, Some(4001));

    let mut again = Socket::connect(&backend, &master, FIRST).await;
    again.open(&model).await;
    let blocks = loop {
        let blocks = again.live().await;
        if blocks.iter().any(|block| matches!(block, Block::Response(_))) {
            break blocks;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(last_text(&blocks).ends_with("199 "), "the turn ran to its end");
    assert_eq!(vendor.requests().len(), 1, "the reopen adopted the tree, which asked once");
    backend.close().await;
}

#[tokio::test]
async fn a_second_socket_takes_the_conversation_over_and_the_first_can_take_it_back() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let model = model_of(&provider, "claude-opus-4-8");

    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open(&model).await;
    let mut second = Socket::connect(&backend, &master, FIRST).await;
    second.open(&model).await;
    assert_eq!(first.frame().await, ServerFrame::Closed);
    first.send(&send("m1", "hi")).await;
    assert_eq!(
        first.frame().await,
        ServerFrame::Rejected {
            command: ClientFrameKind::Send,
            reason: "No session is open".into()
        }
    );
    first.open(&model).await;
    assert_eq!(second.frame().await, ServerFrame::Closed);
    vendor.respond(answer(&["back"], 1, 1));
    first.chat("m2", "I am back").await;
    assert_eq!(last_text(&first.live().await), "back");
    backend.close().await;
}

#[tokio::test]
async fn the_frames_the_backend_refuses_never_reach_the_session() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;

    // A frame outside its schema is answered, and the socket stays open.
    socket.send_text(json!({ "type": "send", "messageId": "m1" }).to_string()).await;
    let ServerFrame::Error { code, .. } = socket.frame().await else {
        panic!("an invalid frame is answered with an error");
    };
    assert_eq!(code.as_deref(), Some("invalid_frame"));
    socket.send(&ClientFrame::Open { model: model_of("someone-elses", "m") }).await;
    let ServerFrame::Error { code, .. } = socket.frame().await else {
        panic!("a provider outside the user's scope is refused");
    };
    assert_eq!(code.as_deref(), Some("provider_not_found"));
    let summary = summaries(&backend, &master).await.remove(0);
    assert_eq!(summary.provider_id, None, "nothing was recorded");
    socket.open(&model_of(&provider, "claude-opus-4-8")).await;

    // Archived, the conversation takes no frame but its close, and no
    // socket.
    harness
        .control_database()
        .execute("UPDATE conversations SET archived = 1 WHERE id = ?1", [FIRST])
        .unwrap();
    socket.send(&send("m1", "hi")).await;
    let ServerFrame::Error { code, .. } = socket.frame().await else {
        panic!("an archived conversation refuses a message");
    };
    assert_eq!(code.as_deref(), Some("conversation_archived"));
    let edit = ClientFrame::EditAndSend {
        request: EditRequest {
            operation_id: "edit-1".try_into().unwrap(),
            target_block_id: "user-1".try_into().unwrap(),
            version: TranscriptVersion {
                epoch: "epoch".into(),
                revision: 1,
            },
            content: client_text("edited"),
        },
    };
    socket.send(&edit).await;
    assert_eq!(
        socket.frame().await,
        ServerFrame::EditResult {
            operation_id: "edit-1".try_into().unwrap(),
            outcome: EditOutcome::Rejected {
                reason: "Restore the conversation before writing to it".into()
            },
        }
    );
    assert!(vendor.requests().is_empty());
    assert_eq!(Socket::try_connect(&backend, &master, FIRST).await.err(), Some(409));
    socket.send(&ClientFrame::Close {}).await;
    assert_eq!(socket.frame().await, ServerFrame::Closed);

    // A message that is not JSON closes the socket.
    socket.send_text("not json".into()).await;
    assert_eq!(socket.closed().await, Some(u16::from(CloseCode::Invalid)));
    let not_socket = backend.get(&format!("/api/conversations/{SECOND}/stream"), Some(&master)).await;
    assert_eq!(not_socket.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound));
    create(&backend, &master, SECOND).await;
    let not_socket = backend.get(&format!("/api/conversations/{SECOND}/stream"), Some(&master)).await;
    assert_eq!(not_socket.refusal(), (StatusCode::UPGRADE_REQUIRED, ErrorCode::UpgradeRequired));
    backend.close().await;
}

/// What the keyed family's runtimes saw: how many were built, and each
/// request's key and its place among its runtime's requests. The first
/// request waits until the test releases it.
struct Runs {
    runtimes: AtomicUsize,
    calls: Mutex<Vec<(String, usize)>>,
    released: watch::Sender<bool>,
}

impl Runs {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            runtimes: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            released: watch::Sender::new(false),
        })
    }

    fn calls(&self) -> Vec<(String, usize)> {
        self.calls.lock().unwrap().clone()
    }
}

/// An API-key family whose runtimes answer each request with the entry's
/// key and the runtime's request count.
struct Keyed(Arc<Runs>);

impl ProviderFamily for Keyed {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(key) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        Ok(Arc::new(KeyedProvider {
            id: args.entry_id,
            key: key.api_key.expose().to_owned(),
            runs: self.0.clone(),
        }))
    }
}

struct KeyedProvider {
    id: String,
    key: String,
    runs: Arc<Runs>,
}

impl Provider for KeyedProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        "Keyed"
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
        self.runs.runtimes.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(KeyedRuntime {
            key: self.key.clone(),
            requests: 0,
            runs: self.runs.clone(),
        }))
    }
}

struct KeyedRuntime {
    key: String,
    requests: usize,
    runs: Arc<Runs>,
}

impl ProviderRuntime for KeyedRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        self.requests += 1;
        let (key, count, runs) = (self.key.clone(), self.requests, self.runs.clone());
        let output = request.output_limit.map_or(0, |limit| u64::from(limit.get()));
        stream::once(async move {
            let first = {
                let mut calls = runs.calls.lock().unwrap();
                calls.push((key.clone(), count));
                calls.len() == 1
            };
            if first {
                let mut released = runs.released.subscribe();
                let _ = released.wait_for(|released| *released).await;
            }
            stream::iter([
                ProviderEvent::TextDelta(format!("{key}:{count}")),
                ProviderEvent::Response(TokenUsage {
                    input_tokens: 1,
                    output_tokens: output,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                }),
            ])
        })
        .flatten()
        .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self {
            key: self.key.clone(),
            requests: 0,
            runs: self.runs.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// A configured model of `output` tokens at most.
fn configured(output: u32) -> Value {
    json!({
        "id": "m", "displayName": "M", "contextWindow": 100000, "outputLimit": output,
        "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
    })
}

#[tokio::test]
async fn an_edit_of_the_entry_reaches_the_next_request_and_a_deleted_entry_refuses_inference() {
    let runs = Runs::new();
    let harness = Harness::new().with_families(FamilyRegistry::builtin().with("keyed", Keyed(runs.clone())));
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "keyed", "label": "Keyed", "apiKey": "old-key",
        "models": [configured(4_000)]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    // The selection the page sent names no output limit; the entry's
    // configured model does.
    socket.open(&model_of(&provider, "m")).await;

    // An edit while the first request runs: that request finishes with the
    // runtime it started with, and the next one gets a new runtime.
    socket.send(&send("m1", "before the edit")).await;
    crate::support::eventually("the first request runs", || async { runs.calls().len() == 1 }).await;
    let path = format!("/api/providers/{provider}");
    let edited = backend
        .patch(&path, &master, json!({ "apiKey": "new-key", "models": [configured(8_000)] }))
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", String::from_utf8_lossy(&edited.body));
    runs.released.send_replace(true);
    socket.until_idle().await;
    socket.chat("m2", "after the edit").await;
    socket.chat("m3", "the same entry").await;
    assert_eq!(
        runs.calls(),
        [("old-key".to_owned(), 1), ("new-key".to_owned(), 1), ("new-key".to_owned(), 2)]
    );
    assert_eq!(runs.runtimes.load(Ordering::SeqCst), 2, "an unchanged entry keeps its runtime");
    let totals = usage(&backend, &master).await.totals;
    assert_eq!((totals[0].requests, totals[0].output_tokens), (3, 4_000 + 8_000 + 8_000), "{totals:?}");

    // A deleted entry refuses the next request before any vendor.
    let deleted = backend.delete(&path, &master).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let turn = socket.chat("m4", "after the deletion").await;
    let refused = turn.iter().any(|frame| {
        matches!(frame, ServerFrame::Error { message, .. } if message.contains("is no longer available to this conversation"))
    });
    assert!(refused, "{turn:?}");
    assert_eq!(runs.calls().len(), 3);
    assert_eq!(usage(&backend, &master).await.totals[0].requests, 3);
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    assert_eq!(kinds(&blocks).last().map(String::as_str), Some("error"));
    backend.close().await;
}

#[tokio::test]
async fn a_request_over_the_rate_limit_fails_without_reaching_the_vendor() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new();
    harness.conversations.requests_per_minute = 1;
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model_of(&provider, "claude-opus-4-8")).await;

    vendor.respond(answer(&["one"], 1, 1));
    socket.chat("m1", "first").await;
    let turn = socket.chat("m2", "second").await;

    let limited = turn.iter().any(|frame| matches!(frame, ServerFrame::Error { code, .. } if code.as_deref() == Some("rate_limited")));
    assert!(limited, "{turn:?}");
    assert_eq!(vendor.requests().len(), 1);
    assert_eq!(usage(&backend, &master).await.totals[0].requests, 1);
    let summary = summaries(&backend, &master).await.remove(0);
    assert_eq!(summary.status, ConversationStatus::Error);
    backend.close().await;
}

#[tokio::test]
async fn a_shutdown_in_the_middle_of_a_turn_saves_its_interruption_and_the_next_start_serves_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let model = model_of(&provider, "claude-opus-4-8");
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model).await;
    vendor.respond(answer(&["first"], 4, 1));
    socket.chat("m1", "a first message").await;
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    socket.send(&send("m2", "take your time")).await;
    vendor.received(2).await;

    backend.close().await;
    assert_eq!(socket.closed().await, Some(u16::from(CloseCode::Away)));

    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, crate::support::MASTER_PASSWORD).await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    assert_eq!(kinds(&blocks), ["user", "text", "response", "user", "error"]);
    let Some(Block::Error(record)) = blocks.last() else { unreachable!() };
    assert_eq!(record.code.as_deref(), Some("interrupted"));
    let summary = summaries(&backend, &master).await.remove(0);
    assert_eq!(summary.status, ConversationStatus::Interrupted);
    // The ledger carries its rows over the restart.
    assert_eq!(usage(&backend, &master).await.totals[0].requests, 1);

    // The next turn runs on the restored tree.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model).await;
    vendor.respond(answer(&["done"], 1, 1));
    socket.chat("m3", "go on").await;
    assert_eq!(last_text(&socket.live().await), "done");
    backend.close().await;
}

#[tokio::test]
async fn the_page_receives_what_the_provider_reads_from_an_error_blocks_record() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model_of(&provider, "claude-opus-4-8")).await;
    vendor.respond(
        MockResponse::status(401)
            .header("retry-after", "120")
            .chunk(r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
    );
    let turn = socket.chat("m1", "hi").await;

    // The frame that brings the error block carries the provider's reading
    // of its record: the vendor's wait, counted from when it failed.
    let read = turn.iter().find_map(|frame| match frame {
        ServerFrame::TranscriptPatch { failures: Some(failures), .. } => Some(failures.clone()),
        _ => None,
    });
    let read = read.unwrap_or_else(|| panic!("no frame carried failure facts: {turn:?}"));
    let blocks = transcript(&backend, &master, FIRST).await;
    let Some(Block::Error(error)) = blocks.blocks.last() else {
        panic!("the turn failed: {:?}", blocks.blocks);
    };
    let retry_at: Timestamp = "2026-09-24T08:02:00.000Z".parse().unwrap();
    assert_eq!(read.get(&error.id).and_then(|facts| facts.retry_at), Some(retry_at));
    assert_eq!(blocks.failures, Some(read));
    let others = turn
        .iter()
        .filter(|frame| matches!(frame, ServerFrame::TranscriptPatch { failures: Some(_), .. }))
        .count();
    assert_eq!(others, 1, "only the frame that brings the block carries facts");

    // With its entry gone, the record shows without facts.
    backend.delete(&format!("/api/providers/{provider}"), &master).await;
    assert_eq!(transcript(&backend, &master, FIRST).await.failures, None);
    backend.close().await;
}

#[tokio::test]
async fn a_deepseek_tool_continuation_sends_the_reasoning_back_to_the_compatible_endpoint() {
    let vendor = MockVendor::start().await;
    let catalog = json!({
        "deepseek": {
            "id": "deepseek", "name": "DeepSeek", "npm": "@ai-sdk/openai-compatible",
            "api": "https://api.deepseek.com", "models": {}
        }
    });
    vendor.respond_at(
        "/api.json",
        MockResponse::status(200).header("etag", "\"fixture\"").chunk(catalog.to_string()),
    );
    let harness = Harness::new().with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "vendor", "vendorId": "deepseek", "label": "DeepSeek", "apiKey": "fake-key",
        "baseUrl": vendor.url("/v1"), "models": [configured(8_000)]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model_of(&provider, "m")).await;
    let stream = |delta: Value| {
        MockResponse::event_stream(format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({ "choices": [{ "delta": delta }] })
        ))
    };
    vendor.respond(stream(json!({
        "reasoning_content": "Read the current directory.",
        "tool_calls": [{ "index": 0, "id": "call-1", "function": {
            "name": "shell_exec", "arguments": json!({ "script": "pwd", "timeoutMs": 1000 }).to_string()
        } }]
    })));
    vendor.respond(stream(json!({ "content": "done" })));

    socket.chat("m1", "read the current directory").await;

    let requests: Vec<Value> = vendor
        .requests()
        .iter()
        .filter(|request| request.uri.path() == "/v1/chat/completions")
        .map(|request| request.json())
        .collect();
    assert_eq!(requests.len(), 2, "the tool's result was sent back");
    let messages = requests[1]["messages"].as_array().unwrap();
    let asked = messages
        .iter()
        .find(|message| message.get("tool_calls").is_some())
        .unwrap_or_else(|| panic!("the continuation replays the tool call: {messages:?}"));
    assert_eq!(asked["reasoning_content"], "Read the current directory.");
    assert_eq!(last_text(&socket.live().await), "done");
    backend.close().await;
}
