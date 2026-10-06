//! Conversations (`web-api.md` § Conversation creation and Fork, § Sidebar
//! mutations, read state and page synchronization; `runtime.md` § Frame
//! protocol): creation under the web app's id, the list and the product
//! state, a chat over the socket with an Anthropic endpoint the test
//! scripts, a reload whose history is what the database holds, a client that
//! falls behind, the frames the backend refuses, a user's context limit
//! on a model, provider edits
//! and deletion at the inference boundary, the rate limit, a shutdown in the
//! middle of a turn and one that a page which stopped reading cannot hold
//! up, and the patches and batches of the sidebar. No test calls a real
//! model.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demi_agent_server::testing::client_text;
use demi_backend_providers::llm::families::{
    FamilyArgs, FamilyCredential, FamilyError, ProviderFamily,
};
use demi_conversation_socket_protocol::{
    ClientFrame, EditOutcome, EditRequest, ServerFrame, TranscriptVersion,
};
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_provider_common::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderEvent, ProviderRun,
    ProviderRuntime, RequestLimits, RuntimeEnv, RuntimeError,
};
use demi_shared_types::{
    AuthState, Block, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, SessionPhase, Timestamp, TokenUsage, TurnId,
};
use demi_web_api_protocol::conversations::{
    BatchAnswer, BatchResult, ConversationStatus, ConversationSummary, CreatedConversation,
    ConversationUpdate, Conversations, FieldResult, ModelSettings, PatchField, Transcript,
};
use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
use demi_web_api_protocol::providers::{CredentialKind, ProviderAnswer};
use demi_web_api_protocol::usage::UsageTotals;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{SinkExt as _, StreamExt as _, stream};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};

use crate::support::{Harness, MASTER_EMAIL, PATIENCE, Paired, Session, TestBackend};

/// The conversation ids the tests create.
pub(crate) const FIRST: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b";
pub(crate) const SECOND: &str = "7d1c2e3f-4a5b-4c1e-9d2b-0b6f7f3e8f3a";
pub(crate) const THIRD: &str = "5a4b3c2d-1e0f-4a1b-8c2d-3e4f5a6b7c8d";

/// What a conversation socket delivered next.
#[derive(Debug)]
enum Received {
    Frame(ServerFrame),
    /// The backend closed the socket, with its close code.
    Closed(Option<u16>),
}

/// A page's conversation socket.
pub(crate) struct Socket {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    /// How long it waits for the next message before the test fails as
    /// hung: ten seconds, longer where a real Cloud boots within a turn.
    pub(crate) patience: Duration,
}

impl Socket {
    pub(crate) async fn connect(
        backend: &TestBackend,
        session: &Session,
        conversation: &str,
    ) -> Self {
        match Self::try_connect(backend, session, conversation).await {
            Ok(socket) => socket,
            Err(status) => panic!("the stream refused the upgrade with {status}"),
        }
    }

    /// The socket, opened from a page of the product, or the status the
    /// route answered instead of upgrading.
    pub(crate) async fn try_connect(
        backend: &TestBackend,
        session: &Session,
        conversation: &str,
    ) -> Result<Self, u16> {
        Self::try_connect_from(backend, session, conversation, &backend.url)
            .await
            .map_err(|(status, _)| status)
    }

