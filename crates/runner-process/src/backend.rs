//! How the runner reaches its backend
//! (`crates/runner-protocol/src/wire/mod.rs`): over the network at the
//! backend's URL; for a Cloud, through the backend's runner socket that the
//! machine manager mounts into the sandbox (`crates/backend-http/src/lib.rs`);
//! or over a channel it was given, its standard input and output, which
//! whoever runs its core carries there (`crates/core-api/src/process/hosts.rs`).
//!
//! Through the socket the URL still names the backend's origin, which requests
//! carry as their host, but no TLS crosses the socket. On a channel, HTTP/2
//! runs on the channel's bytes and a WebSocket is an extended CONNECT
//! (RFC 8441), as on the program's connection to its process; the URL is
//! [`CHANNEL_URL`], which no request leaves the channel for.
//!
//! ```text
//! request ---> BackendHttp --- network or socket ---> reqwest's connections
//!                          `-- channel -------------> HTTP/2 on the channel
//! ```

use std::fmt;
use std::{io, path::PathBuf, str::FromStr, sync::Arc};

use demi_runner_protocol::values::{BackendUrl, ValueError};
use demi_runner_protocol::wire::{Watched, WriteProgress};
use futures_util::future::BoxFuture;
use http_body_util::BodyExt as _;
use hyper::client::conn::http2::SendRequest;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{self, client::IntoClientRequest, handshake::client::Response, protocol::{Role, WebSocketConfig}},
};
use tokio_util::sync::CancellationToken;

/// A connection to the backend, whichever way it went.
pub trait Duplex: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> Duplex for T {}

/// A WebSocket to the backend.
pub type BackendSocket = WebSocketStream<MaybeTlsStream<Box<dyn Duplex>>>;

/// The URL of a backend on a channel: a name under `.localhost`
/// (RFC 6761 § 6.3), as Docker's client names a daemon on a local socket,
/// since requests need a host and the channel has none. It keys the
/// installation (`crates/runner-protocol/src/installation.rs`).
pub const CHANNEL_URL: &str = "http://channel.demi.localhost";

#[derive(Clone)]
pub struct Backend {
    url: BackendUrl,
    way: Way,
}

/// How requests reach the backend.
#[derive(Clone)]
enum Way {
    Network,
    /// The backend's runner socket, which every connection goes through.
    Socket(PathBuf),
    Channel(Channel),
}

impl fmt::Debug for Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let way = match &self.way {
            Way::Network => "network",
            Way::Socket(_) => "socket",
            Way::Channel(_) => "channel",
        };
        formatter
            .debug_struct("Backend")
            .field("url", &self.url.as_str())
            .field("way", &way)
            .finish()
    }
}

impl Backend {
    /// The backend at `url`, over the network.
    pub fn at(url: BackendUrl) -> Self {
        Self { url, way: Way::Network }
    }

    /// The backend of `url` through its runner socket at `socket`.
    pub fn through(url: BackendUrl, socket: PathBuf) -> Self {
        Self {
            url,
            way: Way::Socket(socket),
        }
    }

    /// The backend on `channel`.
    pub fn over(channel: Channel) -> Self {
        Self {
            url: CHANNEL_URL.parse().expect("the channel's URL is a backend's URL"),
            way: Way::Channel(channel),
        }
    }

    pub fn url(&self) -> &BackendUrl {
        &self.url
    }

    /// The origin requests are made to, at the root: the URL's, as HTTP or
    /// HTTPS, and plain HTTP through the socket and on a channel.
    pub fn origin(&self) -> io::Result<reqwest::Url> {
        let mut origin = self.url.url().clone();
        let scheme = match origin.scheme() {
            _ if !matches!(self.way, Way::Network) => "http",
            "ws" | "http" => "http",
            _ => "https",
        };
        origin
            .set_scheme(scheme)
            .map_err(|_| io::Error::other("invalid HTTP origin"))?;
        origin.set_path("/");
        origin.set_query(None);
        Ok(origin)
    }

    /// `builder` with every connection going through the socket, when the
    /// backend is reached through one.
    pub fn http(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        match &self.way {
            #[cfg(unix)]
            Way::Socket(socket) => builder.unix_socket(socket.clone()),
            #[cfg(not(unix))]
            Way::Socket(_) => unreachable!("only a Cloud's runner, on Linux, has a backend socket"),
            Way::Network | Way::Channel(_) => builder,
        }
    }

    /// The backend's client of `client`, which [`http`](Self::http) made:
    /// `client` itself on the network and through the socket, and the
    /// channel's when the backend is on one.
    pub fn client(&self, client: reqwest::Client) -> BackendHttp {
        let channel = match &self.way {
            Way::Channel(channel) => Some(channel.clone()),
            Way::Network | Way::Socket(_) => None,
        };
        BackendHttp { client, channel }
    }

