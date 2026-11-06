//! Each HTTP/2 stream of the process as its request's connection
//! (`crates/core-api/src/peer.rs`): closing the connection ends the stream's
//! bodies, not the program's connection that every other stream shares, and
//! bytes move on it as its bodies move them.

use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::Response;
use http_body::Frame;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::peer::{ConnectionControl, Peer};

/// Gives `request` a connection of its own stream, which its bodies follow.
pub(crate) async fn stream_peer(request: Request, next: Next) -> Response {
    let closed = CancellationToken::new();
    let control = ConnectionControl::new(closed.clone());
    let mut request = request.map(|body| Body::new(Noted::new(body, &control, &closed)));
    request.extensions_mut().insert(ConnectInfo(Peer {
        control: control.clone(),
        address: None,
    }));
    let response = next.run(request).await;
    response.map(|body| Body::new(Noted::new(body, &control, &closed)))
}

/// A body whose data notes that bytes moved on its stream, and which fails
/// once its stream's connection closes.
struct Noted {
    inner: Body,
    control: ConnectionControl,
    closed: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl Noted {
    fn new(inner: Body, control: &ConnectionControl, closed: &CancellationToken) -> Self {
        Self {
            inner,
            control: control.clone(),
            closed: Box::pin(closed.clone().cancelled_owned()),
        }
    }
}

impl http_body::Body for Noted {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        if self.closed.as_mut().poll(cx).is_ready() {
            let closed = std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "the stream's connection closed");
            return Poll::Ready(Some(Err(axum::Error::new(closed))));
        }
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        if let Poll::Ready(Some(Ok(frame))) = &polled
            && frame.is_data()
        {
            self.control.moved();
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}
