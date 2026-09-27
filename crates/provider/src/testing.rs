//! Test support (feature `testing`): a scripted runtime for the tests of what
//! sits above providers, a scripted vendor server for the tests of providers,
//! waits for a run's events that fail instead of hanging, the check of an
//! API-key entry's built-in catalog, event builders and a fixed clock. No test
//! calls a real model.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Request, State},
    response::Response,
};
use demi_core::{AuthState, Clock, RuntimeState, Timestamp, TokenUsage, UserContentBlock};
use futures_util::{
    Stream, StreamExt,
    future::LocalBoxFuture,
    stream::{self, BoxStream, LocalBoxStream},
};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use tokio::sync::Notify;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use crate::{
    InferenceItem, InferenceRequest, Provider, ProviderEvent, ProviderFailure, ProviderRun,
    ProviderRuntime, ToolCall,
};

/// Builders of the events a scripted run yields.
pub mod event {
    use super::*;

    pub fn text(text: &str) -> ProviderEvent {
        ProviderEvent::TextDelta(text.to_owned())
    }

    pub fn thinking(text: &str) -> ProviderEvent {
        ProviderEvent::ThinkingDelta(text.to_owned())
    }

    pub fn tool_call(tool_use_id: &str, tool_name: &str, input: serde_json::Value) -> ProviderEvent {
        ProviderEvent::ToolCall(ToolCall {
            tool_use_id: tool_use_id.to_owned(),
            tool_name: tool_name.to_owned(),
            input,
        })
    }

    /// A response whose usage is `input_tokens` in and `output_tokens` out.
    pub fn response(input_tokens: u64, output_tokens: u64) -> ProviderEvent {
        ProviderEvent::Response(TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        })
    }

    /// A failure with a code and no diagnostics.
    pub fn error(message: &str, code: Option<crate::ErrorCode>) -> ProviderEvent {
        ProviderEvent::Error(ProviderFailure {
            message: message.to_owned(),
            code,
            diagnostics: None,
            retry_after: None,
        })
    }
}

/// A request with a user message `hello` and nothing else; tests replace the
/// fields they care about.
pub fn inference_request() -> InferenceRequest {
    InferenceRequest {
        session_id: "session-1".into(),
        turn_id: "turn-1".into(),
        request_id: "request-1".into(),
        model_id: "model-1".into(),
        output_limit: None,
        output_cap: None,
        system_prompt: String::new(),
        items: Arc::new([InferenceItem::UserMessage {
            content: vec![UserContentBlock::Text {
                text: "hello".into(),
            }],
        }]),
        tools: Arc::new([]),
        thinking: None,
        service_tier_id: None,
        cancel: CancellationToken::new(),
    }
}

/// One scripted run.
pub enum Turn {
    /// These events, in order.
    Events(Vec<ProviderEvent>),
    /// The events a function of the request returns, so that a test can
    /// answer what the request carries.
    Respond(Box<dyn FnOnce(&InferenceRequest) -> Vec<ProviderEvent>>),
    /// Any stream, for a test that controls timing, such as a run that
    /// never ends until it is cancelled.
    Stream(Box<dyn FnOnce(&InferenceRequest) -> LocalBoxStream<'static, ProviderEvent>>),
}

impl Turn {
    /// A run that yields nothing and ends only when it is cancelled.
    pub fn pending() -> Self {
        Self::Stream(Box::new(|_| stream::pending().boxed_local()))
    }
}

/// A runtime that plays scripted runs in order, one per `run`. A runtime and
/// its [`fresh`](ProviderRuntime::fresh) copies share one script, because a
/// test describes one ordered scenario across a session and its compaction
/// copy. A run honors its request's cancellation like a real one. A run
/// beyond the script panics: the test did not script it.
#[derive(Clone)]
pub struct ScriptedRuntime {
    script: Rc<RefCell<Script>>,
}

struct Script {
    turns: VecDeque<Turn>,
    requests: Vec<InferenceRequest>,
    closes: usize,
}

impl ScriptedRuntime {
    pub fn new(turns: impl IntoIterator<Item = Turn>) -> Self {
        Self {
            script: Rc::new(RefCell::new(Script {
                turns: turns.into_iter().collect(),
                requests: Vec::new(),
                closes: 0,
            })),
        }
    }

    /// Every request run so far, in order.
    pub fn requests(&self) -> Vec<InferenceRequest> {
        self.script.borrow().requests.clone()
    }

    /// The scripted runs not yet played.
    pub fn remaining(&self) -> usize {
        self.script.borrow().turns.len()
    }