    /// `url`, which the backend handed out, as a request to it: a path from
    /// the root names a place on the backend, such as its own store's
    /// artifact, which goes the way the runner reaches it; anything else
    /// is a URL on the network.
    pub fn place(&self, url: &str) -> io::Result<Option<reqwest::Url>> {
        if !url.starts_with('/') {
            return Ok(None);
        }
        if url.starts_with("//") || url.contains('\\') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a backend's path must stay on its origin"));
        }
        let origin = self.origin()?;
        let place = origin.join(url).map_err(io::Error::other)?;
        if place.origin() != origin.origin() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a backend's path must stay on its origin"));
        }
        Ok(Some(place))
    }

    /// Opens the WebSocket `request` names, on the backend's [`origin`] with
    /// its `ws` or `wss` form.
    ///
    /// [`origin`]: Self::origin
    pub async fn websocket(
        &self,
        request: impl IntoClientRequest + Unpin,
        config: WebSocketConfig,
    ) -> Result<(BackendSocket, Response), tungstenite::Error> {
        let (socket, response, _) = self.watched_websocket(request, config).await?;
        Ok((socket, response))
    }

    /// As [`websocket`](Self::websocket), with whether its writes move,
    /// which ends the runner's link when they stop
    /// (`crates/runner-protocol/src/wire/mod.rs`); on a channel, the
    /// channel's writes.
    pub async fn watched_websocket(
        &self,
        request: impl IntoClientRequest + Unpin,
        config: WebSocketConfig,
    ) -> Result<(BackendSocket, Response, Arc<WriteProgress>), tungstenite::Error> {
        let request = request.into_client_request()?;
        let stream: Box<dyn Duplex> = match &self.way {
            #[cfg(unix)]
            Way::Socket(socket) => Box::new(tokio::net::UnixStream::connect(socket).await?),
            #[cfg(not(unix))]
            Way::Socket(_) => unreachable!("only a Cloud's runner, on Linux, has a backend socket"),
            Way::Channel(channel) => return channel.websocket(request, config).await,
            Way::Network => {
                let uri = request.uri();
                let host = uri
                    .host()
                    .ok_or(tungstenite::Error::Url(tungstenite::error::UrlError::NoHostName))?;
                let host = host.trim_start_matches('[').trim_end_matches(']');
                let port = uri.port_u16().unwrap_or(match uri.scheme_str() {
                    Some("wss") => 443,
                    _ => 80,
                });
                let stream = tokio::net::TcpStream::connect((host, port)).await?;
                stream.set_nodelay(true)?;
                Box::new(stream)
            }
        };
        let (stream, progress) = Watched::new(stream);
        let stream: Box<dyn Duplex> = Box::new(stream);
        let (socket, response) =
            tokio_tungstenite::client_async_tls_with_config(request, stream, Some(config), None)
                .await?;
        Ok((socket, response, progress))
    }

    /// Resolves once the channel the backend is on ended; never for a
    /// backend reached otherwise, which the runner reconnects to.
    pub async fn ended(&self) {
        match &self.way {
            Way::Channel(channel) => channel.ended.cancelled().await,
            Way::Network | Way::Socket(_) => std::future::pending().await,
        }
    }
}

/// A backend on the network, as a URL names it.
impl FromStr for Backend {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, ValueError> {
        Ok(Self::at(value.parse()?))
    }
}

/// The channel a runner was given: HTTP/2 on its bytes, until they end.
#[derive(Clone)]
pub struct Channel {
    requests: SendRequest<reqwest::Body>,
    /// Whether the channel's writes move.
    writes: Arc<WriteProgress>,
    ended: CancellationToken,
}

