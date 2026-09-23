//! The edge's own listener: shutdown can stop accepting connections while
//! the open ones go on serving, and close those later, which axum's own
//! graceful shutdown, closing connections as it stops accepting, cannot.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

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
/// connections: hyper then drops it, even when it is blocked writing to a
/// peer that stopped reading.
pub(super) struct ConnectionIo {
    stream: TcpStream,
    /// Read and write each wait on their own, so a connection whose halves
    /// two tasks drive wakes both.
    read_closed: Pin<Box<WaitForCancellationFutureOwned>>,
    write_closed: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl ConnectionIo {
    fn new(stream: TcpStream, connections: &CancellationToken) -> Self {
        let closed = connections.child_token();
        Self {
            stream,
            read_closed: Box::pin(closed.clone().cancelled_owned()),
            write_closed: Box::pin(closed.cancelled_owned()),
        }
    }
}

fn closed_by_the_backend() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "the backend closed the connection")
}

impl AsyncRead for ConnectionIo {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.read_closed.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed_by_the_backend()));
        }
        Pin::new(&mut this.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for ConnectionIo {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_closed.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed_by_the_backend()));
        }
        Pin::new(&mut this.stream).poll_write(cx, buf)
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
        Pin::new(&mut this.stream).poll_write_vectored(cx, bufs)
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
