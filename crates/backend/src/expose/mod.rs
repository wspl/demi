//! Exposes (`expose.md`): a service on one of the user's devices under a
//! public URL. The records live in storage; the owner's shard admits each
//! relayed connection, which is where the record, the device and the
//! connection limit are checked, and hands the edge the network stream's
//! ends with a lease; the edge relays the bytes (`edge::expose`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;

use demi_host_remote::{PipeReader, PipeWriter};
use demi_shell::HostErrorKind;
use demi_web_api::ids::ExposeId;
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;
use crate::shard::lease::Lease;
use crate::storage::StorageError;

/// Concurrent relayed connections per expose; one more answers 503
/// (`expose.md` § The public relay).
const CONNECTION_LIMIT: usize = 64;

/// The domain expose hostnames live under (`expose.md` § Deployment), such
/// as `expose.demi.example`: a DNS name, in lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposeDomain(String);

/// Why a text is not an expose domain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("must be a domain name, such as expose.demi.example")]
pub struct NotExposeDomain;

impl FromStr for ExposeDomain {
    type Err = NotExposeDomain;

    fn from_str(text: &str) -> Result<Self, NotExposeDomain> {
        match url::Host::parse(text) {
            Ok(url::Host::Domain(domain)) if domain.split('.').all(|label| !label.is_empty()) => Ok(Self(domain)),
            _ => Err(NotExposeDomain),
        }
    }
}

impl ExposeDomain {
    /// The label an expose hostname puts before this domain: `host`, a
    /// `Host` header's host without its port, is `<label>.<domain>` in any
    /// case, and the label has no dot. `None` for any other host.
    pub(crate) fn label(&self, host: &str) -> Option<String> {
        let host = host.to_ascii_lowercase();
        let label = host.strip_suffix(self.0.as_str())?.strip_suffix('.')?;
        (!label.is_empty() && !label.contains('.')).then(|| label.to_owned())
    }
}

/// The user's live exposes: those with relayed connections open, each with
/// how many and the token that ends them.
#[derive(Default)]
pub(crate) struct Exposes(Rc<RefCell<HashMap<ExposeId, LiveExpose>>>);

struct LiveExpose {
    connections: usize,
    /// Cancelled, every connection of the expose ends.
    ended: CancellationToken,
}

impl Exposes {
    /// Counts one more connection of `id`, unless the expose is at its
    /// limit.
    fn register(&self, id: &ExposeId) -> Option<Registration> {
        let mut live = self.0.borrow_mut();
        let expose = live.entry(id.clone()).or_insert_with(|| LiveExpose {
            connections: 0,
            ended: CancellationToken::new(),
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

    /// Ends every connection, as the shard's close does.
    pub(crate) fn end_all(&self) {
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

/// An admitted relayed connection, as the edge relays it: a network stream
/// to the expose's address on its device. `Send`.
pub(crate) struct ExposeConnection {
    /// The visitor's bytes, which the runner writes to the socket; its end
    /// is the socket's half-close.
    pub(crate) to_service: PipeWriter,
    /// What the socket sends, which ends with the socket's end-of-stream.
    pub(crate) from_service: PipeReader,
    /// Held while the connection is relayed; it ends when the expose does.
    pub(crate) lease: Lease,
}

/// Why a relayed connection was not admitted (`expose.md` § The public
/// relay).
#[derive(Debug, thiserror::Error)]
pub(crate) enum RelayRefusal {
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

impl Shard {
    /// Admits one relayed connection to the expose `id` of this shard's
    /// user: the expose exists and has not expired, its device is
    /// connected, it is below its connection limit, and the runner connected
    /// to its address, which device access reaches without a conversation,
    /// a file gate or a wake. The connection counts until the edge drops
    /// its lease or the expose ends; then both pipes fail, which closes the
    /// runner's socket, unless they completed.
    pub(crate) async fn open_expose_connection(
        &self,
        id: &ExposeId,
        cancel: &CancellationToken,
    ) -> Result<ExposeConnection, RelayRefusal> {
        // Counted before the record is read, so an expose that ends while it
        // is read ends this connection too.
        let registration = self.exposes().register(id).ok_or(RelayRefusal::Limit)?;
        let record = self.services().control.expose(id.clone()).await?;
        let record = record
            .filter(|record| record.user == *self.user() && self.services().clock.now() < record.expires_at)
            .ok_or(RelayRefusal::NotFound)?;
        if registration.ended.is_cancelled() {
            return Err(RelayRefusal::Removed);
        }
        let host = self
            .devices()
            .device_access(&record.device)
            .ok_or(RelayRefusal::DeviceOffline)?;
        let device = record.device.as_str();
        let input = self.pipes().to_device(device);
        let output = self.pipes().from_device(device);
        let to_service = input.writer().expect("a pipe just made has its source free");
        let from_service = output.reader().expect("a pipe just made has its sink free");
        let opened = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(RelayRefusal::Removed),
            () = registration.ended.cancelled() => Err(RelayRefusal::Removed),
            opened = host.open_net(
                record.address.host(),
                record.address.port(),
                input.wire_ref(),
                output.wire_ref(),
            ) => opened.map_err(|error| match error.kind {
                HostErrorKind::Offline => RelayRefusal::DeviceOffline,
                _ => RelayRefusal::Unreachable {
                    code: error.code().unwrap_or("unreachable").to_owned(),
                },
            }),
        };
        if let Err(refusal) = opened {
            input.fail("the relayed connection never opened");
            output.fail("the relayed connection never opened");
            return Err(refusal);
        }
        let ended = registration.ended.clone();
        let (lease, released) = Lease::new(ended.clone());
        self.tasks().spawn_local(async move {
            tokio::select! {
                () = released => {}
                () = ended.cancelled() => {}
            }
            input.fail("the relayed connection ended");
            output.fail("the relayed connection ended");
            drop(registration);
        });
        Ok(ExposeConnection {
            to_service,
            from_service,
            lease,
        })
    }
}