impl Channel {
    /// Speaks HTTP/2 to the backend on `io`.
    pub async fn open(io: impl Duplex + 'static) -> io::Result<Self> {
        let (io, writes) = Watched::new(io);
        let (requests, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
            .handshake(TokioIo::new(io))
            .await
            .map_err(io::Error::other)?;
        let ended = CancellationToken::new();
        let ending = ended.clone();
        tokio::spawn(async move {
            match connection.await {
                Ok(()) => tracing::info!(event = "channel.ended", "the channel to the backend ended"),
                Err(error) => tracing::warn!(event = "channel.ended", %error, "the channel to the backend broke"),
            }
            ending.cancel();
        });
        Ok(Self {
            requests,
            writes,
            ended,
        })
    }

    async fn send(&self, request: http::Request<reqwest::Body>) -> Result<http::Response<hyper::body::Incoming>, hyper::Error> {
        let mut requests = self.requests.clone();
        requests.ready().await?;
        requests.send_request(request).await
    }

    /// The WebSocket of `request` as an extended CONNECT on the channel.
    async fn websocket(
        &self,
        request: http::Request<()>,
        config: WebSocketConfig,
    ) -> Result<(BackendSocket, Response, Arc<WriteProgress>), tungstenite::Error> {
        let (parts, ()) = request.into_parts();
        let mut uri = parts.uri.into_parts();
        uri.scheme = Some(http::uri::Scheme::HTTP);
        let mut connect = http::Request::new(reqwest::Body::from(Vec::new()));
        *connect.method_mut() = http::Method::CONNECT;
        *connect.uri_mut() = http::Uri::from_parts(uri).map_err(|error| tungstenite::Error::HttpFormat(error.into()))?;
        *connect.headers_mut() = parts.headers;
        // The URI names the host, and the handshake's own fields belong to
        // HTTP/1.1 (RFC 8441 § 5).
        for name in ["host", "connection", "upgrade", "sec-websocket-key"] {
            connect.headers_mut().remove(name);
        }
        connect.extensions_mut().insert(hyper::ext::Protocol::from_static("websocket"));
        let mut answer = self.send(connect).await.map_err(|error| tungstenite::Error::Io(io::Error::other(error)))?;
        if answer.status() != http::StatusCode::OK {
            // As a refused handshake reads: the answer, with its body.
            let (parts, body) = answer.into_parts();
            let body = body.collect().await.map(|body| body.to_bytes().to_vec()).ok();
            return Err(tungstenite::Error::Http(Box::new(http::Response::from_parts(parts, body))));
        }
        let upgrade = hyper::upgrade::on(&mut answer);
        let mut response = http::Response::new(None);
        *response.status_mut() = answer.status();
        *response.version_mut() = answer.version();
        *response.headers_mut() = answer.headers().clone();
        let upgraded = upgrade.await.map_err(|error| tungstenite::Error::Io(io::Error::other(error)))?;
        let stream: Box<dyn Duplex> = Box::new(TokioIo::new(upgraded));
        let socket = WebSocketStream::from_raw_socket(MaybeTlsStream::Plain(stream), Role::Client, Some(config)).await;
        Ok((socket, response, self.writes.clone()))
    }
}

/// The backend's client: reqwest's on the network and through a socket,
/// which also builds every request, and the channel's on a channel.
#[derive(Clone)]
pub struct BackendHttp {
    client: reqwest::Client,
    channel: Option<Channel>,
}

/// Why a request to the backend got no answer.
#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error(transparent)]
    Network(#[from] reqwest::Error),
    #[error("the channel to the backend failed: {0}")]
    Channel(#[source] hyper::Error),
}

impl RequestError {
    /// Whether the request failed on its connection without an answer, as
    /// through a proxy that drops a request now and then: a new connection
    /// may succeed.
    pub fn lost_connection(&self) -> bool {
        match self {
            Self::Network(error) => {
                !error.is_builder() && !error.is_redirect() && !error.is_status() && !error.is_decode()
            }
            // A stream of the channel failed; a new one may succeed, and
            // once the channel ended, the runner ends.
            Self::Channel(_) => true,
        }
    }

    /// Whether the request could not be made at all, and would fail the
    /// same way again.
    pub fn is_builder(&self) -> bool {
        matches!(self, Self::Network(error) if error.is_builder())
    }
}

impl BackendHttp {
    pub fn request(&self, method: reqwest::Method, url: reqwest::Url) -> reqwest::RequestBuilder {
        self.client.request(method, url)
    }

    pub async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, RequestError> {
        let Some(channel) = &self.channel else {
            return Ok(request.send().await?);
        };
        let request: http::Request<reqwest::Body> = request.build()?.try_into()?;
        let answer = channel.send(request).await.map_err(RequestError::Channel)?;
        Ok(reqwest::Response::from(answer.map(reqwest::Body::wrap)))
    }
}

/// A download from the backend goes the way the runner reaches it
/// (`crates/shared-artifacts/src/download.rs`); its client follows no
/// redirect, as the client it was made of must not.
impl demi_shared_artifacts::Fetch for BackendHttp {
    fn fetch(
        &self,
        url: reqwest::Url,
        headers: reqwest::header::HeaderMap,
    ) -> BoxFuture<'_, Result<reqwest::Response, demi_shared_artifacts::Unsent>> {
        Box::pin(async move {
            let request = self.client.get(url).headers(headers);
            self.send(request).await.map_err(|error| match error {
                RequestError::Network(error) => demi_shared_artifacts::Unsent::of(error),
                RequestError::Channel(error) => demi_shared_artifacts::Unsent::Connection(
                    demi_shared_artifacts::Error::Download(format!("the channel to the backend failed: {error}")),
                ),
            })
        })
    }
}
