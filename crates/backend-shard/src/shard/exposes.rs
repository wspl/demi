//! The shard's side of the user's exposes (`expose.md`): what an expose
//! needs of the shard (`ExposeShard`), and the relayed connection's network
//! stream, which the shard opens on the expose's device through device
//! access (`sessions-and-targets.md` § Every way to a Host) and hands the
//! edge with a lease.

use std::rc::Rc;

use demi_backend_expose::ExposeShard;
use demi_backend_expose::domain::ExposeDomain;
use demi_backend_expose::relay::{Exposes, RelayRefusal};
use demi_backend_storage::control::ControlService;
use demi_backend_storage::devices::DeviceRecord;
use demi_backend_sync::UserMarks;
use demi_core::Clock;
use demi_host_remote::{PipeReader, PipeWriter};
use demi_shell::HostErrorKind;
use demi_web_api::devices::DeviceKind;
use demi_web_api::ids::{ExposeId, UserId};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use super::Shard;
use demi_backend_host_access::lease::Lease;

impl ExposeShard for Shard {
    fn user(&self) -> &UserId {
        &self.user
    }

    fn control(&self) -> &ControlService {
        &self.services.control
    }

    fn clock(&self) -> &dyn Clock {
        &*self.services.clock
    }

    fn marks(&self) -> UserMarks {
        self.services.sync.of(&self.user)
    }

    fn exposes(&self) -> &Exposes {
        &self.exposes
    }

    fn domain(&self) -> Option<&ExposeDomain> {
        self.services.expose_domain.as_ref()
    }

    fn public_url(&self) -> &Url {
        self.services
            .public_url
            .get()
            .expect("the backend listens before it serves a request or a command")
            .url()
    }

    fn tasks(&self) -> &TaskTracker {
        &self.tasks
    }

    fn this(&self) -> Rc<dyn ExposeShard> {
        self.this
            .upgrade()
            .expect("a shard's method runs while the shard lives")
    }

    fn device_connected(&self, device: &DeviceRecord) -> bool {
        let online = self.devices.online(&device.id);
        match device.kind {
            DeviceKind::User => online,
            DeviceKind::Managed => online && self.cloud.runs(&device.id),
        }
    }
}

/// An admitted relayed connection, as the edge relays it: a network stream
/// to the expose's address on its device. `Send`.
pub struct ExposeConnection {
    /// The visitor's bytes, which the runner writes to the socket; its end
    /// is the socket's half-close.
    pub to_service: PipeWriter,
    /// What the socket sends, which ends with the socket's end-of-stream.
    pub from_service: PipeReader,
    /// Held while the connection is relayed; it ends when the expose does.
    pub lease: Lease,
}

impl Shard {
    /// The shard as its exposes see it, whose operations they are.
    pub fn expose_shard(&self) -> &(dyn ExposeShard + 'static) {
        self
    }

    /// Admits one relayed connection to the expose `id` of this shard's
    /// user (`expose.md` § The public relay): the expose admits it, its
    /// device is connected, and the runner connected to its address, which
    /// device access reaches without a conversation, a file gate or a wake.
    /// The connection counts until the edge drops its lease or the expose
    /// ends; then both pipes fail, which closes the runner's socket, unless
    /// they completed.
    pub async fn open_expose_connection(
        &self,
        id: &ExposeId,
        cancel: &CancellationToken,
    ) -> Result<ExposeConnection, RelayRefusal> {
        let admission = self.expose_shard().admit_relay(id).await?;
        let record = admission.record();
        let host = self
            .devices()
            .device_access(&record.device)
            .ok_or(RelayRefusal::DeviceOffline)?;
        let device = record.device.as_str();
        let input = self.pipes().to_device(device);
        let output = self.pipes().from_device(device);
        let to_service = input
            .writer()
            .expect("a pipe just made has its source free");
        let from_service = output.reader().expect("a pipe just made has its sink free");
        let ending = admission.ending();
        let opened = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(RelayRefusal::Removed),
            () = ending.cancelled() => Err(RelayRefusal::Removed),
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
        let (lease, released) = Lease::new(ending);
        admission.relay(self.expose_shard(), released, move || {
            input.fail("the relayed connection ended");
            output.fail("the relayed connection ended");
        });
        Ok(ExposeConnection {
            to_service,
            from_service,
            lease,
        })
    }
}
