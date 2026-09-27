//! A socket to a page, which the shard serves: the synchronization channel or
//! a conversation socket (`backend.md` § Browser synchronization). Both send
//! their protocol's heartbeat once they have sent nothing else for the
//! heartbeat interval, so that the page can tell a quiet socket from one that
//! died without a close (`web-application.md` § Liveness and reconnection).

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::SinkExt as _;
use futures_util::stream::SplitSink;
use tokio::time::{Instant, Sleep};

use crate::config::PageTuning;

/// The sending half of a socket to a page, which knows when it last sent.
pub(crate) struct PageSocket {
    sink: SplitSink<WebSocket, Message>,
    tuning: PageTuning,
    /// When the socket last sent a message, or was opened.
    sent_at: Instant,
}

/// A send failed: the page closed the socket, or it broke.
#[derive(Debug)]
pub(crate) struct PageGone;

impl PageSocket {
    pub(crate) fn new(sink: SplitSink<WebSocket, Message>, tuning: PageTuning) -> Self {
        Self {
            sink,
            tuning,
            sent_at: Instant::now(),
        }
    }

    /// Sends one JSON text message.
    pub(crate) async fn send(&mut self, text: String) -> Result<(), PageGone> {
        self.sink.send(Message::Text(text.into())).await.map_err(|_| PageGone)?;
        self.sent_at = Instant::now();
        Ok(())
    }

    /// Resolves once the socket has sent nothing for the heartbeat interval,
    /// when its heartbeat is due.
    pub(crate) fn silent(&self) -> Sleep {
        tokio::time::sleep_until(self.sent_at + self.tuning.heartbeat)
    }

    /// Ends the socket with `frame`.
    pub(crate) async fn close(mut self, frame: CloseFrame) {
        // A page that went meanwhile hears nothing, which is what the close
        // tells it.
        let _ = self.sink.send(Message::Close(Some(frame))).await;
    }
}