    /// How often a runtime of this script was closed.
    pub fn closes(&self) -> usize {
        self.script.borrow().closes
    }
}

impl ProviderRuntime for ScriptedRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let (turn, number) = {
            let mut script = self.script.borrow_mut();
            script.requests.push(request.clone());
            (script.turns.pop_front(), script.requests.len())
        };
        let Some(turn) = turn else {
            panic!("ScriptedRuntime: no turn scripted for run #{number}");
        };
        let events = match turn {
            Turn::Events(events) => stream::iter(events).boxed_local(),
            Turn::Respond(respond) => stream::iter(respond(&request)).boxed_local(),
            Turn::Stream(open) => open(&request),
        };
        events
            .take_until(request.cancel.cancelled_owned())
            .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(self.clone())
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        self.script.borrow_mut().closes += 1;
        Box::pin(async {})
    }
}

/// How long a provider test waits for what it awaits before it fails. It
/// only guards against a hang (`testing.md` § Time and stability): every wait
/// of these tests ends far sooner, on a paused clock too, where the longest
/// is the Claude Code runtime's five seconds before it kills a process.
const HANG_GUARD: Duration = Duration::from_secs(10);

/// Awaits `future`, and fails the test when it has not completed within the
/// hang guard; `what` names what the test waits for.
pub async fn guarded<T>(what: &str, future: impl Future<Output = T>) -> T {
    let Ok(value) = tokio::time::timeout(HANG_GUARD, future).await else {
        panic!("{what} did not come within {HANG_GUARD:?}");
    };
    value
}

/// The run's next event, or `None` once it has ended.
pub async fn next_event(run: &mut ProviderRun<'_>) -> Option<ProviderEvent> {
    guarded("the run's next event or its end", run.next()).await
}

/// Every event of `run`, once it has ended.
pub async fn all_events(run: impl Stream<Item = ProviderEvent>) -> Vec<ProviderEvent> {
    guarded("the run's end", run.collect()).await
}

/// Runs `request` on `runtime` and returns every event of the run.
pub async fn run(runtime: &mut dyn ProviderRuntime, request: InferenceRequest) -> Vec<ProviderEvent> {
    all_events(runtime.run(request)).await
}

/// A wall clock that always reads the same moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// A JWT whose payload is `claims`, with a signature nobody checks, as the
/// tokens vendors issue: providers read claims without verifying them.
pub fn jwt(claims: &serde_json::Value) -> String {
    use base64::Engine;
    let encode = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
    format!("{}.{}.signature", encode(r#"{"alg":"none","typ":"JWT"}"#), encode(&claims.to_string()))
}

/// A server-sent events body with one `data:` frame per payload.
pub fn sse_body(payloads: &[serde_json::Value]) -> String {
    payloads
        .iter()
        .map(|payload| format!("data: {payload}\n\n"))
        .collect()
}

/// An in-process HTTP server that answers each request with the next
/// scripted response and records every request. It stops when dropped.
pub struct MockVendor {
    address: SocketAddr,
    /// `http`, or `https` for a vendor started with
    /// [`start_tls`](Self::start_tls).
    scheme: &'static str,
    state: Arc<VendorState>,
    _server: AbortOnDropHandle<()>,
}

struct VendorState {
    // Shared by the server's handler tasks and the test for sections that
    // never await.
    script: Mutex<VendorScript>,
    /// Signalled when a client leaves a response that was still open.
    disconnected: Notify,
    /// Signalled whenever a request arrives.
    requested: Notify,
}

#[derive(Default)]
struct VendorScript {
    responses: VecDeque<MockResponse>,
    /// Answers for one path, which a request to it takes before the others.
    routes: HashMap<String, VecDeque<MockResponse>>,
    requests: Vec<RecordedRequest>,
}

/// A request the vendor received.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: Method,
    /// The path and query.
    pub uri: Uri,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl RecordedRequest {
    /// The body as JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("the request body is JSON")
    }

    /// The value of the header `name`, as text.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }
}

/// A scripted answer.
#[derive(Debug, Clone)]
pub struct MockResponse {
    status: StatusCode,
    headers: Vec<(HeaderName, HeaderValue)>,
    chunks: Vec<Bytes>,
    ending: Ending,
}

/// What a response does after its chunks.
#[derive(Debug, Clone, Copy)]
enum Ending {
    /// The body ends.
    Complete,
    /// The body stays open until the client leaves.
    Open,
    /// The server never answers: no status, no headers.
    Silent,
    /// The connection breaks before the body ends.
    Broken,
}

impl MockResponse {
    /// An empty answer with `status`.
    pub fn status(status: u16) -> Self {
        Self {
            status: StatusCode::from_u16(status).expect("a scripted status is valid"),
            headers: Vec::new(),
            chunks: Vec::new(),
            ending: Ending::Complete,
        }
    }

    /// A 200 event stream whose body is `text`, sent in one chunk.
    pub fn event_stream(text: impl Into<String>) -> Self {
        Self::status(200)
            .header("content-type", "text/event-stream")
            .chunk(text.into())
    }

    /// Adds a header; a repeated name sends the header again.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let name = HeaderName::try_from(name).expect("a scripted header name is valid");
        let value = HeaderValue::try_from(value).expect("a scripted header value is valid");
        self.headers.push((name, value));
        self
    }

    /// Adds a body chunk, sent as one write.
    pub fn chunk(mut self, chunk: impl Into<Bytes>) -> Self {
        self.chunks.push(chunk.into());
        self
    }

    /// Keeps the response open after its chunks until the client leaves.
    /// The server sends an event-stream comment every 10 ms meanwhile, so that
    /// it notices the client leaving.
    pub fn stay_open(mut self) -> Self {
        self.ending = Ending::Open;
        self
    }

    /// Breaks the connection after the chunks, before the body is complete.
    pub fn break_off(mut self) -> Self {
        self.ending = Ending::Broken;
        self
    }

    /// A request that is never answered: the server reads it and sends
    /// nothing, not even a status.
    pub fn silent() -> Self {
        Self {
            ending: Ending::Silent,
            ..Self::status(200)
        }
    }
}

