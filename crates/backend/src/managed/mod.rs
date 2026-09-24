//! The Cloud (`managed-hosts.md`): each user's one managed device, a Host
//! behind a runner like a paired device, which the machine manager runs as
//! a gVisor sandbox. The user's shard holds the Cloud's machine: its phase
//! and the capacity permit it holds while it is not stopped, its gate, which
//! every operation on it holds and which an idle stop and a reset reserve,
//! and the transitions that change it: boot and recovery, save,
//! checkpoint and reset. The capacity across users and the machine
//! manager's client are services every shard shares.

mod access;
pub(crate) mod capacity;
pub(crate) mod client;
mod growth;
mod machine;
mod maintenance;
mod reset;
mod status;
mod uses;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, OnceLock};

use demi_runner_protocol::values::BackendUrl;
use tokio::sync::mpsc;
use url::Url;

pub(crate) use self::capacity::CloudCapacity;
pub(crate) use self::client::MachinesClient;
pub(crate) use self::machine::{Cloud, CloudAdmission, CloudError};
pub(crate) use self::reset::recover_resets;
pub(crate) use self::status::operation_dto;
use crate::backend::Services;
use crate::config::CloudTuning;
use crate::shard::Shards;

/// What every shard's Cloud shares: the machine manager's client, the
/// capacity across users, the Cloud's settings, and the URL a Cloud's
/// runner connects to.
pub(crate) struct CloudServices {
    pub(crate) machines: MachinesClient,
    pub(crate) capacity: CloudCapacity,
    pub(crate) tuning: CloudTuning,
    backend_url: OnceLock<BackendUrl>,
}

impl CloudServices {
    pub(crate) fn new(machines: MachinesClient, tuning: CloudTuning) -> Self {
        Self {
            capacity: CloudCapacity::new(tuning.capacity),
            machines,
            tuning,
            backend_url: OnceLock::new(),
        }
    }

    /// Sets the URL a Cloud's runner connects to once the backend listens
    /// on `address`, before it serves: the public URL, or without one the
    /// listener's own address.
    pub(crate) fn listening(&self, public: Option<&Url>, address: SocketAddr) {
        let url = match public {
            Some(public) => public.as_str().parse::<BackendUrl>(),
            None => {
                // A listener on every address is reached on the loopback one.
                let ip = match address.ip() {
                    IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
                    ip => ip,
                };
                format!("http://{}", SocketAddr::new(ip, address.port())).parse::<BackendUrl>()
            }
        };
        match url {
            // A backend listens once; a second call finds the URL set.
            Ok(url) => {
                let _ = self.backend_url.set(url);
            }
            Err(error) => tracing::error!("the URL a Cloud's runner connects to is not usable: {error}"),
        }
    }

    /// The URL a Cloud's runner connects to, once the backend listens.
    pub(crate) fn backend_url(&self) -> Option<&BackendUrl> {
        self.backend_url.get()
    }
}

/// Routes each death the machine manager reports to the shard of the
/// device's owner (`backend.md` § Runtime model), until the backend closes.
pub(crate) async fn route_deaths(
    mut deaths: mpsc::Receiver<demi_machines_protocol::DeviceId>,
    services: Arc<Services>,
    shards: Shards,
) {
    while let Some(device) = deaths.recv().await {
        let Ok(id) = demi_web_api::ids::DeviceId::try_from(device.as_str()) else {
            continue;
        };
        let owner = match services.control.device(id.clone()).await {
            Ok(Some(record)) => record.user,
            // A device the backend no longer has is no one's to stop.
            Ok(None) => continue,
            Err(error) => {
                tracing::error!(device = %id, "the death of a Cloud could not be routed: {error}");
                continue;
            }
        };
        // A shard that is closing has no Cloud left to stop.
        let _ = shards
            .of(&owner)
            .call(move |shard, _| async move { shard.cloud_died(&id) })
            .await;
    }
}