    /// The socket, opened from a page at `origin`, or the status and error
    /// code the route answered instead of upgrading.
    pub(crate) async fn try_connect_from(
        backend: &TestBackend,
        session: &Session,
        conversation: &str,
        origin: &str,
    ) -> Result<Self, (u16, ErrorCode)> {
        let url = backend.ws_url(&format!("/api/conversations/{conversation}/stream"));
        let mut request = url.into_client_request().unwrap();
        let headers = request.headers_mut();
        headers.insert("cookie", session.cookie.parse().unwrap());
        headers.insert("origin", origin.parse().unwrap());
        match tokio_tungstenite::connect_async(request).await {
            Ok((socket, _)) => Ok(Self {
                socket,
                patience: Duration::from_secs(10),
            }),
            Err(tungstenite::Error::Http(response)) => {
                let body = response.body().as_deref().unwrap_or_default();
                let error: ErrorBody = serde_json::from_slice(body)
                    .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(body)));
                Err((response.status().as_u16(), error.code))
            }
            Err(error) => panic!("the socket did not connect: {error}"),
        }
    }

    pub(crate) async fn send(&mut self, frame: &ClientFrame) {
        self.send_text(serde_json::to_string(frame).unwrap()).await;
    }

    pub(crate) async fn send_text(&mut self, text: String) {
        self.socket.send(Message::Text(text.into())).await.unwrap();
    }

    /// Sends `text`, which the backend may refuse while it is still on its
    /// way: the socket's failure then reaches the send first, as a broken
    /// pipe or a reset, and the next read finds the socket closed.
    pub(crate) async fn send_refused_text(&mut self, text: String) {
        match self.socket.send(Message::Text(text.into())).await {
            Ok(()) => {}
            Err(tokio_tungstenite::tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                ) => {}
            Err(error) => panic!("the send failed otherwise: {error}"),
        }
    }

    async fn next(&mut self) -> Received {
        loop {
            let message = tokio::time::timeout(self.patience, self.socket.next())
                .await
                .unwrap_or_else(|_| panic!("the socket delivers within {:?}", self.patience));
            match message {
                Some(Ok(Message::Text(text))) => {
                    let frame = serde_json::from_str(text.as_str())
                        .unwrap_or_else(|error| panic!("{error}: {text}"));
                    return Received::Frame(frame);
                }
                Some(Ok(Message::Close(close))) => {
                    return Received::Closed(close.map(|close| u16::from(close.code)));
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return Received::Closed(None),
            }
        }
    }

    pub(crate) async fn frame(&mut self) -> ServerFrame {
        match self.next().await {
            Received::Frame(frame) => frame,
            Received::Closed(code) => panic!("the socket closed with {code:?}"),
        }
    }

    /// The frames up to and including the first `done` accepts.
    pub(crate) async fn until(&mut self, done: impl Fn(&ServerFrame) -> bool) -> Vec<ServerFrame> {
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

    /// Opens the conversation, with the model its record holds, and reads
    /// the handshake.
    pub(crate) async fn open(&mut self) -> Vec<ServerFrame> {
        self.send(&ClientFrame::Open {}).await;
        let handshake = self
            .until(|frame| matches!(frame, ServerFrame::PendingSteers { .. }))
            .await;
        assert_eq!(
            handshake.first(),
            Some(&ServerFrame::Opened),
            "{handshake:?}"
        );
        handshake
    }

    /// Sends a message and reads the frames of its turn, to the phase that
    /// says it ended.
    pub(crate) async fn chat(&mut self, id: &str, text: &str) -> Vec<ServerFrame> {
        self.send(&send(id, text)).await;
        self.until_idle().await
    }

    /// The frames up to the idle phase that follows a running one.
    pub(crate) async fn until_idle(&mut self) -> Vec<ServerFrame> {
        let mut ran = false;
        let mut frames = Vec::new();
        loop {
            let frame = self.frame().await;
            let idle = match &frame {
                ServerFrame::Phase {
                    phase: SessionPhase::Running,
                } => {
                    ran = true;
                    false
                }
                ServerFrame::Phase {
                    phase: SessionPhase::Idle,
                } => ran,
                _ => false,
            };
            frames.push(frame);
            if idle {
                return frames;
            }
        }
    }

    /// Stops the running turn, and reads its frames to both the answer of
    /// the stop and the idle phase, which arrive in either order.
    pub(crate) async fn stop(&mut self) {
        self.send(&ClientFrame::Abort {}).await;
        let mut answered = false;
        let mut idle = false;
        while !(answered && idle) {
            match self.frame().await {
                ServerFrame::AbortResult { .. } => answered = true,
                ServerFrame::Phase {
                    phase: SessionPhase::Idle,
                } => idle = true,
                _ => {}
            }
        }
    }

    /// The live transcript, as a fresh reset sends it.
    pub(crate) async fn live(&mut self) -> Vec<Block> {
        self.send(&ClientFrame::SyncTranscript {}).await;
        let frames = self
            .until(|frame| matches!(frame, ServerFrame::TranscriptReset { .. }))
            .await;
        match frames.into_iter().last() {
            Some(ServerFrame::TranscriptReset { blocks, .. }) => blocks,
            other => unreachable!("{other:?}"),
        }
    }

    /// The close code, skipping the frames before it.
    pub(crate) async fn closed(&mut self) -> Option<u16> {
        loop {
            if let Received::Closed(code) = self.next().await {
                return code;
            }
        }
    }
}

pub(crate) fn send(id: &str, text: &str) -> ClientFrame {
    ClientFrame::Send {
        message_id: TurnId::try_from(id).unwrap(),
        content: client_text(text),
    }
}

/// A Messages API stream that answers `deltas`, in that many pieces, and
/// reports `input` and `output` tokens.
pub(crate) fn answer(deltas: &[&str], input: u64, output: u64) -> MockResponse {
    message(
        vec![text_block(0, deltas)],
        "end_turn",
        json!({ "input_tokens": input, "output_tokens": 0 }),
        output,
    )
}

/// A Messages API stream that calls the tool `name` with `input`.
pub(crate) fn tool_use(id: &str, name: &str, input: &Value) -> MockResponse {
    let usage = json!({ "input_tokens": 1, "output_tokens": 0 });
    message(
        vec![tool_use_block(0, id, name, input)],
        "tool_use",
        usage,
        1,
    )
}

/// The frames that open a text block at `index` and fill it with `deltas`.
pub(crate) fn text_block(index: usize, deltas: &[&str]) -> Vec<Value> {
    let mut frames = vec![
        json!({ "type": "content_block_start", "index": index, "content_block": { "type": "text", "text": "" } }),
    ];
    for delta in deltas {
        frames.push(json!({ "type": "content_block_delta", "index": index, "delta": { "type": "text_delta", "text": delta } }));
    }
    frames
}

/// The frames that open a thinking block at `index`, stream `text` and
/// sign it.
pub(crate) fn thinking_block(index: usize, text: &str, signature: &str) -> Vec<Value> {
    vec![
        json!({ "type": "content_block_start", "index": index,
            "content_block": { "type": "thinking", "thinking": "", "signature": "" } }),
        json!({ "type": "content_block_delta", "index": index, "delta": { "type": "thinking_delta", "thinking": text } }),
        json!({ "type": "content_block_delta", "index": index, "delta": { "type": "signature_delta", "signature": signature } }),
    ]
}

/// The frames of a call of the tool `name` with `input`, at `index`.
pub(crate) fn tool_use_block(index: usize, id: &str, name: &str, input: &Value) -> Vec<Value> {
    vec![
        json!({ "type": "content_block_start", "index": index,
            "content_block": { "type": "tool_use", "id": id, "name": name, "input": {} } }),
        json!({ "type": "content_block_delta", "index": index,
            "delta": { "type": "input_json_delta", "partial_json": input.to_string() } }),
    ]
}

/// The text of the tool result `id` that a Messages API `request` carries.
pub(crate) fn tool_result(request: &Value, id: &str) -> String {
    let result = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .find(|block| block["type"] == "tool_result" && block["tool_use_id"] == id)
        .unwrap_or_else(|| panic!("no result of {id}: {request}"));
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

/// One message of the content blocks whose frames `blocks` open and fill,
/// each closed after its frames; `usage` is the message's usage at its
/// start, and `output` its output tokens at its end.
pub(crate) fn message(
    blocks: Vec<Vec<Value>>,
    stop_reason: &str,
    usage: Value,
    output: u64,
) -> MockResponse {
    let mut frames = vec![message_start(usage)];
    for block in blocks {
        let index = block[0]["index"].clone();
        frames.extend(block);
        frames.push(json!({ "type": "content_block_stop", "index": index }));
    }
    frames.push(json!({ "type": "message_delta", "delta": { "stop_reason": stop_reason }, "usage": { "output_tokens": output } }));
    frames.push(json!({ "type": "message_stop" }));
    MockResponse::event_stream(events(&frames))
}

/// The frame that starts a message whose usage at its start is `usage`.
pub(crate) fn message_start(usage: Value) -> Value {
    json!({ "type": "message_start", "message": {
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-4-8", "content": [],
        "usage": usage } })
}

/// `frames` as the text of a Messages API event stream.
pub(crate) fn events(frames: &[Value]) -> String {
    frames
        .iter()
        .map(|frame| {
            format!(
                "event: {}\ndata: {frame}\n\n",
                frame["type"].as_str().unwrap()
            )
        })
        .collect()
}

/// A new API-key entry of the master's.
pub(crate) async fn entry(backend: &TestBackend, master: &Session, body: Value) -> String {
    let created = backend.post("/api/providers", Some(master), body).await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    created
        .json::<ProviderAnswer>()
        .provider
        .id
        .as_str()
        .to_owned()
}

/// An Anthropic entry whose endpoint is `vendor`.
pub(crate) async fn anthropic(
    backend: &TestBackend,
    master: &Session,
    vendor: &MockVendor,
) -> String {
    anthropic_at(backend, master, vendor, "").await
}

/// An Anthropic entry whose endpoint is `vendor` under `prefix`, so that
/// conversations on entries of their own get their own answers however
/// their requests interleave.
pub(crate) async fn anthropic_at(
    backend: &TestBackend,
    master: &Session,
    vendor: &MockVendor,
    prefix: &str,
) -> String {
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url(&format!("{prefix}/v1"))
    });
    entry(backend, master, body).await
}

pub(crate) async fn create(
    backend: &TestBackend,
    session: &Session,
    id: &str,
) -> ConversationSummary {
    let created = backend
        .post("/api/conversations", Some(session), json!({ "id": id }))
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    created.json::<CreatedConversation>().conversation
}

/// Chooses the model `model` of the entry `provider` for the conversation
/// `id`, as a page's first send does, and answers the conversation.
pub(crate) async fn choose(
    backend: &TestBackend,
    session: &Session,
    id: &str,
    provider: &str,
    model: &str,
) -> ConversationSummary {
    let body = json!({ "model": { "providerId": provider, "modelId": model } });
    let chosen = backend
        .patch(&format!("/api/conversations/{id}"), session, body)
        .await;
    assert_eq!(
        chosen.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&chosen.body)
    );
    chosen.json::<ConversationUpdate>().conversation
}

/// Makes the master's conversation `id` work in a new `work` directory of a
/// paired device's real runner, as a target switch would leave it; answers
/// the device and that directory.
pub(crate) async fn on_device(
    harness: &Harness,
    backend: &TestBackend,
    master: &Session,
    id: &str,
) -> (Paired, String) {
    let paired = backend.pair(master, "laptop").await;
    let root = working_on(harness, &paired, id);
    (paired, root)
}

/// Makes the master's conversation `id` work in a new `work` directory of
/// `paired`, as a target switch would leave it; answers that directory.
pub(crate) fn working_on(harness: &Harness, paired: &Paired, id: &str) -> String {
    let root = format!("{}/work", paired.runner.home());
    std::fs::create_dir_all(&root).unwrap();
    harness
        .control_database()
        .execute(
            "UPDATE conversations SET target_kind = 'device', target_device_id = ?1, target_path = ?2 WHERE id = ?3",
            rusqlite::params![paired.id(), root, id],
        )
        .unwrap();
    root
}

pub(crate) async fn summaries(
    backend: &TestBackend,
    session: &Session,
) -> Vec<ConversationSummary> {
    let listed = backend.get("/api/conversations", Some(session)).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    listed.json::<Conversations>().conversations
}

/// The conversation's summary, as the sidebar lists it. Once a socket has
/// seen the idle phase, the save that ended the turn has committed
/// (`runtime.md` § A turn), so the summary and the database hold the turn.
pub(crate) async fn summary(
    backend: &TestBackend,
    session: &Session,
    id: &str,
) -> ConversationSummary {
    summaries(backend, session)
        .await
        .into_iter()
        .find(|summary| summary.id.as_str() == id)
        .expect("the conversation is listed")
}

