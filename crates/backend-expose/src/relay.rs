//! The public relay's connections in the owner's shard (`expose.md` § The
//! public relay): each expose's live connections with how many there are,
//! the token that ends them, and the watch that destroys the expose once it
//! expires; and the admission of a relayed connection up to its device.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_backend_database::StorageError;
use demi_backend_database::exposes::ExposeRecord;
use demi_shared_types::{Clock, Timestamp};
use demi_web_api_protocol::exposes::ExposeDto;
use demi_web_api_protocol::ids::ExposeId;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::ExposeShard;

/// Concurrent relayed connections per expose; one more answers 503.
const CONNECTION_LIMIT: usize = 64;

/// The user's live exposes: those with relayed connections open.
#[derive(Default)]
pub struct Exposes(Rc<RefCell<HashMap<ExposeId, LiveExpose>>>);

struct LiveExpose {
    connections: usize,
    /// Cancelled, every connection of the expose ends.
    ended: CancellationToken,
    /// Destroys the expose once it expired, so its open connections end
    /// then too; the first connection admitted starts it, and it stops with
    /// the last one.
    expiry: Option<AbortOnDropHandle<()>>,
}

impl Exposes {
    /// Counts one more connection of `id`, unless the expose is at its
    /// limit.
    fn register(&self, id: &ExposeId) -> Option<Registration> {
        let mut live = self.0.borrow_mut();
        let expose = live.entry(id.clone()).or_insert_with(|| LiveExpose {
            connections: 0,
            ended: CancellationToken::new(),
            expiry: None,
        });
        if expose.connections == CONNECTION_LIMIT {
            return None;
        }
        expose.connections += 1;
        Some(Registration {
            exposes: self.0.clone(),
            id: id.clone(),
            ended: expose.ended.clone(),
        })
    }

    /// Ends every connection of the exposes `ids`, which were destroyed.
    pub fn end<'a>(&self, ids: impl IntoIterator<Item = &'a ExposeId>) {
        let live = self.0.borrow();
        for id in ids {
            if let Some(expose) = live.get(id) {
                expose.ended.cancel();
            }
        }
    }

    /// Ends every connection, as the shard's close does.
    pub fn end_all(&self) {
        for expose in self.0.borrow().values() {
            expose.ended.cancel();
        }
    }
}

/// One relayed connection's place among its expose's; dropping it frees
/// the place.
struct Registration {
    exposes: Rc<RefCell<HashMap<ExposeId, LiveExpose>>>,
    id: ExposeId,
    ended: CancellationToken,
}

impl Registration {
    /// Starts the expose's expiry watch with `start`, unless one runs.
    fn watch_expiry(&self, start: impl FnOnce() -> AbortOnDropHandle<()>) {
        let mut live = self.exposes.borrow_mut();
        if let Some(expose) = live.get_mut(&self.id)
            && expose.expiry.is_none()
        {
            expose.expiry = Some(start());
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut live = self.exposes.borrow_mut();
        let Some(expose) = live.get_mut(&self.id) else {
            return;
        };
        expose.connections -= 1;
        if expose.connections == 0 {
            live.remove(&self.id);
        }
    }
}

/// Why a relayed connection was not admitted.
#[derive(Debug, thiserror::Error)]
pub enum RelayRefusal {
    /// No such expose, or it expired.
    #[error("no such expose")]
    NotFound,
    /// The expose's device is not connected.
    #[error("the device is offline")]
    DeviceOffline,
    /// The runner could not connect; the code is its reason, such as
    /// `refused`.
    #[error("the service is unreachable ({code})")]
    Unreachable { code: String },
    /// The expose has as many connections as it may.
    #[error("the expose is at its connection limit")]
    Limit,
    /// The expose ended while the connection was being admitted.
    #[error("the expose was removed")]
    Removed,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// A relayed connection admitted up to its device: it counts among its
/// expose's from its admission on, and the shard opens its network stream
/// next.
pub struct RelayAdmission {
    registration: Registration,
    record: ExposeRecord,
}

impl RelayAdmission {
    /// The expose, which has not expired.
    pub fn record(&self) -> &ExposeRecord {
        &self.record
    }

