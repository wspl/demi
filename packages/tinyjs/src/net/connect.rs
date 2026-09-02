//! From a URL to a connected stream: direct or through the proxy the
//! environment names (`HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY`), with
//! `CONNECT` tunnelling for TLS and WebSocket targets.

use std::io;
use std::sync::Arc;

use hyper::http::Uri;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::proxy::matcher::Matcher;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tower_service::Service;

use super::tls;

pub trait Transport: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Transport for T {}

pub type Conn = Box<dyn Transport>;

pub struct Connected {
    pub stream: Conn,
    /// A plain-HTTP request that goes to a proxy is sent in absolute form
    /// with the proxy's credentials; a tunnelled one is not.
    pub via_proxy: Option<hyper_util::client::proxy::matcher::Intercept>,
}

fn proxy_error<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(format!("proxy tunnel failed: {e}"))
}

/// Opens the transport for `target`: TCP to the host, or through the
/// intercepting proxy. `tunnel` asks for a `CONNECT` tunnel when a proxy is
/// used (TLS and WebSocket targets); otherwise the proxy connection itself
/// is returned and `via_proxy` tells the caller to speak proxy HTTP.
pub async fn open(target: &Uri, tunnel: bool) -> io::Result<Connected> {
    let matcher = Matcher::from_env();
    let mut connector = HttpConnector::new();
    connector.enforce_http(false);
    match matcher.intercept(target) {
        Some(intercept) if tunnel => {
            let mut tunnel = Tunnel::new(intercept.uri().clone(), connector);
            if let Some(auth) = intercept.basic_auth() {
                tunnel = tunnel.with_auth(auth.clone());
            }
            let io: TokioIo<TcpStream> = tunnel.call(target.clone()).await.map_err(proxy_error)?;
            Ok(Connected { stream: Box::new(io.into_inner()), via_proxy: None })
        }
        Some(intercept) => {
            let io = connector.call(intercept.uri().clone()).await.map_err(|e| io::Error::other(e.to_string()))?;
            Ok(Connected { stream: Box::new(io.into_inner()), via_proxy: Some(intercept) })
        }
        None => {
            let io = connector.call(target.clone()).await.map_err(|e| {
                // hyper-util wraps the OS error; surface its errno.
                match std::error::Error::source(&e).and_then(|s| s.downcast_ref::<io::Error>()) {
                    Some(inner) => io::Error::new(inner.kind(), e.to_string()).with_errno(inner.raw_os_error()),
                    None => io::Error::other(e.to_string()),
                }
            })?;
            Ok(Connected { stream: Box::new(io.into_inner()), via_proxy: None })
        }
    }
}

/// Wraps a transport in TLS for `host`.
pub async fn tls_wrap(stream: Conn, host: &str) -> io::Result<Conn> {
    let config = tls::client_config().map_err(io::Error::other)?;
    let name = ServerName::try_from(host.to_string()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid server name '{host}'")))?;
    // rustls reports handshake failures as InvalidData; they are protocol
    // errors to the caller, not argument errors.
    let tls = TlsConnector::from(Arc::clone(&config))
        .connect(name, stream)
        .await
        .map_err(|e| if e.raw_os_error().is_some() { e } else { io::Error::other(format!("TLS: {e}")) })?;
    Ok(Box::new(tls))
}

pub trait WithErrno {
    fn with_errno(self, errno: Option<i32>) -> io::Error;
}

impl WithErrno for io::Error {
    fn with_errno(self, errno: Option<i32>) -> io::Error {
        match errno {
            Some(n) => io::Error::from_raw_os_error(n),
            None => self,
        }
    }
}