pub(crate) async fn transcript(backend: &TestBackend, session: &Session, id: &str) -> Transcript {
    let read = backend
        .get(
            &format!("/api/conversations/{id}/transcript"),
            Some(session),
        )
        .await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&read.body)
    );
    read.json()
}

pub(crate) async fn usage(backend: &TestBackend, session: &Session) -> UsageTotals {
    backend.get("/api/usage", Some(session)).await.json()
}

/// Each block's type.
pub(crate) fn kinds(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .map(|block| {
            serde_json::to_value(block).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

/// The text of the last text block.
pub(crate) fn last_text(blocks: &[Block]) -> String {
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
async fn a_conversation_is_created_once_under_the_id_the_web_app_chose_and_listed_for_its_owner() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user(
        "ana@example.test",
        "ana-pass-1",
        demi_web_api_protocol::auth::Role::User,
    );

    let created = create(&backend, &master, FIRST).await;
    assert_eq!(
        (
            created.id.as_str(),
            created.title.as_str(),
            created.status,
            created.revision,
            created.unread
        ),
        (
            FIRST,
            "New conversation",
            ConversationStatus::Idle,
            0,
            false
        )
    );
    assert_eq!(created.cwd, format!("/home/demi/sessions/{FIRST}"));
    assert_eq!(
        serde_json::to_value(&created.target).unwrap(),
        json!({ "kind": "cloud" })
    );

    // A retry, in any spelling, finds the one it created.
    for spelling in [FIRST.to_owned(), FIRST.to_uppercase()] {
        let again = backend
            .post(
                "/api/conversations",
                Some(&master),
                json!({ "id": spelling }),
            )
            .await;
        assert_eq!(again.status, StatusCode::OK);
        assert_eq!(again.json::<CreatedConversation>().conversation, created);
    }
    let second = create(&backend, &master, SECOND).await;
    let listed: Vec<String> = summaries(&backend, &master)
        .await
        .into_iter()
        .map(|summary| summary.id.as_str().to_owned())
        .collect();
    assert_eq!(listed, [SECOND, FIRST], "the newest first");
    let archived = backend
        .get("/api/conversations?archived=true", Some(&master))
        .await;
    assert!(archived.json::<Conversations>().conversations.is_empty());
    let malformed = backend
        .get("/api/conversations?archived=1", Some(&master))
        .await;
    assert_eq!(
        malformed.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery)
    );
    let state = backend.sync(&master).await.snapshot().await;
    assert_eq!(state.conversations, [second, created]);
    assert!(transcript(&backend, &master, FIRST).await.blocks.is_empty());

    // Another user can neither take the id nor reach the conversation.
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let taken = backend
        .post(
            "/api/conversations",
            Some(&ana),
            json!({ "id": FIRST.to_uppercase() }),
        )
        .await;
    assert_eq!(
        taken.refusal(),
        (StatusCode::CONFLICT, ErrorCode::IdUnavailable)
    );
    for path in [
        format!("/api/conversations/{FIRST}/transcript"),
        "/api/conversations/not-a-uuid/transcript".into(),
    ] {
        let foreign = backend.get(&path, Some(&ana)).await;
        assert_eq!(
            foreign.refusal(),
            (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound),
            "{path}"
        );
    }
    assert!(summaries(&backend, &ana).await.is_empty());
    assert_eq!(
        Socket::try_connect(&backend, &ana, FIRST).await.err(),
        Some(404)
    );
    for body in [
        json!({ "id": "conversation-1" }),
        json!({ "id": FIRST, "archived": true }),
        json!({}),
    ] {
        let refused = backend
            .post("/api/conversations", Some(&ana), body.clone())
            .await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{body}"
        );
    }
    backend.close().await;
}

// About a second here: two runners pair.
#[tokio::test]
async fn a_conversation_starts_with_its_settings_and_hosts_from_one_request_and_a_refused_part_creates_nothing()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url("/v1"), "models": [leveled("m", Some("priority"))]
    });
    let provider = entry(&backend, &master, body).await;
    let laptop = backend.pair(&master, "laptop").await;
    let ci = backend.pair(&master, "ci").await;
    let on_laptop = json!({ "kind": "device", "deviceId": laptop.id(), "path": "/work" });
    let model = json!({ "providerId": provider, "modelId": "m" });
    let start = json!({
        "id": FIRST, "title": "Plans", "pinned": true, "target": on_laptop, "model": model,
        "thinkingEffort": "high", "hosts": [{ "deviceId": ci.id(), "name": "build" }]
    });

    let created = backend
        .post("/api/conversations", Some(&master), start.clone())
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let created = created.json::<CreatedConversation>();
    let conversation = &created.conversation;
    assert_eq!(
        (conversation.title.as_str(), conversation.pinned),
        ("Plans", true)
    );
    assert_eq!(serde_json::to_value(&conversation.target).unwrap(), on_laptop);
    assert_eq!(
        conversation.model,
        Some(ModelSettings {
            provider_id: provider.as_str().try_into().unwrap(),
            model_id: "m".into(),
            thinking_effort: Some("high".into()),
            service_tier_id: None,
        })
    );
    let hosts: Vec<(&str, &str)> = created
        .hosts
        .iter()
        .map(|host| (host.device_id.as_str(), host.name.as_str()))
        .collect();
    assert_eq!(hosts, [(ci.id(), "build")]);
    assert_eq!(summary(&backend, &master, FIRST).await, created.conversation);

    // A retry finds it as it is and applies nothing.
    let mut again = start.clone();
    again["title"] = json!("Other plans");
    let retried = backend
        .post("/api/conversations", Some(&master), again)
        .await;
    assert_eq!(retried.status, StatusCode::OK);
    assert_eq!(retried.json::<CreatedConversation>(), created);

    // A part that is refused refuses the creation, and nothing is made.
    let refused = |change: Value| {
        let mut body = start.clone();
        body["id"] = json!(SECOND);
        for (field, value) in change.as_object().unwrap() {
            body[field] = value.clone();
        }
        let backend = &backend;
        let master = &master;
        async move { backend.post("/api/conversations", Some(master), body).await }
    };
    let primary = refused(json!({ "hosts": [{ "deviceId": laptop.id() }] })).await;
    assert_eq!(
        primary.refusal(),
        (StatusCode::CONFLICT, ErrorCode::HostIsPrimary)
    );
    let unlisted = refused(json!({ "model": { "providerId": provider, "modelId": "x" } })).await;
    assert_eq!(
        unlisted.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ModelNotFound)
    );
    let ids: Vec<String> = summaries(&backend, &master)
        .await
        .into_iter()
        .map(|summary| summary.id.as_str().to_owned())
        .collect();
    assert_eq!(ids, [FIRST]);
    backend.close().await;
}

