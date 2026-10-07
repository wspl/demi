//! The tests' relay and sites: the relay speaks the stream as the page's
//! relay does, and the sites answer on loopback under the names the
//! engine's test network gives them.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message as SocketMessage, WebSocketUpgrade};
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_command_package_browser_preview::testing::{Network, Space};
use demi_command_package_browser_preview::{Engine, PageStates, StreamError, TakenState};
use demi_command_package_browser_protocol::preview::{
    BodyHeader, CHUNK_FRAME, CONTROL_FRAME, PageStorage, PreviewClient, PreviewCredentials, PreviewEngineMessage,
    PreviewEnvironment, PreviewHeader, PreviewMode, PreviewRelayMessage, PreviewRequest, PreviewScheme, REQUEST_BODY_FRAME,
    SOCKET_MESSAGE_FRAME, SocketHeader,
};
use demi_preview_rewrite::address::{Environment, label, site_of};
use futures_util::StreamExt;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

pub const DOMAIN: &str = "preview.test";
pub const NAMESPACE: &str = "k3f9a2ab";
pub const HOST: &str = "host-1";
pub const CHROME_154: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36";

const AUTHORITY: &[u8] = include_bytes!("certificates/authority.pem");
const CERTIFICATE: &[u8] = include_bytes!("certificates/site.pem");
const KEY: &[u8] = include_bytes!("certificates/site.key");
/// Guards against a hang only: every wait ends with its event.
const DEADLINE: Duration = Duration::from_secs(10);

/// An engine of the test network: `www.site.test` and `api.site.test` are
/// one public site, `other.test` another, `local.test` a device on the
/// local network; the literal loopback addresses and `localhost` are
/// loopback. Its jar lives in `directory`.
pub fn engine(directory: &tempfile::TempDir) -> Arc<Engine> {
    Engine::open_for_tests(jar(directory), network()).unwrap()
}

pub fn network() -> Network {
    Network::new(AUTHORITY)
        .host("site.test", Space::Public)
        .host("www.site.test", Space::Public)
        .host("api.site.test", Space::Public)
        .host("other.test", Space::Public)
        .host("local.test", Space::Local)
}

pub fn jar(directory: &tempfile::TempDir) -> std::path::PathBuf {
    directory.path().join("browser/preview-cookies.json")
}

/// A desktop Chrome 154 on macOS.
pub fn client() -> PreviewClient {
    PreviewClient {
        user_agent: CHROME_154.into(),
        brands: r#""Chromium";v="154", "Google Chrome";v="154", "Not.A/Brand";v="99""#.into(),
        mobile: false,
        platform: "macOS".into(),
        accept_language: "en-US,en;q=0.9".into(),
    }
}

/// The environment of a preview tab's top document at `origin`.
pub fn top(origin: &str) -> PreviewEnvironment {
    PreviewEnvironment {
        origin: origin.into(),
        top: site_of(origin),
        cross: false,
    }
}

/// The environment of a document at `origin` embedded under `top`.
pub fn embedded(origin: &str, top: &str, cross: bool) -> PreviewEnvironment {
    PreviewEnvironment {
        origin: origin.into(),
        top: top.into(),
        cross,
    }
}

/// The preview origin of an environment, as the relay computes it.
pub fn preview_origin(environment: &PreviewEnvironment) -> String {
    let environment = Environment {
        origin: environment.origin.clone(),
        top: environment.top.clone(),
        cross: environment.cross,
    };
    format!("https://{NAMESPACE}--{}.{DOMAIN}", label(NAMESPACE, HOST, &environment))
}

/// A request a document makes: `GET`, no body, credentials included.
pub fn request(url: &str, mode: PreviewMode, destination: &str, initiator: Option<PreviewEnvironment>) -> PreviewRequest {
    PreviewRequest {
        url: url.into(),
        method: "GET".into(),
        headers: Vec::new(),
        body: false,
        mode,
        destination: destination.into(),
        credentials: PreviewCredentials::Include,
        referrer: String::new(),
        referrer_policy: String::new(),
        keepalive: false,
        initiator,
        user: false,
    }
}

/// The user's own opening of `url` in a preview tab.
pub fn opening(url: &str) -> PreviewRequest {
    PreviewRequest {
        user: true,
        ..request(url, PreviewMode::Navigate, "document", None)
    }
}

/// A frame of the engine.
#[derive(Debug)]
pub enum Frame {
    Control(PreviewEngineMessage),
    Chunk { id: u32, data: Bytes },
    Socket { id: u32, binary: bool, data: Bytes },
}