impl MockVendor {
    pub async fn start() -> Self {
        Self::serve(bind().await, "http")
    }

    /// The vendor over HTTPS, presenting `certificate`, a PEM certificate
    /// chain for `127.0.0.1`, with its PEM `key`: for a client that accepts
    /// nothing but `https`, such as `demi.claude` downloading from a
    /// distribution. The client trusts the chain's issuer by its own means.
    pub async fn start_tls(certificate: &[u8], key: &[u8]) -> Self {
        use tokio_rustls::rustls::pki_types::pem::PemObject as _;
        use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use tokio_rustls::rustls::{ServerConfig, crypto::ring};

        let chain = CertificateDer::pem_slice_iter(certificate)
            .collect::<Result<Vec<_>, _>>()
            .expect("the vendor's certificate is PEM");
        let key = PrivateKeyDer::from_pem_slice(key).expect("the vendor's key is PEM");
        let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring offers the default protocol versions")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("the key is the certificate's");
        let listener = TlsListener {
            tcp: bind().await,
            acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(config)),
        };
        Self::serve(listener, "https")
    }

    /// Serves the vendor's answers on `listener`, whose URLs have `scheme`.
    fn serve<L>(listener: L, scheme: &'static str) -> Self
    where
        L: axum::serve::Listener<Addr = SocketAddr>,
    {
        let address = listener.local_addr().expect("a bound listener has an address");
        let state = Arc::new(VendorState {
            script: Mutex::new(VendorScript::default()),
            disconnected: Notify::new(),
            requested: Notify::new(),
        });
        let router = Router::new().fallback(answer).with_state(state.clone());
        let server = tokio::spawn(async move {
            // The server ends only when the vendor is dropped and its task
            // aborted; a failed accept ends the test's server early, which
            // the test sees as refused requests.
            let _ = axum::serve(listener, router).await;
        });
        Self {
            address,
            scheme,
            state,
            _server: AbortOnDropHandle::new(server),
        }
    }

    /// The URL of `path` on this server, such as `/v1`.
    pub fn url(&self, path: &str) -> String {
        format!("{}://{}{path}", self.scheme, self.address)
    }

    /// Queues the answer to the next request.
    pub fn respond(&self, response: MockResponse) {
        self.lock().responses.push_back(response);
    }

    /// Queues the answer to the next request to `path`, such as
    /// `/oauth/token`, which it takes before the answers of
    /// [`respond`](Self::respond): concurrent requests to different paths
    /// then get their own answers whatever order they arrive in.
    pub fn respond_at(&self, path: &str, response: MockResponse) {
        self.lock()
            .routes
            .entry(path.to_owned())
            .or_default()
            .push_back(response);
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.lock().requests.clone()
    }

    /// Waits until the vendor has received `count` requests, at most five
    /// seconds.
    pub async fn received(&self, count: usize) {
        let arrived = async {
            loop {
                // Created before the count is read, so a request that arrives
                // in between still wakes it.
                let next = self.state.requested.notified();
                if self.lock().requests.len() >= count {
                    return;
                }
                next.await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), arrived)
            .await
            .expect("the vendor receives the requests");
    }

    /// Waits until a client leaves a response that was still open, at most
    /// five seconds.
    pub async fn disconnected(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.state.disconnected.notified())
            .await
            .expect("the client left the open response");
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VendorScript> {
        self.state.script.lock().expect("the vendor script is not poisoned")
    }
}