#[tokio::test]
async fn a_message_runs_over_the_socket_and_a_reload_shows_what_the_database_holds() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;

    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    let handshake = socket.open().await;
    let kinds_of: Vec<String> = handshake
        .iter()
        .map(|frame| {
            serde_json::to_value(frame).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        kinds_of,
        [
            "opened",
            "transcript_reset",
            "phase",
            "queue",
            "pending_steers"
        ]
    );
    vendor.respond(answer(&["Hello", " there."], 12, 3));
    let turn = socket.chat("m1", "Say hello").await;
    assert!(
        turn.iter()
            .any(|frame| matches!(frame, ServerFrame::TranscriptPatch { .. })),
        "{turn:?}"
    );

    let sent = &vendor.requests()[0];
    assert_eq!(
        (sent.uri.path(), sent.header("x-api-key")),
        ("/v1/messages", Some("sk-ant-test"))
    );
    let body = sent.json();
    let system = body["system"].to_string();
    assert!(
        system.contains("You are a coding agent. Use shell session tools"),
        "{system}"
    );
    assert!(
        system.contains("demi host"),
        "the backend's group is among the commands: {system}"
    );
    // Without the command packages the backend's own groups are
    // offered, and none of the packages'.
    assert!(system.contains("demi expose"), "{system}");
    assert!(
        !system.contains("demi file") && !system.contains("demi browser"),
        "{system}"
    );
    assert_eq!(body["model"], "claude-opus-4-8");
    assert!(
        body["messages"][0].to_string().contains("Say hello"),
        "{body}"
    );

    // Live equals cold once the page has seen the turn end: the live tree's
    // transcript is what the database holds, which the history route reads
    // without a session.
    let summary = summary(&backend, &master, FIRST).await;
    let live = socket.live().await;
    assert_eq!(kinds(&live), ["user", "text", "response"]);
    assert_eq!(last_text(&live), "Hello there.");
    let cold = transcript(&backend, &master, FIRST).await;
    assert_eq!(cold.blocks, live);
    assert!(cold.failures.is_none() && cold.subagents.is_empty());

    assert_eq!(
        (summary.status, summary.unread),
        (ConversationStatus::Completed, true)
    );
    assert!(summary.revision > 0);
    let model = summary
        .model
        .as_ref()
        .map(|model| (model.provider_id.as_str(), model.model_id.as_str()));
    assert_eq!(model, Some((provider.as_str(), "claude-opus-4-8")));
    let beyond = backend
        .post(
            &format!("/api/conversations/{FIRST}/read"),
            Some(&master),
            json!({ "revision": summary.revision + 1 }),
        )
        .await;
    assert_eq!(
        beyond.refusal(),
        (StatusCode::CONFLICT, ErrorCode::InvalidRevision)
    );
    let read = backend
        .post(
            &format!("/api/conversations/{FIRST}/read"),
            Some(&master),
            json!({ "revision": summary.revision }),
        )
        .await;
    assert_eq!(read.status, StatusCode::NO_CONTENT);
    let older = backend
        .post(
            &format!("/api/conversations/{FIRST}/read"),
            Some(&master),
            json!({ "revision": 0 }),
        )
        .await;
    assert_eq!(older.status, StatusCode::NO_CONTENT);
    let summary = summaries(&backend, &master).await.remove(0);
    assert_eq!(
        (summary.read_revision, summary.unread),
        (summary.revision, false)
    );

    // The request is metered: its usage is a ledger row.
    let totals = usage(&backend, &master).await.totals;
    assert_eq!(
        (
            totals.len(),
            totals[0].requests,
            totals[0].input_tokens,
            totals[0].output_tokens
        ),
        (1, 1, 12, 3)
    );
    assert_eq!(
        (totals[0].provider_id.as_str(), totals[0].model_id.as_str()),
        (provider.as_str(), "claude-opus-4-8")
    );

    // A reload opens the same history, and a later message continues it.
    drop(socket);
    let mut reloaded = Socket::connect(&backend, &master, FIRST).await;
    let handshake = reloaded.open().await;
    let ServerFrame::TranscriptReset { blocks, .. } = &handshake[1] else {
        panic!("{handshake:?}");
    };
    assert_eq!(*blocks, live);
    vendor.respond(answer(&["Again."], 20, 2));
    reloaded.chat("m2", "Once more").await;
    let replayed = vendor.requests()[1].json();
    assert_eq!(
        replayed["messages"].as_array().unwrap().len(),
        3,
        "{replayed}"
    );
    assert_eq!(
        kinds(&transcript(&backend, &master, FIRST).await.blocks).len(),
        6
    );
    backend.close().await;
}

/// `value` without the cache marks an Anthropic request carries, which
/// the vendor does not count as content: what the vendor caches.
fn unmarked(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, field)| (key.clone(), unmarked(field)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(unmarked).collect()),
        other => other.clone(),
    }
}

// A backend, a scripted vendor and four requests: about half a second.
#[tokio::test]
async fn a_request_the_vendor_refuses_as_too_large_compacts_and_goes_again_from_the_summary() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url("/v1"),
        "models": [{
            "id": "model-a", "displayName": "A", "contextWindow": 200000, "outputLimit": 8000,
            "thinkingEfforts": [], "acceptedExtensions": [], "fastTier": null
        }]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "model-a").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["First answer."], 12, 3));
    socket.chat("m1", "First question").await;

    // The second request is refused as too large, as a compatible endpoint
    // with a smaller limit than the API's would.
    let refusal = json!({ "type": "error", "error": { "type": "request_too_large", "message": "Request exceeds the maximum size" } });
    vendor.respond(
        MockResponse::status(413)
            .header("content-type", "application/json")
            .chunk(refusal.to_string()),
    );
    vendor.respond(answer(&["The user asked a first question."], 10, 5));
    vendor.respond(answer(&["Second answer."], 8, 2));
    let turn = socket.chat("m2", "Second question").await;

    // One pass: the summary request is the first request, which the vendor
    // took, with the instruction after it; then the refused request goes
    // again, from the summary.
    let requests: Vec<Value> = vendor
        .requests()
        .iter()
        .map(|request| request.json())
        .collect();
    let [first, refused, summary, again] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    let messages = |body: &Value| unmarked(&body["messages"]).as_array().unwrap().clone();
    // Each block with its message's role: consecutive user items share a
    // message, and the vendor caches by block.
    let blocks = |body: &Value| -> Vec<(Value, Value)> {
        messages(body)
            .iter()
            .flat_map(|message| {
                let role = message["role"].clone();
                message["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(move |block| (role.clone(), block.clone()))
            })
            .collect()
    };
    let (first_blocks, summary_blocks) = (blocks(first), blocks(summary));
    assert_eq!(summary_blocks[..first_blocks.len()], first_blocks[..]);
    assert_eq!(summary_blocks.len(), first_blocks.len() + 1);
    let instruction = summary_blocks.last().unwrap().1.to_string();
    assert!(
        instruction.contains("Summarize the conversation above"),
        "{summary}"
    );
    assert_eq!(
        (unmarked(&summary["system"]), &summary["tools"]),
        (unmarked(&first["system"]), &first["tools"])
    );
    assert_eq!(messages(refused).len(), 3);
    let again = messages(again);
    assert!(
        again[0]
            .to_string()
            .contains("Previous conversation summary:\\nThe user asked a first question."),
        "{again:?}"
    );
    assert!(again[1].to_string().contains("First answer."), "{again:?}");
    assert!(
        again[2].to_string().contains("Second question"),
        "{again:?}"
    );
    // The page saw the pass, and no retry: the refusal left nothing behind.
    assert!(
        turn.iter().any(|frame| matches!(
            frame,
            ServerFrame::Phase {
                phase: SessionPhase::Compacting
            }
        )),
        "{turn:?}"
    );
    assert!(
        !turn
            .iter()
            .any(|frame| matches!(frame, ServerFrame::RetryScheduled { .. })),
        "{turn:?}"
    );
    assert_eq!(
        kinds(&socket.live().await),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "user",
            "compaction_marker",
            "text",
            "response"
        ]
    );
    backend.close().await;
}

/// A configured model `id` with a window of 800,000 tokens, which offers
/// the limits 300K and 200K.
fn large_window(id: &str) -> Value {
    json!({
        "id": id, "displayName": id, "contextWindow": 800000, "outputLimit": null,
        "thinkingEfforts": [], "acceptedExtensions": [], "fastTier": null
    })
}

/// Whether the turn's frames show a compaction pass.
fn compacted(turn: &[ServerFrame]) -> bool {
    turn.iter().any(|frame| {
        matches!(
            frame,
            ServerFrame::Phase {
                phase: SessionPhase::Compacting
            }
        )
    })
}

