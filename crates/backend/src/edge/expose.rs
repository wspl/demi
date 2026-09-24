//! The public relay (`expose.md` § The public relay): every request whose
//! `Host` is an expose hostname, whatever its path and method, before the
//! routes see it. The owner's shard admits the connection and hands over a
//! network stream to the service on the device; the relay forwards the
//! visitor's request over it with hyper's HTTP/1 client, which keeps each
//! header name's case, and relays the answer as it arrives, or, after a
//! WebSocket upgrade, copies the bytes both ways without reading them.

use std::future::Future;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::http::header::{CONNECTION, CONTENT_TYPE, HOST, UPGRADE};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode, Uri, Version};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use demi_web_api::exposes::ExposeAddress;
use demi_web_api::ids::ExposeId;
use http_body::{Frame, SizeHint};
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use tokio_util::io::{CopyToBytes, SinkWriter, StreamReader};

use super::AppState;
use super::cookies::over_https;
use super::listener::{ConnectionWatch, Peer};
use crate::expose::{ExposeConnection, RelayRefusal};
use crate::shard::lease::Lease;

/// Headers that concern one connection only, which the relay never passes
/// on; the names a `Connection` header lists join them.
const HOP_BY_HOP: [HeaderName; 9] = [
    CONNECTION,
    HeaderName::from_static("keep-alive"),
    HeaderName::from_static("proxy-connection"),
    HeaderName::from_static("proxy-authenticate"),
    HeaderName::from_static("proxy-authorization"),
    HeaderName::from_static("te"),
    HeaderName::from_static("trailer"),
    HeaderName::from_static("transfer-encoding"),
    UPGRADE,
];

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

/// The client connection one exchange runs on, which resolves once the
/// exchange is over or its stream went to an upgrade. hyper does not name
/// the type of a client connection with upgrades.
type ClientConnection = Pin<Box<dyn Future<Output = hyper::Result<()>> + Send>>;

/// The label before the instance's expose domain when `request` is for an
/// expose hostname, which then belongs to the relay; `None` for every other
/// request, and for every request when the instance has no expose domain.
pub(super) fn expose_label(state: &AppState, request: &Request<Incoming>) -> Option<String> {
    let domain = state.services.expose_domain.as_ref()?;
    let host = request.headers().get(HOST)?.to_str().ok()?;
    let authority = host.parse::<Authority>().ok()?;
    domain.label(authority.host())
}

/// Answers a visitor of the expose whose hostname starts with `label`:
/// unknown, malformed and expired ids answer 404, and a connection the
/// owner's shard does not admit answers its reason.
pub(super) async fn relay(state: AppState, peer: Peer, label: String, request: Request<Incoming>) -> Response {
    let Ok(id) = ExposeId::try_from(label) else {
        return not_found();
    };
    let record = match state.services.control.expose(id.clone()).await {
        Ok(Some(record)) => record,
        Ok(None) => return not_found(),
        Err(error) => {
            tracing::error!(error = &error as &dyn std::error::Error, "an expose could not be read");
            return unavailable();
        }
    };
    let admitted = state
        .shards
        .of(&record.user)
        .call(move |shard, cancel| async move { shard.open_expose_connection(&id, &cancel).await })
        .await;
    match admitted {
        Ok(Ok(connection)) => {
            let idle = state.services.expose_tuning.idle;
            forward(connection, &record.address, &peer, idle, request).await
        }
        Ok(Err(refusal)) => refused(refusal),
        // The backend is shutting down.
        Err(_) => unavailable(),
    }
}

