//! The edge's own listener and connections: shutdown can stop accepting
//! connections while the open ones go on serving, and close those later,
//! which axum's own graceful shutdown, closing connections as it stops
//! accepting, cannot. Each connection also has a control a request can
//! close it through, and an idle deadline a file transfer arms
//! (`sessions-and-targets.md` § Host operations): a connection on which no
//! byte moves for the deadline is closed, whoever stopped moving them. The
//! edge serves its connections itself, with hyper's HTTP/1 server keeping
//! each header name's case, which axum's `serve` cannot: the expose relay
//! passes names on as the visitor wrote them (`expose.md` § The public
//! relay).

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::http::Request;
use axum::response::Response;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
use tokio_util::task::{AbortOnDropHandle, TaskTracker};

pub(super) struct EdgeListener {
    /// Dropped, which closes the socket, once shutdown starts.
    tcp: Option<TcpListener>,
    closing: CancellationToken,
    connections: CancellationToken,
}

impl EdgeListener {
    pub(super) fn new(tcp: TcpListener, closing: CancellationToken, connections: CancellationToken) -> Self {
        Self {
            tcp: Some(tcp),
            closing,
            connections,
        }
    }

    /// The next connection, until shutdown starts; then the socket closes
    /// and this never resolves.
    async fn accept(&mut self) -> (ConnectionIo, SocketAddr) {
        if let Some(tcp) = self.tcp.as_mut() {
            tokio::select! {
                biased;
                () = self.closing.cancelled() => {}
                // axum's own accept, which waits out and logs transient errors.
                (stream, peer) = axum::serve::Listener::accept(tcp) => {
                    return (ConnectionIo::new(stream, &self.connections), peer);
                }
            }
            self.tcp = None;
        }
        std::future::pending().await
    }
}