/// An answer the relay received whole.
#[derive(Debug)]
pub struct Fetched {
    pub status: u16,
    pub headers: Vec<PreviewHeader>,
    pub labels: BTreeMap<String, PreviewEnvironment>,
    pub body: Bytes,
}

impl Fetched {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap()
    }

    /// The request the echo route saw.
    pub fn echoed(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

/// A header of the request an echo route saw.
pub fn echoed_header<'v>(echoed: &'v Value, name: &str) -> Option<&'v str> {
    echoed["headers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pair| pair[0].as_str().unwrap().eq_ignore_ascii_case(name))
        .map(|pair| pair[1].as_str().unwrap())
}

/// A conversation that runs no browser: no page state moves.
struct NoBrowser;

impl PageStates for NoBrowser {
    fn take(&self, _tab: String) -> futures_util::future::BoxFuture<'static, Result<TakenState, String>> {
        Box::pin(async { Err("No browser runs.".to_owned()) })
    }

    fn keep(&self, _token: String, _sites: Vec<String>, _storage: Option<PageStorage>) {
        unreachable!("no test keeps a page state without a browser")
    }
}

/// The page's relay: it frames its messages onto the engine's stream and
/// splits the engine's.
pub struct Relay {
    input: Option<mpsc::UnboundedSender<std::io::Result<Bytes>>>,
    output: mpsc::UnboundedReceiver<Bytes>,
    pending: BytesMut,
    served: Option<tokio::task::JoinHandle<Result<(), StreamError>>>,
    next_id: u32,
    /// The browser the relay's requests describe.
    pub client: PreviewClient,
}

impl Relay {
    /// A stream of `engine`, after its hello.
    pub fn open(engine: Arc<Engine>) -> Self {
        Self::open_on(engine, PreviewScheme::Https, DOMAIN)
    }

    /// A stream of `engine` whose conversation's browser is `states`, after its hello.
    pub fn open_with(engine: Arc<Engine>, states: Arc<dyn PageStates>) -> Self {
        let relay = Self::serving(engine, states);
        relay.hello(PreviewScheme::Https, DOMAIN);
        relay
    }

    /// A stream of `engine` whose previews live under `scheme` and `domain`.
    pub fn open_on(engine: Arc<Engine>, scheme: PreviewScheme, domain: &str) -> Self {
        let relay = Self::unopened(engine);
        relay.hello(scheme, domain);
        relay
    }

    fn hello(&self, scheme: PreviewScheme, domain: &str) {
        self.send(&PreviewRelayMessage::Hello {
            scheme,
            domain: domain.into(),
            namespace: NAMESPACE.into(),
            host: HOST.into(),
        });
    }

    /// A stream of `engine` that has heard nothing yet, whose conversation
    /// runs no browser.
    pub fn unopened(engine: Arc<Engine>) -> Self {
        Self::serving(engine, Arc::new(NoBrowser))
    }

    fn serving(engine: Arc<Engine>, states: Arc<dyn PageStates>) -> Self {
        let (input, received) = mpsc::unbounded_channel();
        let (sent, output) = mpsc::unbounded_channel::<Bytes>();
        let stream = futures_util::stream::unfold(received, |mut received| async move {
            received.recv().await.map(|item| (item, received))
        });
        let sink = futures_util::sink::unfold(sent, |sent, bytes: Bytes| async move {
            sent.send(bytes).map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(sent)
        });
        let served = tokio::spawn(demi_command_package_browser_preview::serve(
            engine,
            states,
            Box::pin(stream),
            Box::pin(sink),
            CancellationToken::new(),
        ));
        Self {
            input: Some(input),
            output,
            pending: BytesMut::new(),
            served: Some(served),
            next_id: 0,
            client: client(),
        }
    }

    pub fn raw(&self, bytes: impl Into<Bytes>) {
        // A stream that ended refuses input; the test reads why from its end.
        let _ = self.input.as_ref().unwrap().send(Ok(bytes.into()));
    }

    fn framed(&self, kind: u8, payload: &[u8]) {
        let mut bytes = BytesMut::new();
        bytes.put_u32(1 + payload.len() as u32);
        bytes.put_u8(kind);
        bytes.put_slice(payload);
        self.raw(bytes.freeze());
    }

    pub fn send(&self, message: &PreviewRelayMessage) {
        self.framed(CONTROL_FRAME, &serde_json::to_vec(message).unwrap());
    }

    pub fn request_body(&self, id: u32, data: &[u8]) {
        let mut payload = Vec::new();
        BodyHeader { id }.write(&mut payload);
        payload.extend_from_slice(data);
        self.framed(REQUEST_BODY_FRAME, &payload);
    }