/// Forwards `request` over the admitted connection and relays the answer.
/// The visitor's connection closes once no byte moved on it for `idle`, and
/// at once when the expose ends.
async fn forward(
    connection: ExposeConnection,
    address: &ExposeAddress,
    peer: &Peer,
    idle: Duration,
    mut request: Request<Incoming>,
) -> Response {
    let ExposeConnection {
        to_service,
        from_service,
        lease,
    } = connection;
    let watch = peer.control.watch(idle, lease.ending());
    // The network stream as one byte stream: what the socket sends, read
    // from the output pipe, and the visitor's bytes, written into the input
    // pipe, whose close is the socket's half-close.
    let io = tokio::io::join(
        StreamReader::new(from_service.into_stream()),
        SinkWriter::new(CopyToBytes::new(to_service.into_sink())),
    );
    let (mut sender, connection) = match http1::Builder::new()
        .preserve_header_case(true)
        .handshake(TokioIo::new(io))
        .await
    {
        Ok(handshake) => handshake,
        Err(error) => {
            tracing::debug!(error = &error as &dyn std::error::Error, "the relay's client did not start");
            return unreachable("no_response");
        }
    };
    let mut connection: Option<ClientConnection> = Some(Box::pin(connection.with_upgrades()));
    let upgrade = wants_upgrade(request.headers());
    let visitor_upgrade = hyper::upgrade::on(&mut request);
    let mut exchange = pin!(sender.send_request(forwarded_request(request, address, peer, upgrade)));
    // The client connection writes the request and reads the answer while
    // the exchange waits for the answer's head.
    let answer = loop {
        tokio::select! {
            biased;
            () = lease.ended() => return removed(),
            answer = &mut exchange => break answer,
            // Its end or failure reaches the exchange too.
            _ = running(&mut connection) => connection = None,
        }
    };
    let mut answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            tracing::debug!(error = &error as &dyn std::error::Error, "the exposed service gave no answer");
            return unreachable("no_response");
        }
    };
    if upgrade && answer.status() == StatusCode::SWITCHING_PROTOCOLS {
        let service_upgrade = hyper::upgrade::on(&mut answer);
        let (mut parts, _) = answer.into_parts();
        parts.version = Version::HTTP_11;
        remove_hop_by_hop(&mut parts.headers, true);
        tokio::spawn(copy_upgraded(visitor_upgrade, service_upgrade, connection, lease, watch));
        return Response::from_parts(parts, Body::empty());
    }
    let (mut parts, body) = answer.into_parts();
    parts.version = Version::HTTP_11;
    remove_hop_by_hop(&mut parts.headers, false);
    let body = RelayedBody {
        body,
        connection,
        _lease: lease,
        _watch: watch,
    };
    Response::from_parts(parts, Body::new(body))
}

/// Resolves when the client connection ends, and never when there is none.
async fn running(connection: &mut Option<ClientConnection>) -> hyper::Result<()> {
    match connection {
        Some(connection) => connection.await,
        None => std::future::pending().await,
    }
}

/// The request as the service receives it (`expose.md` § The public
/// relay): its target in origin form, `Host` rewritten to the expose's
/// address, the visitor's address appended to `X-Forwarded-For`, and
/// `X-Forwarded-Host` and `X-Forwarded-Proto` set to the host and the scheme
/// the visitor used. Connection-level headers go, except those an upgrade's
/// handshake needs; any other request asks the service to close its
/// connection after the answer. Every other header, its name's case and
/// the separate lines of a repeated one pass unchanged.
fn forwarded_request(request: Request<Incoming>, address: &ExposeAddress, peer: &Peer, upgrade: bool) -> Request<Incoming> {
    let https = over_https(request.uri(), request.headers());
    let (mut parts, body) = request.into_parts();
    parts.uri = parts
        .uri
        .path_and_query()
        .map_or_else(|| Uri::from_static("/"), |target| Uri::from(target.clone()));
    parts.version = Version::HTTP_11;
    let headers = &mut parts.headers;
    let visitor_host = headers.get(HOST).cloned();
    remove_hop_by_hop(headers, upgrade);
    if !upgrade {
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
    }
    let address = HeaderValue::try_from(address.as_str()).expect("an expose address is visible ASCII");
    headers.insert(HOST, address);
    let visitor = HeaderValue::try_from(peer.addr.ip().to_string()).expect("an IP address is visible ASCII");
    headers.append(X_FORWARDED_FOR, visitor);
    if let Some(host) = visitor_host {
        headers.insert(X_FORWARDED_HOST, host);
    }
    let scheme = HeaderValue::from_static(if https { "https" } else { "http" });
    headers.insert(X_FORWARDED_PROTO, scheme);
    Request::from_parts(parts, body)
}

