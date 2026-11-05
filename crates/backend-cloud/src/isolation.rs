//! How Clouds are isolated: the master's choice, which the control database
//! stores, and what the machine manager can do with each isolation, which
//! its `hello` says (`crates/machine-manager-protocol/src/isolation.rs`).
//! The manager keeps the choice in memory only, so the backend names it on
//! each connection as well as after each change.

use demi_backend_database::StorageError;
use demi_machine_manager_protocol::{Download, DeviceId, Isolation as MachineIsolation, Offer, SetIsolationParams};
use demi_web_api_protocol::settings::{CloudIsolation, Isolation, IsolationDownload, IsolationRuntime};

use crate::client::MachinesError;
use crate::{CloudServices, ControlService};

/// The manager's name for `isolation`.
pub fn machine_isolation(isolation: Isolation) -> MachineIsolation {
    match isolation {
        Isolation::Simple => MachineIsolation::Simple,
        Isolation::Gvisor => MachineIsolation::Gvisor,
    }
}

fn web_isolation(isolation: MachineIsolation) -> Isolation {
    match isolation {
        MachineIsolation::Simple => Isolation::Simple,
        MachineIsolation::Gvisor => Isolation::Gvisor,
    }
}

/// Has the manager run Clouds on `isolation` from now on.
pub async fn apply(cloud: &CloudServices, isolation: Isolation) -> Result<(), MachinesError> {
    cloud
        .machines
        .call(SetIsolationParams {
            isolation: machine_isolation(isolation),
        })
        .await
}

/// The chosen isolation, and what the manager last said it can do with
/// each.
pub async fn setting(control: &ControlService, cloud: &CloudServices) -> Result<CloudIsolation, StorageError> {
    let runtimes = cloud.machines.runtimes().map(|offers| {
        offers
            .into_iter()
            .map(|offer| {
                let (enforces, unavailable, download) = match offer.offer {
                    Offer::Runs(enforces) => (Some(enforces), None, None),
                    Offer::AfterDownload { enforces, download } => (Some(enforces), None, Some(web_download(download))),
                    Offer::Unavailable(unavailable) => (None, Some(unavailable.reason), None),
                };
                IsolationRuntime {
                    isolation: web_isolation(offer.isolation),
                    unavailable,
                    network_policy: enforces.is_some_and(|enforces| enforces.network_policy),
                    storage_limits: enforces.is_some_and(|enforces| enforces.storage_limits),
                    download,
                }
            })
            .collect()
    });
    Ok(CloudIsolation {
        isolation: control.isolation().await?,
        runtimes,
    })
}

/// Why the manager can't run `isolation`, in its words; none when it runs
/// it, runs it once it has downloaded it, or has not said.
pub fn unavailable(cloud: &CloudServices, isolation: Isolation) -> Option<String> {
    let isolation = machine_isolation(isolation);
    cloud
        .machines
        .runtimes()?
        .into_iter()
        .find(|offer| offer.isolation == isolation)
        .and_then(|offer| match offer.offer {
            Offer::Unavailable(unavailable) => Some(unavailable.reason),
            Offer::Runs(_) | Offer::AfterDownload { .. } => None,
        })
}

fn web_download(download: Download) -> IsolationDownload {
    match download {
        Download::Waiting => IsolationDownload::Waiting,
        Download::Downloading { received, total } => IsolationDownload::Downloading { received, total },
        Download::Starting => IsolationDownload::Starting,
        Download::Failed { message } => IsolationDownload::Failed { message },
    }
}

/// Whether the network policy decides what the Cloud `device` reaches: its
/// storage's isolation enforces it, or the manager has not said.
pub fn network_policy_applies(cloud: &CloudServices, device: &str) -> bool {
    let machines = &cloud.machines;
    DeviceId::parse(device)
        .ok()
        .and_then(|device| machines.status(&device))
        .and_then(|status| machines.enforces(status.isolation))
        .is_none_or(|enforces| enforces.network_policy)
}

/// Whether each user's Cloud Storage bounds the Clouds: the chosen
/// isolation enforces it, or the manager has not said.
pub async fn storage_limits(control: &ControlService, cloud: &CloudServices) -> Result<bool, StorageError> {
    let isolation = machine_isolation(control.isolation().await?);
    Ok(match cloud.machines.runtimes() {
        Some(_) => cloud.machines.enforces(isolation).is_some_and(|enforces| enforces.storage_limits),
        None => true,
    })
}
