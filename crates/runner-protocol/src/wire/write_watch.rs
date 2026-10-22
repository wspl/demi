//! Whether a connection's writes make progress.
//!
//! A frame is one message, so a write that takes long may still be moving
//! bytes: a 4 MiB frame on a slow uplink is. The socket below the WebSocket
//! knows: its write either accepts bytes or waits for room. Each end notes
//! that here, as its socket's writes are polled ([`Watched`] for the runner's,
//! the edge's own connection type for the backend's), and the writer of the
//! link asks for the moment no byte was accepted for [`LINK_WRITE_TIMEOUT`]
//! ([`WriteProgress::stalled`]).

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;

use super::LINK_WRITE_TIMEOUT;

/// Since when a connection's writes wait for room, if they do.
#[derive(Default)]
pub struct WriteProgress {
    waiting_since: Mutex<Option<Instant>>,
    changed: Notify,
}

impl WriteProgress {
    /// Notes the result of a poll to write bytes: waiting for room starts the
    /// count, and any byte accepted ends it.
    pub fn polled<T>(&self, polled: &Poll<io::Result<T>>, accepted: impl Fn(&T) -> bool) {
        let mut since = self
            .waiting_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match polled {
            Poll::Pending => {
                if since.is_none() {
                    *since = Some(Instant::now());
                    self.changed.notify_waiters();
                }
            }
            Poll::Ready(Ok(done)) if accepted(done) => {
                if since.take().is_some() {
                    self.changed.notify_waiters();
                }
            }
            Poll::Ready(_) => {}
        }
    }

    /// Resolves once the writes waited for room, with no byte accepted, for
    /// [`LINK_WRITE_TIMEOUT`]. Never resolves for writes that move or none.
    pub async fn stalled(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let since = *self
                .waiting_since
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match since {
                None => changed.await,
                Some(since) => {
                    let due = tokio::time::Instant::from_std(since + LINK_WRITE_TIMEOUT);
                    tokio::select! {
                        () = tokio::time::sleep_until(due) => {
                            let now = *self
                                .waiting_since
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner);
                            if now == Some(since) {
                                return;
                            }
                        }
                        () = &mut changed => {}
                    }
                }
            }
        }
    }
}

/// A stream whose writes are noted in `progress`.
pub struct Watched<S> {
    stream: S,
    progress: Arc<WriteProgress>,
}

impl<S> Watched<S> {
    pub fn new(stream: S) -> (Self, Arc<WriteProgress>) {
        let progress = Arc::new(WriteProgress::default());
        (
            Self {
                stream,
                progress: progress.clone(),
            },
            progress,
        )
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Watched<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Watched<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write(cx, buf);
        self.progress.polled(&polled, |&written| written > 0);
        polled
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write_vectored(cx, bufs);
        self.progress.polled(&polled, |&written| written > 0);
        polled
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