/// Serves each connection `listener` accepts, answering each request with
/// `respond`, until `stop` is cancelled: then no connection is accepted,
/// the open ones finish the requests they serve, and this resolves once
/// they closed. A connection's upgraded stream, such as a WebSocket, lives
/// on without it.
pub(super) async fn serve<R, F>(mut listener: EdgeListener, stop: CancellationToken, respond: R)
where
    R: Fn(Peer, Request<Incoming>) -> F + Clone + Send + 'static,
    F: Future<Output = Response> + Send + 'static,
{
    let connections = TaskTracker::new();
    loop {
        let (io, addr) = tokio::select! {
            () = stop.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let peer = Peer {
            addr,
            control: io.control.clone(),
        };
        let respond = respond.clone();
        let service = service_fn(move |request| {
            let answer = respond(peer.clone(), request);
            async move { Ok::<_, Infallible>(answer.await) }
        });
        connections.spawn(serve_connection(io, service, stop.clone()));
    }
    drop(listener);
    connections.close();
    connections.wait().await;
}

/// Serves one connection until it closes; once `stop` is cancelled it
/// closes after the request it serves.
async fn serve_connection<S>(io: ConnectionIo, service: S, stop: CancellationToken)
where
    S: hyper::service::Service<Request<Incoming>, Response = Response, Error = Infallible>,
{
    let mut builder = http1::Builder::new();
    builder.preserve_header_case(true);
    let connection = builder.serve_connection(TokioIo::new(io), service).with_upgrades();
    let mut connection = std::pin::pin!(connection);
    let mut stopping = false;
    loop {
        tokio::select! {
            served = connection.as_mut() => {
                // A peer that went away or broke the protocol ends its own
                // connection; nobody waits to hear why.
                if let Err(error) = served {
                    tracing::trace!(error = &error as &dyn std::error::Error, "a connection ended");
                }
                return;
            }
            () = stop.cancelled(), if !stopping => {
                stopping = true;
                connection.as_mut().graceful_shutdown();
            }
        }
    }
}

/// An accepted connection, whose IO fails once the edge closes its
/// connections or its control closes it: hyper then drops it, even when it
/// is blocked writing to a peer that stopped reading.
pub(super) struct ConnectionIo {
    stream: TcpStream,
    /// Read and write each wait on their own, so a connection whose halves
    /// two tasks drive wakes both.
    read_closed: Pin<Box<WaitForCancellationFutureOwned>>,
    write_closed: Pin<Box<WaitForCancellationFutureOwned>>,
    control: ConnectionControl,
}

impl ConnectionIo {
    fn new(stream: TcpStream, connections: &CancellationToken) -> Self {
        let closed = connections.child_token();
        Self {
            stream,
            read_closed: Box::pin(closed.clone().cancelled_owned()),
            write_closed: Box::pin(closed.clone().cancelled_owned()),
            control: ConnectionControl(Arc::new(ControlState {
                closed,
                opened: Instant::now(),
                moved: AtomicU64::new(0),
            })),
        }
    }

    /// Notes that bytes moved when `polled` moved some.
    fn moved<T>(&self, polled: &Poll<io::Result<T>>) {
        if matches!(polled, Poll::Ready(Ok(_))) {
            self.control.0.touch();
        }
    }
}

/// A connection as a request sees it: where it comes from, and its control.
#[derive(Clone)]
pub(crate) struct Peer {
    pub(crate) addr: SocketAddr,
    pub(crate) control: ConnectionControl,
}

/// Closes a connection from the request it serves: at once, or once no
/// byte moved on it for a while. Cloning it is cheap.
#[derive(Clone)]
pub(crate) struct ConnectionControl(Arc<ControlState>);

struct ControlState {
    /// Cancelled, the connection's IO fails.
    closed: CancellationToken,
    opened: Instant,
    /// When bytes last moved, in milliseconds after `opened`.
    moved: AtomicU64,
}

impl ControlState {
    fn touch(&self) {
        let elapsed = u64::try_from(self.opened.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.moved.store(elapsed, Ordering::Relaxed);
    }

    /// When bytes last moved.
    fn last_moved(&self) -> Instant {
        self.opened + Duration::from_millis(self.moved.load(Ordering::Relaxed))
    }
}

impl ConnectionControl {
    /// A control of no connection, whose closing a test observes.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self(Arc::new(ControlState {
            closed: CancellationToken::new(),
            opened: Instant::now(),
            moved: AtomicU64::new(0),
        }))
    }

    /// As if bytes moved on the connection just now.
    #[cfg(test)]
    pub(crate) fn touched(&self) {
        self.0.touch();
    }

    #[cfg(test)]
    pub(crate) fn is_closed(&self) -> bool {
        self.0.closed.is_cancelled()
    }

    /// Closes the connection now.
    pub(crate) fn close(&self) {
        self.0.closed.cancel();
    }

    /// Closes the connection once no byte moved on it for `limit`, counted
    /// from now, or once `ended` is cancelled, until the answer is dropped.
    pub(crate) fn watch(&self, limit: Duration, ended: CancellationToken) -> ConnectionWatch {
        self.0.touch();
        let state = self.0.clone();
        let watching = tokio::spawn(async move {
            loop {
                let due = state.last_moved() + limit;
                tokio::select! {
                    () = ended.cancelled() => break,
                    () = state.closed.cancelled() => return,
                    () = tokio::time::sleep_until(due) => {
                        // Bytes moved while this slept: the deadline moved.
                        if state.last_moved() + limit <= Instant::now() {
                            break;
                        }
                    }
                }
            }
            state.closed.cancel();
        });
        ConnectionWatch(AbortOnDropHandle::new(watching))
    }
}

/// A connection's watch; dropping it stops watching.
pub(crate) struct ConnectionWatch(#[expect(dead_code, reason = "held for its drop")] AbortOnDropHandle<()>);

fn closed_by_the_backend() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "the backend closed the connection")
}

impl AsyncRead for ConnectionIo {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.read_closed.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed_by_the_backend()));
        }
        let before = buf.filled().len();
        let polled = Pin::new(&mut this.stream).poll_read(cx, buf);
        if buf.filled().len() > before {
            this.moved(&polled);
        }
        polled
    }
}

impl AsyncWrite for ConnectionIo {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_closed.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed_by_the_backend()));
        }
        let polled = Pin::new(&mut this.stream).poll_write(cx, buf);
        this.moved(&polled);
        polled
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_closed.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed_by_the_backend()));
        }
        let polled = Pin::new(&mut this.stream).poll_write_vectored(cx, bufs);
        this.moved(&polled);
        polled
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.write_closed.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed_by_the_backend()));
        }
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
