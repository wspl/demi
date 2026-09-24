//! Leases (`concurrency.md` § The user shard): a host operation the shard
//! admitted that outlives the call, such as a file transfer or a user
//! stream, whose bytes the edge moves. The edge holds the lease while bytes
//! move; dropping it tells the shard the operation is over, which releases
//! its admission. The shard can end the operation first, for example when
//! the conversation is archived: the lease's `ended` then resolves, and the
//! edge stops.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::oneshot;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

/// The edge's hold on an admitted operation. `Send`.
pub(crate) struct Lease {
    ended: CancellationToken,
    /// Dropped with the lease, which the shard's side sees.
    _held: oneshot::Sender<()>,
}

/// The shard's side of a lease: resolves once the edge dropped it.
pub(crate) struct Released(oneshot::Receiver<()>);

impl Lease {
    /// A lease that `ended` ends from the shard's side.
    pub(crate) fn new(ended: CancellationToken) -> (Self, Released) {
        let (held, released) = oneshot::channel();
        (Self { ended, _held: held }, Released(released))
    }

    /// Resolves once the shard ended the operation.
    pub(crate) fn ended(&self) -> WaitForCancellationFuture<'_> {
        self.ended.cancelled()
    }

    /// The token that ends the operation, for a watch at the edge that
    /// outlives this borrow.
    pub(crate) fn ending(&self) -> CancellationToken {
        self.ended.clone()
    }
}

impl Future for Released {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // The lease never sends; its end is the sender's drop.
        Pin::new(&mut self.get_mut().0).poll(cx).map(|_| ())
    }
}
