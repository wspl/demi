//! Exposes (`expose.md`): a service on one of the user's devices under a
//! public URL for an hour. The records live in storage, and the operations
//! here create, list, renew and remove them for the page and for the agent's
//! `demi host expose` (`records`). The owner's shard admits each relayed
//! connection, which is where the record and the connection limit are
//! checked, and counts it until the edge is done with it or the expose ends
//! (`relay`); the shard opens the network stream to the device, and the edge
//! relays the bytes. Whatever destroys an expose ends its connections: its
//! expiry, its removal, its Cloud's stop and its device's revocation.
//!
//! The operations are methods of `dyn ExposeShard`, what an expose needs of
//! its user's shard (`concurrency.md` § The user shard), which the shard
//! implements.

pub mod domain;
pub mod records;
pub mod relay;

use std::rc::Rc;

use demi_backend_storage::control::ControlService;
use demi_backend_storage::devices::DeviceRecord;
use demi_backend_sync::UserMarks;
use demi_core::Clock;
use demi_web_api::ids::UserId;
use tokio_util::task::TaskTracker;
use url::Url;

use self::domain::ExposeDomain;
use self::relay::Exposes;

/// What an expose needs of its user's shard: the handles its operations
/// use, and whether a device takes a new expose.
pub trait ExposeShard {
    fn user(&self) -> &UserId;
    fn control(&self) -> &ControlService;
    /// The wall clock expiries are read by.
    fn clock(&self) -> &dyn Clock;
    /// The user's pages, which show the exposes.
    fn marks(&self) -> UserMarks;
    /// The user's exposes with relayed connections open.
    fn exposes(&self) -> &Exposes;
    /// The domain of expose hostnames; without one, the instance has no
    /// exposes.
    fn domain(&self) -> Option<&ExposeDomain>;
    /// The backend's public URL, whose scheme and port an expose's URL
    /// takes.
    fn public_url(&self) -> &Url;
    /// Every task the shard spawns, which its close waits for.
    fn tasks(&self) -> &TaskTracker;
    /// The shard, for a task that outlives the call that starts it.
    fn this(&self) -> Rc<dyn ExposeShard>;
    /// Whether `device` takes a new expose: its runner is connected and,
    /// for a Cloud, the Cloud runs (`expose.md` § Lifetime).
    fn device_connected(&self, device: &DeviceRecord) -> bool;
}