// A backend, a scripted vendor, three conversations and seven requests:
// about a second.
#[tokio::test]
async fn a_users_context_limit_on_a_model_applies_to_each_of_their_conversations_with_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url("/v1"),
        "models": [large_window("model-a"), large_window("model-b")]
    });
    let provider = entry(&backend, &master, body).await;
    let limit = json!({ "contextLimit": { "providerId": provider, "modelId": "model-a", "tokens": 200000 } });
    let limited = backend
        .patch("/api/settings/preferences", &master, limit)
        .await;
    assert_eq!(limited.status, StatusCode::OK);

    // An answer that reports 170,000 tokens: over 80% of 200K, under 80% of
    // the model's own 800K.
    for (id, model, limited) in [
        (FIRST, "model-a", true),
        (SECOND, "model-a", true),
        (THIRD, "model-b", false),
    ] {
        create(&backend, &master, id).await;
        choose(&backend, &master, id, &provider, model).await;
        let mut socket = Socket::connect(&backend, &master, id).await;
        socket.open().await;
        let before = vendor.requests().len();
        vendor.respond(answer(&["An answer."], 170_000, 5));
        if limited {
            vendor.respond(answer(&["The user asked a question."], 10, 5));
            vendor.respond(answer(&["Continued."], 20, 2));
        }
        let turn = socket.chat("m1", "A question").await;

        // The limited model's turn compacts after the answer and goes on
        // from the summary; the other model's ends with its answer.
        let requests = vendor.requests().len() - before;
        assert_eq!(
            (compacted(&turn), requests),
            if limited { (true, 3) } else { (false, 1) },
            "{id} {model}"
        );
    }
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
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let deltas: Vec<String> = (0..200).map(|index| format!("{index} ")).collect();
    let deltas: Vec<&str> = deltas.iter().map(String::as_str).collect();
    vendor.respond(answer(&deltas, 5, 200));

    let mut slow = Socket::connect(&backend, &master, FIRST).await;
    slow.open().await;
    slow.send(&send("m1", "Count")).await;
    assert_eq!(slow.closed().await, Some(4001));

    let mut again = Socket::connect(&backend, &master, FIRST).await;
    again.open().await;
    let deadline = tokio::time::Instant::now() + PATIENCE;
    let blocks = loop {
        let blocks = again.live().await;
        if blocks
            .iter()
            .any(|block| matches!(block, Block::Response(_)))
        {
            break blocks;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the adopted turn never ended: {blocks:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        last_text(&blocks).ends_with("199 "),
        "the turn ran to its end"
    );
    assert_eq!(
        vendor.requests().len(),
        1,
        "the reopen adopted the tree, which asked once"
    );
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
    socket
        .send_text(json!({ "type": "send", "messageId": "m1" }).to_string())
        .await;
    let ServerFrame::Error { code, .. } = socket.frame().await else {
        panic!("an invalid frame is answered with an error");
    };
    assert_eq!(code.as_deref(), Some("invalid_frame"));
    // A conversation without a model opens no tree.
    socket.send(&ClientFrame::Open {}).await;
    let ServerFrame::Error { code, .. } = socket.frame().await else {
        panic!("an open without a model is refused");
    };
    assert_eq!(code.as_deref(), Some("model_not_selected"));
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    socket.open().await;
    // The usage follows the handshake.
    socket
        .until(|frame| matches!(frame, ServerFrame::ContextUsage { .. }))
        .await;

    // Archived, the conversation takes no frame but its close, and no
    // socket.
    harness
        .control_database()
        .execute(
            "UPDATE conversations SET archived = 1 WHERE id = ?1",
            [FIRST],
        )
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
    assert_eq!(
        Socket::try_connect(&backend, &master, FIRST).await.err(),
        Some(409)
    );
    socket.send(&ClientFrame::Close {}).await;
    assert_eq!(socket.frame().await, ServerFrame::Closed);

    // A message that is not JSON closes the socket.
    socket.send_text("not json".into()).await;
    assert_eq!(socket.closed().await, Some(u16::from(CloseCode::Invalid)));
    // A request from the product's page that is not an upgrade.
    let product = [("origin", backend.url.as_str())];
    let path = format!("/api/conversations/{SECOND}/stream");
    let not_socket = backend.get_with(&path, &master, &product).await;
    assert_eq!(
        not_socket.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound)
    );
    create(&backend, &master, SECOND).await;
    let not_socket = backend.get_with(&path, &master, &product).await;
    assert_eq!(
        not_socket.refusal(),
        (StatusCode::UPGRADE_REQUIRED, ErrorCode::UpgradeRequired)
    );
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
            key: key.api_key.expose().to_owned(),
            runs: self.0.clone(),
        }))
    }
}

struct KeyedProvider {
    key: String,
    runs: Arc<Runs>,
}

impl Provider for KeyedProvider {
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
        let output = request
            .output_limit
            .map_or(0, |limit| u64::from(limit.get()));
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

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits::default()
    }
}

/// A configured model of `output` tokens at most.
pub(crate) fn configured(output: u32) -> Value {
    json!({
        "id": "m", "displayName": "M", "contextWindow": 100000, "outputLimit": output,
        "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
    })
}

/// A page on an expose shares the product's site, so the user's browser
/// sends it the session cookie; the socket must still refuse it
/// (`backend.md` § Authentication and ownership).
#[tokio::test]
async fn the_conversation_socket_opens_only_from_a_page_of_the_product() {
    let harness = Harness::new().with_expose_domain("expose.localhost");
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    // An expose's origin as the backend prints its URLs: the public URL's
    // scheme and port under the expose domain.
    let port = backend.url.rsplit(':').next().unwrap();
    let expose = format!("http://a1b2c3d4e5.expose.localhost:{port}");
    for origin in ["https://elsewhere.example", expose.as_str()] {
        let refused = Socket::try_connect_from(&backend, &master, FIRST, origin)
            .await
            .err();
        assert_eq!(refused, Some((403, ErrorCode::ForbiddenOrigin)), "{origin}");
    }
    // The product's own page opens it.
    Socket::connect(&backend, &master, FIRST).await;
    backend.close().await;
}

#[tokio::test]
async fn an_edit_of_the_entry_reaches_the_next_request_and_a_deleted_entry_refuses_inference() {
    let runs = Runs::new();
    let harness = Harness::new()
        .with_families(demi_backend::families::builtin().with("keyed", Keyed(runs.clone())));
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "keyed", "label": "Keyed", "apiKey": "old-key",
        "models": [configured(4_000)]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // An edit while the first request runs: that request finishes with the
    // runtime it started with, and the next one gets a new runtime.
    socket.send(&send("m1", "before the edit")).await;
    crate::support::eventually("the first request runs", || async {
        runs.calls().len() == 1
    })
    .await;
    let path = format!("/api/providers/{provider}");
    let edited = backend
        .patch(
            &path,
            &master,
            json!({ "apiKey": "new-key", "models": [configured(8_000)] }),
        )
        .await;
    assert_eq!(
        edited.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&edited.body)
    );
    runs.released.send_replace(true);
    socket.until_idle().await;
    socket.chat("m2", "after the edit").await;
    socket.chat("m3", "the same entry").await;
    assert_eq!(
        runs.calls(),
        [
            ("old-key".to_owned(), 1),
            ("new-key".to_owned(), 1),
            ("new-key".to_owned(), 2)
        ]
    );
    assert_eq!(
        runs.runtimes.load(Ordering::SeqCst),
        2,
        "an unchanged entry keeps its runtime"
    );
    let totals = usage(&backend, &master).await.totals;
    assert_eq!(
        (totals[0].requests, totals[0].output_tokens),
        (3, 4_000 + 8_000 + 8_000),
        "{totals:?}"
    );

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
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    vendor.respond(answer(&["one"], 1, 1));
    socket.chat("m1", "first").await;
    let turn = socket.chat("m2", "second").await;

    let limited = turn.iter().any(|frame| matches!(frame, ServerFrame::Error { code, .. } if code.as_deref() == Some("rate_limited")));
    assert!(limited, "{turn:?}");
    assert_eq!(vendor.requests().len(), 1);
    assert_eq!(usage(&backend, &master).await.totals[0].requests, 1);
    assert_eq!(
        summary(&backend, &master, FIRST).await.status,
        ConversationStatus::Error
    );
    backend.close().await;
}

