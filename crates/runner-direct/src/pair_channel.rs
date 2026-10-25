//! A channel of a pair's peer (`demi_runner_protocol::pairs`), as the runner
//! uses it: a forwarded connection's (`crate::forward`) or a pipe's bytes
//! between two moves (`crates/runner-process/src/pipes/moves.rs`).
//!
//! Its bytes go in binary messages, its end as `{ end: true }`, and the side
//! that takes them grants back what its consumer took
//! (`crate::channel`), which the sending side can watch.

use bytes::Bytes;
use demi_runner_protocol::direct::{MESSAGE_BYTES, WriteEnd};
use str0m::channel::ChannelId;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::operation::{Input, Output};

/// A channel of a pair's peer, as a byte stream both ways.
pub struct PairChannel {
    pub(crate) input: mpsc::UnboundedReceiver<Input>,
    pub(crate) output: mpsc::Sender<Output>,
    pub(crate) took: Took,
    /// Canceled once the channel goes.
    pub(crate) cancel: CancellationToken,
    /// Closes the channel at once, canceled when its work ends before both
    /// its sides ended cleanly.
    pub(crate) abort: CancellationToken,
    /// Every byte the other side granted, on a channel this side opened.
    pub(crate) granted: Option<watch::Receiver<u64>>,
}

/// Where a consumer says how many bytes it took, which its channel grants
/// back to the other side.
#[derive(Clone)]
pub(crate) struct Took {
    pub(crate) id: ChannelId,
    pub(crate) sender: mpsc::UnboundedSender<(ChannelId, usize)>,
}

impl Took {
    pub(crate) fn report(&self, bytes: usize) {
        // A peer that ended grants nothing more.
        let _ = self.sender.send((self.id, bytes));
    }
}

/// What the other side sent on a channel.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    Bytes(Bytes),
    /// It sent everything.
    End,
}

/// The channel went: its peer ended, or the other side closed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the direct channel closed")]
pub struct ChannelGone;

impl PairChannel {
    /// Sends `bytes`, in messages the channel takes as its window and queue
    /// let it.
    pub async fn send(&self, bytes: Bytes) -> Result<(), ChannelGone> {
        let mut start = 0;
        while start < bytes.len() {
            let end = (start + MESSAGE_BYTES).min(bytes.len());
            self.output
                .send(Output::Binary(bytes.slice(start..end)))
                .await
                .map_err(|_| ChannelGone)?;
            start = end;
        }
        Ok(())
    }

    /// Says that this side sent everything.
    pub async fn end(&self) -> Result<(), ChannelGone> {
        let end = serde_json::to_string(&WriteEnd { end: true }).expect("an end serializes");
        self.output.send(Output::Text(end)).await.map_err(|_| ChannelGone)
    }

    /// The bytes the other side took, from the channel's first, as they
    /// grow; always none on a channel the other side opened.
    pub fn granted(&self) -> watch::Receiver<u64> {
        self.granted
            .clone()
            .unwrap_or_else(|| watch::channel(0).1)
    }

    /// The other side's next bytes, or its end; none once the channel went.
    pub async fn receive(&mut self) -> Option<Received> {
        loop {
            match self.input.recv().await? {
                Input::Bytes(bytes) => return Some(Received::Bytes(bytes)),
                Input::End => return Some(Received::End),
                Input::Watch(_) | Input::Watched(_) => {}
            }
        }
    }

    /// This side's consumer took `bytes` of what the other side sent, which
    /// the channel grants back.
    pub fn took(&self, bytes: usize) {
        self.took.report(bytes);
    }

    /// Resolves once the channel went.
    pub async fn gone(&self) {
        self.cancel.cancelled().await;
    }

    /// Closes the channel at once, whatever it still holds.
    pub fn close(self) {
        self.abort.cancel();
    }
}
