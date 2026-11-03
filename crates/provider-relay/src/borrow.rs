//! The borrower's end: a provider whose requests another server runs.
//!
//! The provider reads what the lender says of the lent entry, its name, its
//! catalog and each model's limits, over HTTPS. Each runtime holds one
//! socket, opened by its first run and closed with it. A run sends its request
//! as what changed (`crate::delta`) and takes the events of its run; one the
//! session cancelled or dropped is cancelled on the socket before the next
//! run, whose events then follow.
//!
//! The borrower composes every failure's message from its code: nothing of
//! the lender's failure crosses but the code.
//!
//! Its media are made by the lender too, each kind by one maker (`media`)
//! that sends each call over HTTPS. What it makes is what the lender lists
//! now: a kind the lender does not make lists no models.
//!
//! The entry keeps its name, its key and its switch, and nothing it can
//! read from the lender: the lender's own settings, its models' and its
//! media's switches and its provider's, are what a borrower uses.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use demi_provider_common::{
    CatalogError, ErrorCode, Provider, ProviderEvent, ProviderFailure, ProviderHttp, ProviderRun, ProviderRuntime,
    RequestBuilder, RequestLimits, RuntimeEnv, RuntimeError, InferenceRequest,
};
use demi_provider_common::audio::AudioMaker;
use demi_provider_common::images::ImageMaker;
use demi_provider_common::videos::VideoMaker;
use demi_shared_types::{
    AuthState, MediaKind, Model, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState,
    Timestamp,
};
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::stream::{SplitStream, StreamExt as _};
use futures_util::SinkExt as _;
use http::HeaderValue;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, Message};
use url::Url;

use crate::delta::Outbox;
use crate::key::ShareKey;
use crate::wire::{
    ClientMessage, Lent, MESSAGE_BYTES, OtherVersion, PROVIDER_ROUTE, SERVER_HEADER, SOCKET_ROUTE, ServerMessage,
    TURNED_OFF, VERSION, VERSION_HEADER,
};

/// The family of a borrowed entry, which holds the share key it was
/// borrowed with.
pub const FAMILY: &str = "relay";

mod media;

/// A provider another server lends.
pub struct Borrowed {
    config: Arc<Config>,
    images: media::Relayed,
    videos: media::Relayed,
    audio: media::Relayed,
}

struct Config {
    /// The entry's name, which a provider the lender turned off is told by.
    name: String,
    key: ShareKey,
    /// The lender as the borrower's messages name it: its host.
    lender: String,
    /// This server as the lender shows it: the host of its public URL.
    server: String,
    http: ProviderHttp,
    /// Each model's limits, as the lender last said them.
    limits: RwLock<HashMap<String, RequestLimits>>,
}

/// Why the lender did not answer as a lender.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Refusal {
    /// The key is revoked, expired or unknown.
    Key,
    /// The lender speaks another version of the wire.
    Version(u32),
    /// The server is no lender: an older release, or no Demi.
    NoLender,
    /// It could not be reached, or answered nothing a lender answers.
    Unreachable(String),
    /// The lender turned the provider off; it lends again once it is on.
    TurnedOff,
}

impl Refusal {
    fn message(&self, config: &Config) -> String {
        let lender = &config.lender;
        match self {
            Self::TurnedOff => format!("{} is turned off on {lender}.", config.name),
            Self::Key => format!("{lender} no longer accepts this share key. Paste a new one."),
            Self::Version(version) if *version > VERSION => {
                format!("{lender} runs a newer version of Demi. Update this server.")
            }
            Self::Version(_) => format!("{lender} runs an older version of Demi. Update it."),
            Self::NoLender => format!("{lender} does not share providers. Update it."),
            Self::Unreachable(cause) => format!("{lender} could not be reached: {cause}"),
        }
    }

    fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Key => Some(ErrorCode::AuthExpired),
            Self::Unreachable(_) => Some(ErrorCode::Overloaded),
            Self::Version(_) | Self::NoLender | Self::TurnedOff => None,
        }
    }

    fn failure(&self, config: &Config) -> ProviderFailure {
        ProviderFailure {
            message: self.message(config),
            code: self.code(),
            diagnostics: None,
            retry_after: None,
        }
    }

    /// The refusal an answer of `status` with `body` says.
    fn of(status: http::StatusCode, body: &[u8]) -> Self {
        match status.as_u16() {
            401 | 403 => Self::Key,
            404 | 405 => Self::NoLender,
            TURNED_OFF => Self::TurnedOff,
            409 => match serde_json::from_slice::<OtherVersion>(body) {
                Ok(other) => Self::Version(other.version),
                Err(_) => Self::NoLender,
            },
            status => Self::Unreachable(format!("HTTP {status}")),
        }
    }
}

/// The message of a failure the lender reported with `code`.
fn failed(lender: &str, code: Option<&ErrorCode>) -> String {
    match code {
        Some(ErrorCode::UsageLimit | ErrorCode::RateLimit) => format!("The provider on {lender} reached its limit."),
        Some(ErrorCode::Overloaded | ErrorCode::DeviceOffline) => {
            format!("The provider on {lender} is not available right now.")
        }
        Some(ErrorCode::ContextLengthExceeded) => {
            format!("The conversation is longer than the model on {lender} accepts.")
        }
        Some(ErrorCode::ModelUnavailable) => format!("The model is not available on {lender}."),
        Some(ErrorCode::RequestTooLarge) => format!("The request is larger than the provider on {lender} accepts."),
        Some(ErrorCode::Incomplete) => format!("The provider on {lender} ended the reply early."),
        Some(ErrorCode::Filtered) => format!("The provider on {lender} stopped the reply."),
        Some(code) => format!("The provider on {lender} could not answer ({}).", code.as_str()),
        None => format!("The provider on {lender} could not answer."),
    }
}

