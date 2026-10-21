//! Where a message is sent: a lane of the link, which holds only the senders
//! of its own stream when its peer does not read it.

use std::sync::Arc;

use demi_runner_protocol::wire::Frame;
use tokio::sync::{
    mpsc::{self, error::{SendError, TrySendError}},
    watch,
};

use crate::connection::Owner;

/// Sends frames to the other end of a link. Cloning it is cheap.
///
/// A frame goes on the lane its route names. A sender waits for room in that
/// lane's queue, and a lane whose stream is full waits for its peer's window,
/// so what a peer does not read of one lane holds the senders of that lane and
/// no other.
#[derive(Clone)]
pub struct Outgoing(Kind);

#[derive(Clone)]
enum Kind {
    Link(Arc<Owner>),
    /// One queue for every frame, for a test of a link's user.
    Plain(mpsc::Sender<Frame>),
    /// Whichever outgoing the watch holds now; frames sent while it holds
    /// none go nowhere.
    Switch(watch::Receiver<Option<Outgoing>>),
}

impl Outgoing {
    pub(crate) fn link(owner: Arc<Owner>) -> Self {
        Self(Kind::Link(owner))
    }

    /// An outgoing that queues every frame in one channel, whatever its
    /// lane, for a test of a link's user.
    pub fn plain(capacity: usize) -> (Self, mpsc::Receiver<Frame>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (Self(Kind::Plain(sender)), receiver)
    }

    /// An outgoing that follows `current`: what it sends goes to the
    /// outgoing the watch holds now, and nowhere while it holds none, as the
    /// messages of jobs that outlive a connection go to whichever one serves.
    pub fn following(current: watch::Receiver<Option<Outgoing>>) -> Self {
        Self(Kind::Switch(current))
    }

    /// Sends `frame` once its lane has room; the frame comes back when the
    /// link is over.
    pub async fn send(&self, frame: Frame) -> Result<(), SendError<Frame>> {
        match &self.0 {
            Kind::Link(shared) => shared.send(frame).await.map_err(SendError),
            Kind::Plain(sender) => sender.send(frame).await,
            Kind::Switch(current) => {
                let outgoing = current.borrow().clone();
                match outgoing {
                    // A connection that ended takes nothing more, and the
                    // next one reads what it needs from the jobs.
                    Some(outgoing) => {
                        let _ = Box::pin(outgoing.send(frame)).await;
                        Ok(())
                    }
                    None => Ok(()),
                }
            }
        }
    }

    /// Queues `frame` without waiting, for a caller that cannot wait, such as
    /// a drop: behind what its lane already holds, whether or not the lane
    /// has room. A full lane never ends the connection, and what is posted
    /// keeps its order among what the lane carries.
    pub fn post(&self, frame: Frame) {
        match &self.0 {
            Kind::Link(shared) => shared.post(frame),
            Kind::Plain(sender) => match sender.try_send(frame) {
                Ok(()) | Err(TrySendError::Closed(_)) => {}
                Err(TrySendError::Full(frame)) => {
                    let sender = sender.clone();
                    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                        runtime.spawn(async move {
                            let _ = sender.send(frame).await;
                        });
                    }
                }
            },
            Kind::Switch(current) => {
                let outgoing = current.borrow().clone();
                if let Some(outgoing) = outgoing {
                    outgoing.post(frame);
                }
            }
        }
    }

    /// Sends `frame` if its lane has room now.
    pub fn try_send(&self, frame: Frame) -> Result<(), TrySendError<Frame>> {
        match &self.0 {
            Kind::Link(shared) => shared.try_send(frame).map_err(|(frame, closed)| {
                if closed {
                    TrySendError::Closed(frame)
                } else {
                    TrySendError::Full(frame)
                }
            }),
            Kind::Plain(sender) => sender.try_send(frame),
            Kind::Switch(current) => {
                let outgoing = current.borrow().clone();
                match outgoing {
                    Some(outgoing) => match outgoing.try_send(frame) {
                        Err(TrySendError::Full(frame)) => Err(TrySendError::Full(frame)),
                        _ => Ok(()),
                    },
                    None => Ok(()),
                }
            }
        }
    }

    /// Whether the link is over: nothing sent reaches the other end.
    pub fn is_closed(&self) -> bool {
        match &self.0 {
            Kind::Link(shared) => shared.is_closed(),
            Kind::Plain(sender) => sender.is_closed(),
            Kind::Switch(current) => current
                .borrow()
                .as_ref()
                .is_some_and(Outgoing::is_closed),
        }
    }

    /// Ends the link: what is queued goes, then a close frame, which names
    /// the message this end refuses when it gives a reason, and the peer's
    /// close is awaited for a few seconds.
    pub async fn close(&self, refusal: Option<String>) {
        match &self.0 {
            Kind::Link(shared) => shared.close(refusal).await,
            _ => {}
        }
    }

    /// Opens a stream on the link that starts with `header`, bypassing the
    /// link's rules, for a test of a peer that breaks them.
    #[cfg(feature = "testing")]
    pub async fn rogue_stream(
        &self,
        header: &[u8],
    ) -> std::io::Result<Box<dyn tokio::io::AsyncWrite + Unpin + Send>> {
        match &self.0 {
            Kind::Link(shared) => Ok(Box::new(shared.rogue(header).await?)),
            _ => Err(std::io::ErrorKind::Unsupported.into()),
        }
    }

    /// Resolves once the link is over.
    pub async fn closed(&self) {
        match &self.0 {
            Kind::Link(shared) => shared.closed.cancelled().await,
            Kind::Plain(sender) => sender.closed().await,
            Kind::Switch(_) => std::future::pending().await,
        }
    }
}