#[tokio::test]
async fn a_shutdown_in_the_middle_of_a_turn_saves_its_interruption_and_the_next_start_serves_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["first"], 4, 1));
    socket.chat("m1", "a first message").await;
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    socket.send(&send("m2", "take your time")).await;
    vendor.received(2).await;

    backend.close().await;
    assert_eq!(socket.closed().await, Some(u16::from(CloseCode::Away)));

    let backend = harness.start().await;
    let master = backend
        .login(MASTER_EMAIL, crate::support::MASTER_PASSWORD)
        .await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "text", "response", "user", "error"]
    );
    let Some(Block::Error(record)) = blocks.last() else {
        unreachable!()
    };
    assert_eq!(record.code.as_deref(), Some("interrupted"));
    let summary = summaries(&backend, &master).await.remove(0);
    assert_eq!(summary.status, ConversationStatus::Interrupted);
    // The ledger carries its rows over the restart.
    assert_eq!(usage(&backend, &master).await.totals[0].requests, 1);

    // The next turn runs on the restored tree.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["done"], 1, 1));
    socket.chat("m3", "go on").await;
    assert_eq!(last_text(&socket.live().await), "done");
    backend.close().await;
}

/// A yield wakeup outlives a restart of the backend (`runtime.md` § Yield
/// wakeups): the model asks to check the build in ten minutes, the backend
/// is down past that time, and once it starts again the wakeup's turn runs
/// with no page open.
#[tokio::test]
async fn a_wakeup_due_while_the_backend_was_down_runs_its_turn_once_it_starts_again() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(tool_use(
        "toolu_wait",
        "yield",
        &json!({ "durationMs": 600_000 }),
    ));
    socket.chat("m1", "start the build").await;
    let asked = vendor.requests().len();
    drop(socket);
    backend.close().await;

    harness.clock.advance(jiff::SignedDuration::from_mins(11));
    vendor.respond(answer(&["the build passed"], 1, 1));
    let backend = harness.start().await;
    vendor.received(asked + 1).await;
    let woken = vendor.requests()[asked].json()["messages"].to_string();
    assert!(woken.contains("Scheduled yield wakeup fired"), "{woken}");
    let master = backend
        .login(MASTER_EMAIL, crate::support::MASTER_PASSWORD)
        .await;
    crate::support::eventually("the woken turn is saved", || async {
        let blocks = transcript(&backend, &master, FIRST).await.blocks;
        kinds(&blocks).ends_with(&["wakeup".into(), "text".into(), "response".into()])
    })
    .await;
    backend.close().await;
}

/// A page's conversation socket that stops reading where the test says,
/// with a receive buffer of a few kilobytes: a frame larger than the
/// backend's send buffer then fills the socket's buffers, and the backend's
/// send of it waits for a read that never comes.
struct StalledPage {
    socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
}

impl StalledPage {
    async fn connect(backend: &TestBackend, session: &Session, conversation: &str) -> Self {
        let tcp = tokio::net::TcpSocket::new_v4().unwrap();
        tcp.set_recv_buffer_size(4096).unwrap();
        let stream = tcp.connect(backend.address()).await.unwrap();
        let url = backend.ws_url(&format!("/api/conversations/{conversation}/stream"));
        let mut request = url.into_client_request().unwrap();
        let headers = request.headers_mut();
        headers.insert("cookie", session.cookie.parse().unwrap());
        headers.insert("origin", backend.url.parse().unwrap());
        let (socket, _) = tokio_tungstenite::client_async(request, stream)
            .await
            .unwrap();
        Self { socket }
    }

    /// Opens the conversation and reads the socket's bytes as far as the
    /// header of the frame after `opened`, the transcript's reset, whose
    /// length it answers: the backend is sending the reset, and the page
    /// reads no further.
    async fn open_until_the_reset_is_sent(&mut self) -> u64 {
        let open = serde_json::to_string(&ClientFrame::Open {}).unwrap();
        self.socket.send(Message::Text(open.into())).await.unwrap();
        // Read beneath the WebSocket, which would read the whole reset: a
        // final text frame for `opened`, then the reset's frame header with
        // its 64-bit length.
        let opened = br#"{"type":"opened"}"#;
        let mut head = [0; 2 + 17 + 10];
        tokio::time::timeout(PATIENCE, self.socket.get_mut().read_exact(&mut head))
            .await
            .expect("the backend sends the handshake")
            .unwrap();
        assert_eq!(head[..2], [0x81, 17]);
        assert_eq!(&head[2..19], opened);
        assert_eq!(head[19..21], [0x81, 127]);
        u64::from_be_bytes(head[21..].try_into().unwrap())
    }
}

/// A page that stopped reading, its socket's buffers full, holds up
/// shutdown no longer than the close frame's bound (`backend.md` § Startup
/// and shutdown): the backend is in the middle of sending a transcript
/// larger than any socket buffer when it shuts down. The 8 MiB answer that
/// makes the transcript so large costs most of the test's second: a smaller
/// one can fit the send buffer, whose limit is 4 MiB on Linux and macOS. The
/// answer is the model's, since a page sends no message over 1 MiB. It comes
/// in 128 KiB deltas, as a vendor streams it: the stream's parser reads a
/// line again from its start each time more of it arrives, so one 8 MiB line
/// cost seconds, and up to 12 under a full test run.
#[tokio::test]
async fn a_page_that_stopped_reading_does_not_hold_up_shutdown() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new();
    harness.pages.close_wait = Duration::from_millis(100);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let piece = "x".repeat(128 << 10);
    vendor.respond(answer(&[piece.as_str(); 64], 1, 1));
    socket.chat("m1", "Write at length.").await;

    let mut stalled = StalledPage::connect(&backend, &master, FIRST).await;
    let reset = stalled.open_until_the_reset_is_sent().await;
    assert!(
        reset > 8 << 20,
        "the reset carries the whole transcript: {reset} bytes"
    );
    tokio::time::timeout(PATIENCE, backend.close())
        .await
        .expect("a page that stopped reading does not hold up shutdown");
}