/// How a server reached at `public` names itself to a lender: the host of its
/// public URL, or the machine's name when that is a loopback address, which
/// would name no machine to the lender.
pub fn server_name(public: Option<&Url>) -> String {
    let host = public.and_then(|url| url.host());
    let loopback = match &host {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => *name == "localhost",
        None => true,
    };
    match host {
        Some(host) if !loopback => host.to_string(),
        _ => hostname::get().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

impl Borrowed {
    /// The provider `key` lends to the entry `name`, asked for by `server`,
    /// the host of this server's public URL, with `http`, the entry's client.
    pub fn new(key: ShareKey, name: String, server: String, http: ProviderHttp) -> Self {
        let lender = key.url.host_str().unwrap_or("the lending server").to_owned();
        let config = Arc::new(Config {
            name,
            key,
            lender,
            server,
            http,
            limits: RwLock::new(HashMap::new()),
        });
        let maker = |kind| media::Relayed::new(config.clone(), kind);
        Self {
            images: maker(MediaKind::Image),
            videos: maker(MediaKind::Video),
            audio: maker(MediaKind::Audio),
            config,
        }
    }

    /// What the lender says of the lent entry now.
    pub async fn lent(&self) -> Result<Lent, String> {
        self.config.lent().await.map_err(|refusal| refusal.message(&self.config))
    }
}

impl Config {
    fn route(&self, route: &str) -> Url {
        self.key.url.join(route.trim_start_matches('/')).expect("a route joins a URL")
    }

    /// `request` with what every request to the lender carries: the key,
    /// the wire's version and this server's name.
    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header(http::header::AUTHORIZATION, format!("Bearer {}", self.key.token))
            .header(VERSION_HEADER, VERSION.to_string())
            .header(SERVER_HEADER, self.server.clone())
    }

    async fn lent(&self) -> Result<Lent, Refusal> {
        let answer = self
            .authorized(self.http.get(self.route(PROVIDER_ROUTE)))
            .send()
            .await
            .map_err(|error| Refusal::Unreachable(error.to_string()))?;
        let status = answer.status();
        let body = answer.bytes().await.map_err(|error| Refusal::Unreachable(error.to_string()))?;
        if !status.is_success() {
            return Err(Refusal::of(status, &body));
        }
        let lent: Lent = serde_json::from_slice(&body).map_err(|_| Refusal::NoLender)?;
        let mut limits = self.limits.write().expect("the limits' lock is never poisoned");
        *limits = lent.limits.iter().map(|model| (model.model.clone(), model.limits.into())).collect();
        drop(limits);
        Ok(lent)
    }

    /// What the lender lends now: a provider it turned off is refused.
    async fn lending(&self) -> Result<Lent, Refusal> {
        let lent = self.lent().await?;
        if lent.turned_off {
            return Err(Refusal::TurnedOff);
        }
        Ok(lent)
    }
}

impl Provider for Borrowed {
    fn capabilities(&self) -> demi_provider_common::Capabilities {
        demi_provider_common::Capabilities::default()
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async move {
            match self.config.lending().await {
                Ok(_) => AuthState::Authenticated { account_label: None },
                Err(Refusal::Key) => AuthState::Unauthenticated {
                    message: Some(Refusal::Key.message(&self.config)),
                },
                Err(refusal) => AuthState::Unknown {
                    message: Some(refusal.message(&self.config)),
                },
            }
        })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready {
            message: Some(format!("Uses a provider {} shares", self.config.lender)),
        }
    }

    /// The lender's catalog; a provider it turned off lists nothing, and says
    /// so, rather than failing, so the list a page keeps is emptied.
    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(async move {
            match self.config.lent().await {
                Ok(mut lent) if lent.turned_off => {
                    lent.catalog.warnings = vec![Refusal::TurnedOff.message(&self.config)];
                    Ok(lent.catalog)
                }
                Ok(lent) => Ok(lent.catalog),
                Err(Refusal::Key) => Err(CatalogError::Unauthenticated(Refusal::Key.message(&self.config))),
                Err(refusal) => Err(CatalogError::Unavailable(refusal.message(&self.config))),
            }
        })
    }

    fn read_failure(&self, _: &ProviderErrorDiagnostics, _: Timestamp) -> ProviderFailureFacts {
        ProviderFailureFacts {
            retry_at: None,
            continue_with: None,
            signed_out: None,
            device_offline: None,
        }
    }

    fn images(&self) -> Option<&dyn ImageMaker> {
        Some(&self.images)
    }

    fn videos(&self) -> Option<&dyn VideoMaker> {
        Some(&self.videos)
    }

    fn audio(&self) -> Option<&dyn AudioMaker> {
        Some(&self.audio)
    }

    fn runtime(&self, _env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(BorrowedRuntime {
            config: self.config.clone(),
            connection: None,
            next_run: 0,
        }))
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A session's runtime: its socket, once a run opened it.
struct BorrowedRuntime {
    config: Arc<Config>,
    connection: Option<Connection>,
    next_run: u64,
}

/// An open socket and what its end keeps.
struct Connection {
    /// To the task that writes the socket, so `idle` sends without waiting.
    send: mpsc::UnboundedSender<ClientMessage>,
    receive: SplitStream<Socket>,
    writer: tokio::task::JoinHandle<()>,
    outbox: Outbox,
    /// The run whose end has not come: one cancelled, or whose stream the
    /// session dropped.
    unended: Option<u64>,
}