/// The tokens of the `Connection` headers, each trimmed.
fn connection_tokens(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
}

/// Whether the visitor asks to switch protocols, as a WebSocket does.
fn wants_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(UPGRADE) && connection_tokens(headers).any(|token| token.eq_ignore_ascii_case("upgrade"))
}

/// Removes the connection-level headers; an upgrade keeps `Connection` and
/// `Upgrade`, which its handshake needs.
fn remove_hop_by_hop(headers: &mut HeaderMap, upgrade: bool) {
    let listed: Vec<HeaderName> = connection_tokens(headers)
        .filter_map(|token| HeaderName::try_from(token).ok())
        .collect();
    for name in HOP_BY_HOP.iter().chain(&listed) {
        if upgrade && (*name == CONNECTION || *name == UPGRADE) {
            continue;
        }
        headers.remove(name);
    }
}

/// After the service switched protocols: once the client connection handed
/// its stream to the upgrade, which it may have done already, and the
/// visitor's switch went out, copies bytes both ways, unread, until both
/// sides ended or the expose ends.
async fn copy_upgraded(
    visitor: OnUpgrade,
    service: OnUpgrade,
    connection: Option<ClientConnection>,
    lease: Lease,
    watch: ConnectionWatch,
) {
    let copied = async {
        if let Some(connection) = connection {
            connection.await?;
        }
        let (visitor, service) = tokio::try_join!(visitor, service)?;
        tokio::io::copy_bidirectional(&mut TokioIo::new(visitor), &mut TokioIo::new(service)).await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    };
    tokio::select! {
        () = lease.ended() => {}
        copied = copied => {
            // A side that went away ends the copy; nobody waits to hear why.
            if let Err(error) = copied {
                tracing::debug!(%error, "an upgraded relay connection ended");
            }
        }
    }
    drop(watch);
}

/// A relayed answer's body: the service's, as it arrives, while the
/// exchange's client connection runs beside it, writing what is left of the
/// request and ending the stream's input once the answer is complete. It
/// holds the lease and the idle watch until the answer ends or the visitor
/// leaves.
struct RelayedBody {
    body: Incoming,
    connection: Option<ClientConnection>,
    _lease: Lease,
    _watch: ConnectionWatch,
}

impl http_body::Body for RelayedBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let this = self.get_mut();
        if let Some(connection) = &mut this.connection
            && connection.as_mut().poll(cx).is_ready()
        {
            // Its end or failure reaches the body too.
            this.connection = None;
        }
        Pin::new(&mut this.body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

/// Why the relay did not reach the service, as a plain page.
fn refused(refusal: RelayRefusal) -> Response {
    match refusal {
        RelayRefusal::NotFound => not_found(),
        RelayRefusal::DeviceOffline => unreachable("device_offline"),
        RelayRefusal::Unreachable { code } => unreachable(&code),
        RelayRefusal::Limit => page(
            StatusCode::SERVICE_UNAVAILABLE,
            "This expose is at its concurrent-connection limit; retry shortly.\n",
        ),
        RelayRefusal::Removed => removed(),
        RelayRefusal::Storage(error) => {
            tracing::error!(error = &error as &dyn std::error::Error, "an expose could not be read");
            unavailable()
        }
    }
}

fn not_found() -> Response {
    page(
        StatusCode::NOT_FOUND,
        "This expose does not exist (anymore); its URL is gone or has expired.\n",
    )
}

fn unreachable(reason: &str) -> Response {
    page(
        StatusCode::BAD_GATEWAY,
        &format!("The exposed service is unreachable ({reason}).\n"),
    )
}

fn removed() -> Response {
    page(StatusCode::BAD_GATEWAY, "This expose was removed while serving.\n")
}

fn unavailable() -> Response {
    page(StatusCode::SERVICE_UNAVAILABLE, "This expose cannot be served right now; retry shortly.\n")
}

fn page(status: StatusCode, text: &str) -> Response {
    (status, [(CONTENT_TYPE, "text/plain; charset=utf-8")], text.to_owned()).into_response()
}
