//! A socket to a page: the synchronization channel and the conversation
//! sockets, which the shard serves (`backend.md` § Page synchronization), and
//! the file watch, which the edge relays (`web-api.md` § File watch). Each
//! sends its protocol's heartbeat once it has sent nothing else for the
//! heartbeat interval, so that the page can tell a quiet socket from one that
//! died without a close (`web-application.md` § Liveness and reconnection),
//! and each ends with a close frame that a page which stopped reading cannot
//! hold up (`backend.md` § Startup and shutdown).

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::SinkExt as _;
use futures_util::stream::SplitSink;
use tokio::time::{Instant, Sleep};

use crate::tuning::PageTuning;

/// The sending half of a socket to a page, which knows when it last sent.
pub struct PageSocket {
    sink: SplitSink<WebSocket, Message>,
    tuning: PageTuning,
    /// When the socket last sent a message, or was opened.
    sent_at: Instant,
}

/// A send failed: the page closed the socket, or it broke.
#[derive(Debug)]
pub struct PageGone;

impl PageSocket {
    pub fn new(sink: SplitSink<WebSocket, Message>, tuning: PageTuning) -> Self {
        Self {
            sink,
            tuning,
            sent_at: Instant::now(),
        }
    }

    /// Sends one JSON text message.
    pub async fn send(&mut self, text: String) -> Result<(), PageGone> {
        self.sink
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| PageGone)?;
        self.sent_at = Instant::now();
        Ok(())
    }

    /// Resolves once the socket has sent nothing for the heartbeat interval,
    /// when its heartbeat is due.
    pub fn silent(&self) -> Sleep {
        tokio::time::sleep_until(self.sent_at + self.tuning.heartbeat)
    }

    /// Ends the socket with `frame`, which waits for the page at most the
    /// close's bound: behind it may be the rest of a message the socket was
    /// sending when its owner stopped, which a page that stopped reading
    /// never takes.
    pub async fn close(mut self, frame: CloseFrame) {
        // A page that went meanwhile, or does not read, hears nothing, and
        // dropping the socket ends the connection all the same.
        let _ = tokio::time::timeout(
            self.tuning.close_wait,
            self.sink.send(Message::Close(Some(frame))),
        )
        .await;
    }
}