#[tokio::test]
async fn the_page_receives_what_the_provider_reads_from_an_error_blocks_record() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(
        MockResponse::status(401)
            .header("retry-after", "120")
            .chunk(r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
    );
    let turn = socket.chat("m1", "hi").await;

    // The frame that brings the error block carries the provider's reading
    // of its record: the vendor's wait, counted from when it failed.
    let read = turn.iter().find_map(|frame| match frame {
        ServerFrame::TranscriptPatch {
            failures: Some(failures),
            ..
        } => Some(failures.clone()),
        _ => None,
    });
    let read = read.unwrap_or_else(|| panic!("no frame carried failure facts: {turn:?}"));
    let blocks = transcript(&backend, &master, FIRST).await;
    let Some(Block::Error(error)) = blocks.blocks.last() else {
        panic!("the turn failed: {:?}", blocks.blocks);
    };
    let retry_at: Timestamp = "2026-09-24T08:02:00.000Z".parse().unwrap();
    assert_eq!(
        read.get(&error.id).and_then(|facts| facts.retry_at),
        Some(retry_at)
    );
    assert_eq!(blocks.failures, Some(read));
    let others = turn
        .iter()
        .filter(|frame| {
            matches!(
                frame,
                ServerFrame::TranscriptPatch {
                    failures: Some(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        others, 1,
        "only the frame that brings the block carries facts"
    );

    // With its entry gone, the record shows without facts.
    backend
        .delete(&format!("/api/providers/{provider}"), &master)
        .await;
    assert_eq!(transcript(&backend, &master, FIRST).await.failures, None);
    backend.close().await;
}

// About a second: the tool call boots the Cloud to run its shell job.
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
        MockResponse::status(200)
            .header("etag", "\"fixture\"")
            .chunk(catalog.to_string()),
    );
    let harness = Harness::new().with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "vendor", "vendorId": "deepseek", "label": "DeepSeek", "apiKey": "fake-key",
        "baseUrl": vendor.url("/v1"), "models": [configured(8_000)]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
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

fn applied(field: PatchField) -> FieldResult {
    FieldResult::Applied { field }
}

#[tokio::test]
async fn each_field_of_a_patch_applies_on_its_own_and_an_archived_conversation_takes_only_its_restore()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user(
        "ana@example.test",
        "ana-pass-1",
        demi_web_api_protocol::auth::Role::User,
    );
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    create(&backend, &master, SECOND).await;
    let path = format!("/api/conversations/{FIRST}");
    let listed = async || -> Vec<String> {
        summaries(&backend, &master)
            .await
            .into_iter()
            .map(|summary| summary.id.as_str().to_owned())
            .collect()
    };

    let renamed = backend
        .patch(&path, &master, json!({ "title": "  Build failure  " }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    let update = renamed.json::<ConversationUpdate>();
    assert_eq!(update.results, [applied(PatchField::Title)]);
    assert_eq!(update.conversation.title, "Build failure");
    // The target the conversation has already is no switch.
    let body = json!({
        "pinned": true, "model": { "providerId": provider, "modelId": "claude-opus-4-8" }, "target": { "kind": "cloud" }
    });
    let chosen = backend.patch(&path, &master, body).await;
    assert_eq!(chosen.status, StatusCode::OK);
    let update = chosen.json::<ConversationUpdate>();
    assert_eq!(
        update.results,
        [
            applied(PatchField::Pinned),
            applied(PatchField::Model),
            applied(PatchField::Target)
        ]
    );
    let conversation = update.conversation;
    let model = conversation
        .model
        .as_ref()
        .map(|model| (model.provider_id.as_str(), model.model_id.as_str()));
    assert_eq!(
        (conversation.pinned, model),
        (true, Some((provider.as_str(), "claude-opus-4-8")))
    );
    assert_eq!(
        listed().await,
        [FIRST, SECOND],
        "a pinned conversation leads"
    );
    // A patch of one field that is refused answers that field's refusal.
    let foreign = backend
        .patch(
            &path,
            &master,
            json!({ "model": { "providerId": "someone-elses", "modelId": "m" } }),
        )
        .await;
    assert_eq!(
        foreign.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound)
    );

    // The archive goes first, so the rename beside it is refused.
    let archived = backend
        .patch(
            &path,
            &master,
            json!({ "title": "Renamed", "archived": true }),
        )
        .await;
    assert_eq!(archived.status, StatusCode::MULTI_STATUS);
    let update = archived.json::<ConversationUpdate>();
    assert_eq!(update.results[0], applied(PatchField::Archived));
    let FieldResult::Failed {
        field,
        code,
        http_status,
        ..
    } = &update.results[1]
    else {
        panic!(
            "the rename of an archived conversation is refused: {:?}",
            update.results
        );
    };
    assert_eq!(
        (*field, *code, *http_status),
        (PatchField::Title, ErrorCode::ConversationArchived, 409)
    );
    assert_eq!(
        (
            update.conversation.archived,
            update.conversation.title.as_str()
        ),
        (true, "Build failure")
    );
    assert_eq!(listed().await, [SECOND]);
    let archived = backend
        .get("/api/conversations?archived=true", Some(&master))
        .await;
    assert_eq!(
        archived.json::<Conversations>().conversations,
        [update.conversation]
    );
    // Its history stays readable.
    assert!(transcript(&backend, &master, FIRST).await.blocks.is_empty());
    for body in [
        json!({ "pinned": false }),
        json!({ "target": { "kind": "cloud" } }),
    ] {
        let refused = backend.patch(&path, &master, body.clone()).await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::CONFLICT, ErrorCode::ConversationArchived),
            "{body}"
        );
    }
    let restored = backend
        .patch(&path, &master, json!({ "archived": false }))
        .await;
    assert_eq!(restored.status, StatusCode::OK);
    assert_eq!(
        listed().await,
        [FIRST, SECOND],
        "the restore keeps the place and the pin"
    );

    // A body outside the patch's rules changes nothing, and neither does a
    // conversation the caller does not have.
    let bodies = [
        json!({ "title": "   " }),
        json!({ "title": "x".repeat(257) }),
        json!({ "name": "x" }),
        json!({ "model": { "providerId": provider } }),
        json!({ "model": null }),
        json!({ "archived": "yes" }),
    ];
    for body in bodies {
        let refused = backend.patch(&path, &master, body.clone()).await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{body}"
        );
    }
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    for path in [path.clone(), "/api/conversations/not-a-uuid".into()] {
        let foreign = backend.patch(&path, &ana, json!({ "pinned": true })).await;
        assert_eq!(
            foreign.refusal(),
            (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound),
            "{path}"
        );
    }
    assert_eq!(summaries(&backend, &master).await[0].title, "Build failure");
    backend.close().await;
}

/// A configured model with the efforts `low` and `high`, whose Fast is the
/// tier `fast` when there is one.
pub(crate) fn leveled(id: &str, fast: Option<&str>) -> Value {
    json!({
        "id": id, "displayName": id.to_uppercase(), "contextWindow": 100000, "outputLimit": 4000,
        "thinkingEfforts": ["low", "high"], "acceptedExtensions": null, "fastTier": fast
    })
}

/// A conversation's model settings are one value that every page shows
/// (`web-api.md` § Sidebar mutations, read state and page synchronization):
/// a change names only its parts, so two pages that change different parts
/// keep both; another page reads the change in its next snapshot; the
/// conversation's next request uses it, whichever page sends it; and a tree
/// opened after a change made while none was live opens with it.
#[tokio::test]
async fn a_change_of_the_model_settings_reaches_every_page_and_the_next_request() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url("/v1"), "models": [leveled("m", Some("priority")), leveled("n", None)]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    create(&backend, &master, SECOND).await;
    let path = format!("/api/conversations/{FIRST}");
    let settings = |model: &str, effort: Option<&str>, tier: Option<&str>| ModelSettings {
        provider_id: provider.as_str().try_into().unwrap(),
        model_id: model.into(),
        thinking_effort: effort.map(str::to_owned),
        service_tier_id: tier.map(str::to_owned),
    };
    let chosen = choose(&backend, &master, FIRST, &provider, "m").await;
    assert_eq!(chosen.model, Some(settings("m", Some("low"), None)));

    // Page A raises the effort, and page B, another sign-in that has not
    // read that, turns Fast on: the value ends with both, and page A reads
    // it in its next snapshot.
    let other = backend
        .login(MASTER_EMAIL, crate::support::MASTER_PASSWORD)
        .await;
    let mut socket = Socket::connect(&backend, &other, FIRST).await;
    socket.open().await;
    let raised = backend
        .patch(&path, &master, json!({ "thinkingEffort": "high" }))
        .await;
    assert_eq!(
        raised.json::<ConversationUpdate>().results,
        [applied(PatchField::ThinkingEffort)]
    );
    let fast = backend
        .patch(&path, &other, json!({ "serviceTierId": "priority" }))
        .await;
    let both = settings("m", Some("high"), Some("priority"));
    assert_eq!(
        fast.json::<ConversationUpdate>().conversation.model,
        Some(both.clone())
    );
    assert_eq!(summary(&backend, &master, FIRST).await.model, Some(both));

    // Page B's message runs with both.
    vendor.respond(answer(&["one"], 1, 1));
    socket.chat("m1", "first").await;
    let sent = vendor.requests()[0].json();
    assert_eq!(
        (
            &sent["model"],
            &sent["output_config"]["effort"],
            &sent["service_tier"]
        ),
        (&json!("m"), &json!("high"), &json!("priority"))
    );

    // Page A switches the model as its menu does, naming the effort the new
    // model lists; the new model has no Fast tier, so the tier goes. Page
    // B's next message runs with the new value.
    let body =
        json!({ "model": { "providerId": provider, "modelId": "n" }, "thinkingEffort": "high" });
    let switched = backend.patch(&path, &master, body).await;
    let update = switched.json::<ConversationUpdate>();
    assert_eq!(
        update.results,
        [
            applied(PatchField::Model),
            applied(PatchField::ThinkingEffort)
        ]
    );
    assert_eq!(
        update.conversation.model,
        Some(settings("n", Some("high"), None))
    );
    vendor.respond(answer(&["two"], 1, 1));
    socket.chat("m2", "second").await;
    let sent = vendor.requests()[1].json();
    assert_eq!(
        (
            &sent["model"],
            &sent["output_config"]["effort"],
            sent.get("service_tier")
        ),
        (&json!("n"), &json!("high"), None)
    );

    // A change while no tree is live is the record's, and the next open
    // takes it.
    socket.send(&ClientFrame::Close {}).await;
    socket
        .until(|frame| matches!(frame, ServerFrame::Closed))
        .await;
    let lowered = backend
        .patch(&path, &master, json!({ "thinkingEffort": "low" }))
        .await;
    assert_eq!(lowered.status, StatusCode::OK);
    socket.open().await;
    vendor.respond(answer(&["three"], 1, 1));
    socket.chat("m3", "third").await;
    assert_eq!(
        vendor.requests()[2].json()["output_config"]["effort"],
        "low"
    );

    // A part the model does not offer, a model the catalog does not list,
    // and a part for a conversation without a model are refused and change
    // nothing.
    let second = format!("/api/conversations/{SECOND}");
    let refusals = [
        (
            &path,
            json!({ "serviceTierId": "priority" }),
            StatusCode::CONFLICT,
            ErrorCode::SettingUnavailable,
        ),
        (
            &path,
            json!({ "thinkingEffort": "max" }),
            StatusCode::CONFLICT,
            ErrorCode::SettingUnavailable,
        ),
        (
            &path,
            json!({ "model": { "providerId": provider, "modelId": "x" } }),
            StatusCode::NOT_FOUND,
            ErrorCode::ModelNotFound,
        ),
        (
            &second,
            json!({ "thinkingEffort": "low" }),
            StatusCode::CONFLICT,
            ErrorCode::ModelNotSelected,
        ),
    ];
    for (target, body, status, code) in refusals {
        let refused = backend.patch(target, &master, body.clone()).await;
        assert_eq!(refused.refusal(), (status, code), "{body}");
    }
    assert_eq!(
        summary(&backend, &master, FIRST).await.model,
        Some(settings("n", Some("low"), None))
    );
    backend.close().await;
}