impl Connection {
    async fn open(config: &Config) -> Result<Self, Refusal> {
        let mut url = config.route(SOCKET_ROUTE);
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme).expect("ws and wss are schemes of an http URL");
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| Refusal::Unreachable(error.to_string()))?;
        let headers = request.headers_mut();
        let header = |value: String| HeaderValue::try_from(value).map_err(|error| Refusal::Unreachable(error.to_string()));
        headers.insert(http::header::AUTHORIZATION, header(format!("Bearer {}", config.key.token))?);
        headers.insert(VERSION_HEADER, header(VERSION.to_string())?);
        headers.insert(SERVER_HEADER, header(config.server.clone())?);
        let socket_config = WebSocketConfig::default()
            .max_message_size(Some(MESSAGE_BYTES))
            .max_frame_size(Some(MESSAGE_BYTES));
        let (socket, _) = tokio_tungstenite::connect_async_with_config(request, Some(socket_config), false)
            .await
            .map_err(|error| match error {
                tungstenite::Error::Http(response) => {
                    Refusal::of(response.status(), response.body().as_deref().unwrap_or_default())
                }
                error => Refusal::Unreachable(error.to_string()),
            })?;
        let (mut sink, receive) = socket.split();
        let (send, mut outgoing) = mpsc::unbounded_channel::<ClientMessage>();
        let writer = tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                let text = serde_json::to_string(&message).expect("a relay message serializes");
                if sink.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
            }
            let _ = sink.close().await;
        });
        Ok(Self {
            send,
            receive,
            writer,
            outbox: Outbox::default(),
            unended: None,
        })
    }

    /// The next message of the lender's; none once the socket closed or
    /// broke the wire.
    async fn next(&mut self) -> Option<ServerMessage> {
        loop {
            match self.receive.next().await? {
                Ok(Message::Text(text)) => return serde_json::from_str(&text).ok(),
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                Ok(_) | Err(_) => return None,
            }
        }
    }

    /// Waits for the end of the run left unended; false when the socket
    /// closed meanwhile.
    async fn settle(&mut self) -> bool {
        let Some(run) = self.unended else { return true };
        loop {
            match self.next().await {
                Some(ServerMessage::End { run: ended }) if ended == run => {
                    self.unended = None;
                    return true;
                }
                Some(_) => {}
                None => return false,
            }
        }
    }

    async fn close(self) {
        drop(self.send);
        let _ = self.writer.await;
    }
}

impl BorrowedRuntime {
    /// The socket, open and with no run left unended.
    async fn ready(&mut self) -> Result<&mut Connection, Refusal> {
        if let Some(connection) = &mut self.connection {
            if connection.settle().await {
                return Ok(self.connection.as_mut().expect("the connection is kept"));
            }
            self.connection = None;
        }
        let connection = Connection::open(&self.config).await?;
        // A catalog read from the cache since the backend started told no
        // limits; the socket's first run takes them.
        let unknown = self.config.limits.read().expect("the limits' lock is never poisoned").is_empty();
        if unknown {
            let _ = self.config.lent().await;
        }
        Ok(self.connection.insert(connection))
    }

    /// Why the socket closed during a run, as the lender tells it now: a
    /// revoked key closes its sessions, and its run then says so rather
    /// than that the connection was lost.
    async fn lost(&self) -> ProviderFailure {
        let refusal = match self.config.lending().await {
            Err(refusal) => refusal,
            Ok(_) => Refusal::Unreachable("the connection was lost".into()),
        };
        refusal.failure(&self.config)
    }
}

impl ProviderRuntime for BorrowedRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        async_stream::stream! {
            let config = self.config.clone();
            let run = self.next_run;
            self.next_run += 1;
            let connection = match self.ready().await {
                Ok(connection) => connection,
                Err(refusal) => {
                    yield ProviderEvent::Error(refusal.failure(&config));
                    return;
                }
            };
            let wire = connection.outbox.send(&request);
            connection.unended = Some(run);
            let mut broken = connection.send.send(ClientMessage::Run { run, request: wire }).is_err();
            while !broken {
                tokio::select! {
                    biased;
                    () = request.cancel.cancelled() => {
                        let _ = connection.send.send(ClientMessage::Cancel { run });
                        return;
                    }
                    message = connection.next() => match message {
                        Some(ServerMessage::Event { run: of, event }) if of == run => {
                            yield event.borrowed(|code| failed(&config.lender, code));
                        }
                        Some(ServerMessage::End { run: ended }) if ended == run => {
                            connection.unended = None;
                            return;
                        }
                        Some(_) => {}
                        None => broken = true,
                    },
                }
            }
            self.connection = None;
            yield ProviderEvent::Error(self.lost().await);
        }
        .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self {
            config: self.config.clone(),
            connection: None,
            next_run: 0,
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some(connection) = self.connection.take() {
                connection.close().await;
            }
        })
    }

    fn idle(&mut self) {
        if let Some(connection) = &self.connection {
            let _ = connection.send.send(ClientMessage::Idle);
        }
    }

    fn request_limits(&self, model: &Model) -> RequestLimits {
        self.config
            .limits
            .read()
            .expect("the limits' lock is never poisoned")
            .get(&model.id)
            .copied()
            .unwrap_or_default()
    }
}