    /// The token that ends the connection, cancelled once the expose ends.
    pub fn ending(&self) -> CancellationToken {
        self.registration.ended.clone()
    }

    /// Relays the connection: the expose's expiry watch starts unless one
    /// runs, and the connection counts until `released` resolves or the
    /// expose ends, which then runs `end`.
    pub fn relay(
        self,
        shard: &(dyn ExposeShard + 'static),
        released: impl Future<Output = ()> + 'static,
        end: impl FnOnce() + 'static,
    ) {
        let RelayAdmission {
            registration,
            record,
        } = self;
        registration.watch_expiry(|| {
            let watch = expire_when_due(Rc::downgrade(&shard.this()), record.id, record.expires_at);
            AbortOnDropHandle::new(shard.tasks().spawn_local(watch))
        });
        let ended = registration.ended.clone();
        shard.tasks().spawn_local(async move {
            tokio::select! {
                () = released => {}
                () = ended.cancelled() => {}
            }
            end();
            drop(registration);
        });
    }
}

impl dyn ExposeShard {
    /// Admits one relayed connection to the expose `id` of this shard's
    /// user up to its device: the expose exists and has not expired, and it
    /// is below its connection limit. The shard then connects to the
    /// expose's address through device access (`sessions-and-targets.md`
    /// § Every way to a Host) and relays the connection.
    pub async fn admit_relay(&self, id: &ExposeId) -> Result<RelayAdmission, RelayRefusal> {
        // Counted before the record is read, so an expose that ends while it
        // is read ends this connection too.
        let registration = self.exposes().register(id).ok_or(RelayRefusal::Limit)?;
        let record = self.owned_expose(id).await?.ok_or(RelayRefusal::NotFound)?;
        if self.destroy_if_expired(&record).await? {
            return Err(RelayRefusal::NotFound);
        }
        if registration.ended.is_cancelled() {
            return Err(RelayRefusal::Removed);
        }
        Ok(RelayAdmission {
            registration,
            record,
        })
    }
}

/// Destroys the expose `id` of the shard's user once it expired, first at
/// `expires_at`, and again at each later expiry a renewal set meanwhile;
/// ends once the expose is gone.
async fn expire_when_due(shard: Weak<dyn ExposeShard>, id: ExposeId, mut expires_at: Timestamp) {
    loop {
        let Some(now) = shard.upgrade().map(|shard| shard.clock().now()) else {
            return;
        };
        tokio::time::sleep(until(now, expires_at)).await;
        let Some(shard) = shard.upgrade() else {
            return;
        };
        let expired = async {
            let Some(record) = shard.owned_expose(&id).await? else {
                return Ok(None);
            };
            if shard.destroy_if_expired(&record).await? {
                return Ok(None);
            }
            Ok::<_, StorageError>(Some(record.expires_at))
        };
        match expired.await {
            Ok(Some(renewed)) => expires_at = renewed,
            Ok(None) => return,
            Err(error) => {
                // The expose's connections then end on their own or by the
                // idle rule, and the next read of the record destroys it.
                tracing::error!(expose = %id, "an expired expose could not be destroyed: {error}");
                return;
            }
        }
    }
}

/// Resolves once the first of `exposes` expires by `clock`'s time, as a
/// page that shows them must learn (`web-api.md` § Page synchronization);
/// never while there is none.
pub async fn first_expiry(clock: &dyn Clock, exposes: &[ExposeDto]) {
    let Some(first) = exposes.iter().map(|expose| expose.expires_at).min() else {
        return std::future::pending().await;
    };
    tokio::time::sleep(until(clock.now(), first)).await;
}

/// How long from `now` until `at`, none once it passed.
fn until(now: Timestamp, at: Timestamp) -> Duration {
    let milliseconds = at.as_millisecond().saturating_sub(now.as_millisecond());
    Duration::from_millis(u64::try_from(milliseconds).unwrap_or(0))
}
