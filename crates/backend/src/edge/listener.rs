//! The edge's own listener: shutdown can stop accepting connections while
//! the open ones go on serving, and close those later, which axum's own
//! graceful shutdown, closing connections as it stops accepting, cannot.
//! Each connection also has a control a request can close it through, and
//! an idle deadline a file transfer arms (`sessions-and-targets.md` § Host
//! operations): a connection on which no byte moves for the deadline is
//! closed, whoever stopped moving them.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
use tokio_util::task::AbortOnDropHandle;

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
}

impl axum::serve::Listener for EdgeListener {
    type Io = ConnectionIo;
    type Addr = SocketAddr;

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

    fn local_addr(&self) -> io::Result<SocketAddr> {
        match &self.tcp {
            Some(tcp) => tcp.local_addr(),
            None => Err(io::Error::new(io::ErrorKind::NotConnected, "the listener is closed")),
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
    #[expect(dead_code, reason = "the expose relay forwards the visitor's address")]
    pub(crate) addr: SocketAddr,
    pub(crate) control: ConnectionControl,
}

impl Connected<IncomingStream<'_, EdgeListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, EdgeListener>) -> Self {
        Self {
            addr: *stream.remote_addr(),
            control: stream.io().control.clone(),
        }
    }
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