/// A change that names no effort takes the first effort the model lists,
/// on a model that can turn thinking off too (`models.md` § A conversation's
/// model settings): the settings never leave thinking to the vendor, and the
/// request carries the effort every page shows. A patch cannot name no
/// effort with null.
#[tokio::test]
async fn a_change_that_names_no_effort_takes_the_first_effort_the_model_lists() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    // Configured models can turn thinking off and list `low` first.
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url("/v1"), "models": [leveled("m", None), leveled("n", None)]
    });
    let provider = entry(&backend, &master, body).await;
    create(&backend, &master, FIRST).await;
    let effort =
        |summary: ConversationSummary| summary.model.and_then(|model| model.thinking_effort);
    let chosen = choose(&backend, &master, FIRST, &provider, "m").await;
    assert_eq!(effort(chosen).as_deref(), Some("low"));
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["one"], 1, 1));
    socket.chat("m1", "first").await;
    let sent = vendor.requests()[0].json();
    assert_eq!(
        (&sent["thinking"]["type"], &sent["output_config"]["effort"]),
        (&json!("adaptive"), &json!("low"))
    );

    // A switch from thinking off that names no effort takes the new model's
    // first effort as well.
    let path = format!("/api/conversations/{FIRST}");
    let off = backend
        .patch(&path, &master, json!({ "thinkingEffort": "disabled" }))
        .await;
    assert_eq!(
        effort(off.json::<ConversationUpdate>().conversation).as_deref(),
        Some("disabled")
    );
    let switch = json!({ "model": { "providerId": provider, "modelId": "n" } });
    let switched = backend.patch(&path, &master, switch).await;
    assert_eq!(
        effort(switched.json::<ConversationUpdate>().conversation).as_deref(),
        Some("low")
    );

    let null = backend
        .patch(&path, &master, json!({ "thinkingEffort": null }))
        .await;
    assert_eq!(
        null.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );
    backend.close().await;
}

#[tokio::test]
async fn an_archive_refuses_running_work_and_holds_the_open_socket_until_the_restore() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    socket.send(&send("m1", "take your time")).await;
    vendor.received(1).await;

    let path = format!("/api/conversations/{FIRST}");
    let busy = backend
        .patch(&path, &master, json!({ "archived": true }))
        .await;
    assert_eq!(
        busy.refusal(),
        (StatusCode::CONFLICT, ErrorCode::TurnInFlight)
    );

    // Stopped and saved, the turn no longer holds the conversation.
    socket.stop().await;
    assert_eq!(
        summary(&backend, &master, FIRST).await.status,
        ConversationStatus::Stopped
    );
    let archived = backend
        .patch(&path, &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK);
    socket.send(&send("m2", "still there?")).await;
    let ServerFrame::Error { code, .. } = socket.frame().await else {
        panic!("an archived conversation refuses a message");
    };
    assert_eq!(code.as_deref(), Some("conversation_archived"));
    assert_eq!(
        Socket::try_connect(&backend, &master, FIRST).await.err(),
        Some(409)
    );

    // Restored, the socket that stayed open runs a turn again.
    let restored = backend
        .patch(&path, &master, json!({ "archived": false }))
        .await;
    assert_eq!(restored.status, StatusCode::OK);
    vendor.respond(answer(&["back"], 1, 1));
    socket.chat("m3", "and now?").await;
    assert_eq!(last_text(&socket.live().await), "back");
    assert_eq!(
        vendor.requests().len(),
        2,
        "the refused message never reached the vendor"
    );
    backend.close().await;
}

#[tokio::test]
async fn a_batch_answers_each_item_on_its_own() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user(
        "ana@example.test",
        "ana-pass-1",
        demi_web_api_protocol::auth::Role::User,
    );
    create(&backend, &master, FIRST).await;
    create(&backend, &master, SECOND).await;
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    create(&backend, &ana, THIRD).await;

    let body = json!({ "items": [
        { "id": FIRST, "patch": { "pinned": true } },
        { "id": SECOND, "patch": { "archived": true, "title": "Old" } },
        { "id": THIRD, "patch": { "pinned": true } },
    ] });
    let answered = backend
        .post("/api/conversations/batch", Some(&master), body)
        .await;
    assert_eq!(answered.status, StatusCode::MULTI_STATUS);
    let results = answered.json::<BatchAnswer>().results;
    let [first, second, third] = results.as_slice() else {
        panic!("one outcome per item: {results:?}");
    };
    let BatchResult::Updated {
        id,
        conversation,
        results,
    } = first
    else {
        panic!("{first:?}");
    };
    assert_eq!(
        (id.as_str(), conversation.pinned, results.as_slice()),
        (FIRST, true, &[applied(PatchField::Pinned)][..])
    );
    let BatchResult::Updated {
        conversation,
        results,
        ..
    } = second
    else {
        panic!("{second:?}");
    };
    assert!(conversation.archived);
    assert!(matches!(
        results.as_slice(),
        [
            FieldResult::Applied {
                field: PatchField::Archived
            },
            FieldResult::Failed {
                code: ErrorCode::ConversationArchived,
                ..
            }
        ]
    ));
    let BatchResult::Refused { id, code, .. } = third else {
        panic!("another user's conversation is refused: {third:?}");
    };
    assert_eq!(
        (id.as_str(), *code),
        (THIRD, ErrorCode::ConversationNotFound)
    );
    assert!(!summaries(&backend, &ana).await[0].pinned);

    let too_many: Vec<Value> = (0..101)
        .map(|_| json!({ "id": FIRST, "patch": {} }))
        .collect();
    for body in [
        json!({ "items": [] }),
        json!({ "items": too_many }),
        json!({ "items": [{ "id": FIRST }] }),
    ] {
        let refused = backend
            .post("/api/conversations/batch", Some(&master), body)
            .await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
        );
    }
    backend.close().await;
}
