//! The callers waiting for work that needs an artifact's location, such as a
//! shared download (`cache`) or a resident's start (`registry`), in the order
//! they joined, each with the resolver that locates the artifact for its own
//! work. The work runs for none of them in particular: it asks the first
//! caller still waiting, and a caller that leaves, even while its resolver
//! answers, hands the question to the next. The caller that started the work
//! may be gone by the time it, or an expired URL's retry, asks.

use std::sync::{Arc, Mutex, MutexGuard};

use demi_command_protocol::PackageArtifact;
use futures_util::future::BoxFuture;
use tokio_util::sync::CancellationToken;

use crate::{ArtifactResolver, ArtifactSource, RuntimeError};

/// The callers waiting for one piece of work, which is the resolver it asks.
#[derive(Clone, Default)]
pub(crate) struct Askers(Arc<Mutex<Callers>>);

#[derive(Default)]
struct Callers {
    next: u64,
    waiting: Vec<Caller>,
}

struct Caller {
    id: u64,
    resolver: Arc<dyn ArtifactResolver>,
    /// Cancelled when the caller is cancelled or leaves.
    left: CancellationToken,
}

impl Askers {
    fn lock(&self) -> MutexGuard<'_, Callers> {
        self.0.lock().expect("the waiting callers are intact")
    }

    /// `resolver` joins for a caller that waits until `cancel` is cancelled
    /// or the answer is dropped.
    pub(crate) fn join(&self, resolver: Arc<dyn ArtifactResolver>, cancel: &CancellationToken) -> Asking {
        let mut callers = self.lock();
        let id = callers.next;
        callers.next += 1;
        callers.waiting.push(Caller {
            id,
            resolver,
            left: cancel.child_token(),
        });
        Asking {
            askers: self.clone(),
            id,
        }
    }
}

/// A caller's place among the askers; dropping it takes the caller off.
pub(crate) struct Asking {
    askers: Askers,
    id: u64,
}

impl Drop for Asking {
    fn drop(&mut self) {
        let mut callers = self.askers.lock();
        if let Some(index) = callers.waiting.iter().position(|caller| caller.id == self.id) {
            callers.waiting.remove(index).left.cancel();
        }
    }
}

impl ArtifactResolver for Askers {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            loop {
                let first = self
                    .lock()
                    .waiting
                    .iter()
                    .find(|caller| !caller.left.is_cancelled())
                    .map(|caller| (caller.resolver.clone(), caller.left.clone()));
                // With nobody waiting, the work is about to be dropped.
                let Some((resolver, left)) = first else {
                    return Err(RuntimeError::Cancelled);
                };
                let resolved = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(RuntimeError::Cancelled),
                    _ = left.cancelled() => continue,
                    resolved = resolver.resolve(artifact, &left) => resolved,
                };
                match resolved {
                    Err(_) if left.is_cancelled() => continue,
                    resolved => return resolved,
                }
            }
        })
    }
}