    pub fn socket_message(&self, id: u32, binary: bool, data: &[u8]) {
        let mut payload = Vec::new();
        SocketHeader { id, binary }.write(&mut payload);
        payload.extend_from_slice(data);
        self.framed(SOCKET_MESSAGE_FRAME, &payload);
    }

    /// The engine's next frame.
    pub async fn next(&mut self) -> Frame {
        tokio::time::timeout(DEADLINE, self.read()).await.expect("the engine's next frame")
    }

    async fn read(&mut self) -> Frame {
        loop {
            if self.pending.len() >= 4 {
                let length = u32::from_be_bytes(self.pending[..4].try_into().unwrap()) as usize;
                if self.pending.len() >= 4 + length {
                    self.pending.advance(4);
                    let mut frame = self.pending.split_to(length).freeze();
                    return match frame.get_u8() {
                        CONTROL_FRAME => Frame::Control(PreviewEngineMessage::decode(&frame).unwrap()),
                        CHUNK_FRAME => {
                            let (header, data) = BodyHeader::split(&frame).unwrap();
                            Frame::Chunk {
                                id: header.id,
                                data: frame.slice_ref(data),
                            }
                        }
                        SOCKET_MESSAGE_FRAME => {
                            let (header, data) = SocketHeader::split(&frame).unwrap();
                            Frame::Socket {
                                id: header.id,
                                binary: header.binary,
                                data: frame.slice_ref(data),
                            }
                        }
                        kind => panic!("the engine sends no frame of kind {kind}"),
                    };
                }
            }
            let bytes = self.output.recv().await.expect("the stream's output is open");
            self.pending.extend_from_slice(&bytes);
        }
    }

    pub fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// Sends `request` for `environment` and reads its whole answer: its
    /// body is sent on the engine's pulls, the answer's pulled chunk by
    /// chunk. A failed request answers its reason.
    pub async fn fetch(&mut self, environment: PreviewEnvironment, request: PreviewRequest) -> Result<Fetched, String> {
        self.fetch_with_body(environment, request, &[]).await
    }