/// Asserts what the backend's catalog reads of an API-key entry's provider
/// (`models.md` § Directories): the entry starts no process, is
/// authenticated by its key and ready, and its models are the directory
/// built into the provider, never fetched and not stale, with its default
/// among them. Reading none of it sends `vendor` a request.
pub async fn assert_built_in_catalog(provider: &dyn Provider, vendor: &MockVendor) {
    assert!(!provider.capabilities().process_host, "an API-key entry starts no process");
    let auth = provider.auth_status().await;
    assert!(matches!(auth, AuthState::Authenticated { .. }), "{auth:?}");
    let runtime = provider.runtime_state();
    assert!(matches!(runtime, RuntimeState::Ready { .. }), "{runtime:?}");
    let list = provider.list_models().await.expect("a built-in directory is always read");
    assert_eq!(
        (list.source_fetched_at, list.stale),
        (Timestamp::UNIX_EPOCH, false),
        "a built-in directory is never fetched"
    );
    let default = list.default_model_id.as_deref().expect("the directory names its default");
    assert!(
        list.models.iter().any(|model| model.id == default),
        "the default {default} is one of the directory's models"
    );
    assert!(vendor.requests().is_empty(), "reading the status and the models makes no request");
}

async fn answer(State(state): State<Arc<VendorState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("the request body arrives");
    let next = {
        let mut script = state.script.lock().expect("the vendor script is not poisoned");
        let routed = script
            .routes
            .get_mut(parts.uri.path())
            .and_then(VecDeque::pop_front);
        script.requests.push(RecordedRequest {
            method: parts.method,
            uri: parts.uri,
            headers: parts.headers,
            body,
        });
        state.requested.notify_waiters();
        routed.or_else(|| script.responses.pop_front())
    };
    let Some(scripted) = next else {
        let mut response = Response::new(Body::from("MockVendor: no response scripted"));
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        return response;
    };
    if let Ending::Silent = scripted.ending {
        // The handler never returns, so the server sends nothing; the
        // connection closes when the client leaves or the vendor stops.
        return std::future::pending().await;
    }
    let chunks = stream::iter(scripted.chunks.into_iter().map(Ok::<_, std::io::Error>));
    let body: BoxStream<'static, Result<Bytes, std::io::Error>> = match scripted.ending {
        Ending::Complete | Ending::Silent => chunks.boxed(),
        Ending::Open => {
            let guard = LeaveGuard(state.clone());
            let pings = stream::unfold(guard, |guard| async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Some((Ok(Bytes::from_static(b": ping\n\n")), guard))
            });
            chunks.chain(pings).boxed()
        }
        // A body stream that fails makes the server drop the connection
        // without ending the body. The pause before it lets the server flush
        // what came before, which it does when the body has nothing ready.
        Ending::Broken => {
            let failure = async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Err(std::io::Error::other("MockVendor breaks the connection"))
            };
            chunks.chain(stream::once(failure)).boxed()
        }
    };
    let mut response = Response::new(Body::from_stream(body));
    *response.status_mut() = scripted.status;
    for (name, value) in scripted.headers {
        response.headers_mut().append(name, value);
    }
    response
}

/// A port of the loopback interface for a vendor.
async fn bind() -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a test port is free")
}

/// How long a client has to complete its TLS handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// A listener that completes each connection's TLS handshake before the
/// vendor serves it.
struct TlsListener {
    tcp: tokio::net::TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
}

impl axum::serve::Listener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, address) = axum::serve::Listener::accept(&mut self.tcp).await;
            // A client that fails its handshake, or leaves it unfinished, sent
            // no request; the vendor goes on with the next one.
            let handshake = tokio::time::timeout(HANDSHAKE, self.acceptor.accept(stream)).await;
            if let Ok(Ok(stream)) = handshake {
                return (stream, address);
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

/// Held by an open response's body; the server drops the body when the
/// client leaves.
struct LeaveGuard(Arc<VendorState>);

impl Drop for LeaveGuard {
    fn drop(&mut self) {
        self.0.disconnected.notify_one();
    }
}
