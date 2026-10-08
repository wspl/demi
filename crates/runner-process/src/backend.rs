//! How the runner reaches its backend (`runner.md` § Connection and
//! identity): over the network at the backend's URL, or, for a Cloud, through
//! the backend's runner socket that the machine manager mounts into the
//! sandbox (`managed-hosts.md` § Backend socket). Through the socket the URL
//! still names the backend's origin, which requests carry as their host, but
//! no TLS crosses the socket.

use std::{io, path::PathBuf, str::FromStr};

use demi_runner_protocol::values::{BackendUrl, ValueError};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{self, client::IntoClientRequest, handshake::client::Response, protocol::WebSocketConfig},
};

/// A connection to the backend, whichever way it went.
pub trait Duplex: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> Duplex for T {}

/// A WebSocket to the backend.
pub type BackendSocket = WebSocketStream<MaybeTlsStream<Box<dyn Duplex>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backend {
    url: BackendUrl,
    /// The backend's runner socket, which every connection goes through.
    socket: Option<PathBuf>,
}

impl Backend {
    /// The backend at `url`, over the network.
    pub fn at(url: BackendUrl) -> Self {
        Self { url, socket: None }
    }

    /// The backend of `url` through its runner socket at `socket`.
    pub fn through(url: BackendUrl, socket: PathBuf) -> Self {
        Self {
            url,
            socket: Some(socket),
        }
    }

    pub fn url(&self) -> &BackendUrl {
        &self.url
    }

    /// The origin requests are made to, at the root: the URL's, as HTTP or
    /// HTTPS, and plain HTTP through the socket.
    pub fn origin(&self) -> io::Result<reqwest::Url> {
        let mut origin = self.url.url().clone();
        let scheme = match origin.scheme() {
            _ if self.socket.is_some() => "http",
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
        match &self.socket {
            #[cfg(unix)]
            Some(socket) => builder.unix_socket(socket.clone()),
            #[cfg(not(unix))]
            Some(_) => unreachable!("only a Cloud's runner, on Linux, has a backend socket"),
            None => builder,
        }
    }

    /// `url` as this backend's HTTP client requests it, when it lies on the
    /// backend's origin and the backend is reached through its socket; a URL
    /// elsewhere, or any URL when the backend is on the network, is fetched
    /// as it is.
    pub fn local(&self, url: &reqwest::Url) -> Option<reqwest::Url> {
        self.socket.as_ref()?;
        let public = Self::at(self.url.clone()).origin().ok()?;
        if url.origin() != public.origin() {
            return None;
        }
        let mut local = url.clone();
        local.set_scheme("http").ok()?;
        Some(local)
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
        let request = request.into_client_request()?;
        let stream: Box<dyn Duplex> = match &self.socket {
            #[cfg(unix)]
            Some(socket) => Box::new(tokio::net::UnixStream::connect(socket).await?),
            #[cfg(not(unix))]
            Some(_) => unreachable!("only a Cloud's runner, on Linux, has a backend socket"),
            None => {
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
        tokio_tungstenite::client_async_tls_with_config(request, stream, Some(config), None).await
    }
}

/// A backend on the network, as a URL names it.
impl FromStr for Backend {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, ValueError> {
        Ok(Self::at(value.parse()?))
    }
}