    pub async fn fetch_with_body(
        &mut self,
        environment: PreviewEnvironment,
        mut request: PreviewRequest,
        body: &[u8],
    ) -> Result<Fetched, String> {
        let id = self.id();
        request.body = !body.is_empty();
        self.send(&PreviewRelayMessage::Request {
            id,
            environment,
            request,
            client: self.client.clone(),
        });
        let mut chunks = body.chunks(100_000);
        let (status, headers, labels) = loop {
            match self.next().await {
                Frame::Control(PreviewEngineMessage::Pull { id: pulled }) if pulled == id => {
                    self.request_body(id, chunks.next().unwrap_or_default());
                }
                Frame::Control(PreviewEngineMessage::Response {
                    id: answered,
                    status,
                    headers,
                    labels,
                }) if answered == id => break (status, headers, labels),
                Frame::Control(PreviewEngineMessage::Failed { id: failed, reason }) if failed == id => {
                    return Err(reason);
                }
                other => panic!("unexpected {other:?}"),
            }
        };
        let mut body = BytesMut::new();
        loop {
            self.send(&PreviewRelayMessage::Pull { id });
            match self.next().await {
                Frame::Chunk { id: answered, data } if answered == id => {
                    if data.is_empty() {
                        break;
                    }
                    body.extend_from_slice(&data);
                }
                Frame::Control(PreviewEngineMessage::Failed { id: failed, reason }) if failed == id => {
                    return Err(reason);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        Ok(Fetched {
            status,
            headers,
            labels,
            body: body.freeze(),
        })
    }

    /// Ends the relay's side and answers how the stream ended.
    pub async fn end(mut self) -> Result<(), StreamError> {
        self.input = None;
        self.ended().await
    }

    /// How the stream ended, by itself.
    pub async fn ended(&mut self) -> Result<(), StreamError> {
        tokio::time::timeout(DEADLINE, self.served.take().unwrap())
            .await
            .expect("the stream ends")
            .unwrap()
    }
}

/// A request a site received.
#[derive(Clone, Debug)]
pub struct Received {
    pub method: String,
    pub path: String,
}

/// The fixture sites: one HTTP and one HTTPS port on loopback, answering for
/// every name of the test network.
///
/// - `/echo?…`: the request it received, as JSON: method, path, headers in
///   order, body.
/// - `/content?type=…&body=…`: that body with that content type.
/// - `/socket`: a WebSocket that echoes each message.
///
/// Every route but the socket also takes `h=<name>:<value>`, repeated, for
/// headers of its answer, and `status=`.
pub struct Site {
    pub http: u16,
    pub https: u16,
    received: Arc<Mutex<Vec<Received>>>,
    _serving: [AbortOnDropHandle<()>; 2],
}

impl Site {
    pub async fn start() -> Self {
        let received = Arc::new(Mutex::new(Vec::new()));
        let router = {
            let received = received.clone();
            Router::new().route("/socket", get(socket)).fallback(any(move |request: Request| {
                let received = received.clone();
                async move { answer(request, &received).await }
            }))
        };
        let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let secure = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http = plain.local_addr().unwrap().port();
        let https = secure.local_addr().unwrap().port();
        Self {
            http,
            https,
            received,
            _serving: [
                AbortOnDropHandle::new(tokio::spawn(serve(plain, None, router.clone()))),
                AbortOnDropHandle::new(tokio::spawn(serve(secure, Some(tls()), router))),
            ],
        }
    }

    pub fn https(&self, host: &str, path: &str) -> String {
        format!("https://{host}:{}{path}", self.https)
    }

    /// The origin of `host` over HTTPS.
    pub fn origin(&self, host: &str) -> String {
        self.https(host, "")
    }

    pub fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

fn tls() -> tokio_rustls::TlsAcceptor {
    use tokio_rustls::rustls::pki_types::pem::PemObject as _;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use tokio_rustls::rustls::{ServerConfig, crypto::aws_lc_rs};
    let chain = CertificateDer::pem_slice_iter(CERTIFICATE).collect::<Result<Vec<_>, _>>().unwrap();
    let key = PrivateKeyDer::from_pem_slice(KEY).unwrap();
    let mut config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

/// Accepts connections until aborted; aborting ends the connections too.
async fn serve(listener: TcpListener, tls: Option<tokio_rustls::TlsAcceptor>, router: Router) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let (tls, router) = (tls.clone(), router.clone());
        connections.spawn(async move {
            let service = TowerToHyperService::new(router);
            let builder = auto::Builder::new(TokioExecutor::new());
            // A connection the client drops ends here; that is the client's
            // choice.
            match tls {
                Some(tls) => {
                    let Ok(stream) = tls.accept(stream).await else { return };
                    let _ = builder.serve_connection_with_upgrades(TokioIo::new(stream), service).await;
                }
                None => {
                    let _ = builder.serve_connection_with_upgrades(TokioIo::new(stream), service).await;
                }
            }
        });
    }
}

async fn answer(request: Request, received: &Mutex<Vec<Received>>) -> Response {
    let (parts, body) = request.into_parts();
    received.lock().unwrap().push(Received {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
    });
    let query: Vec<(String, String)> = url::form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes())
        .into_owned()
        .collect();
    let parameter = |name: &str| query.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
    let body = axum::body::to_bytes(body, 16 * 1024 * 1024).await.unwrap();
    let (content_type, content) = if parts.uri.path() == "/echo" {
        let echoed = json!({
            "method": parts.method.as_str(),
            "path": parts.uri.path(),
            "headers": parts.headers.iter().map(|(name, value)| [name.as_str(), value.to_str().unwrap_or_default()]).collect::<Vec<_>>(),
            "body": String::from_utf8_lossy(&body),
        });
        ("application/json".to_owned(), Bytes::from(echoed.to_string()))
    } else {
        (
            parameter("type").unwrap_or_else(|| "text/plain".into()),
            Bytes::from(parameter("body").unwrap_or_default()),
        )
    };
    let mut response = (
        axum::http::StatusCode::from_u16(parameter("status").map_or(200, |status| status.parse().unwrap())).unwrap(),
        content,
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert("content-type", content_type.parse().unwrap());
    for (_, header) in query.iter().filter(|(key, _)| key == "h") {
        let (name, value) = header.split_once(':').unwrap();
        headers.append(
            axum::http::HeaderName::try_from(name.trim()).unwrap(),
            value.trim().parse().unwrap(),
        );
    }
    response
}

async fn socket(upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .protocols(["chat"])
        .on_upgrade(|mut socket| async move {
            // A close is answered by the socket itself, with the same code,
            // as the next read flushes it.
            while let Some(Ok(message)) = socket.next().await {
                let echoed = match message {
                    SocketMessage::Text(_) | SocketMessage::Binary(_) => message,
                    _ => continue,
                };
                if socket.send(echoed).await.is_err() {
                    return;
                }
            }
        })
}

/// `value` percent-encoded for a query.
pub fn encoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
