//! How Clouds are isolated: the master's choice, which the control database
//! stores, and what the machine manager can do with each isolation, which
//! its `hello` says (`crates/machine-manager-protocol/src/isolation.rs`).
//! The manager keeps the choice in memory only, so the backend names it on
//! each connection as well as after each change.

use demi_backend_database::StorageError;
use demi_machine_manager_protocol::{DeviceId, Isolation as MachineIsolation, SetIsolationParams};
use demi_web_api_protocol::settings::{CloudIsolation, Isolation, IsolationRuntime};

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
                let enforces = offer.offer.as_ref().ok();
                IsolationRuntime {
                    isolation: web_isolation(offer.isolation),
                    unavailable: offer.offer.as_ref().err().map(|unavailable| unavailable.reason.clone()),
                    private_network: enforces.is_some_and(|enforces| enforces.private_network),
                    storage_limits: enforces.is_some_and(|enforces| enforces.storage_limits),
                }
            })
            .collect()
    });
    Ok(CloudIsolation {
        isolation: control.isolation().await?,
        runtimes,
    })
}

/// Whether the manager's firewall keeps the Cloud `device` from the private
/// network unless the master opens it: its storage's isolation enforces
/// Private Network Access, or the manager has not said.
pub fn private_network_closed(cloud: &CloudServices, device: &str) -> bool {
    let machines = &cloud.machines;
    DeviceId::parse(device)
        .ok()
        .and_then(|device| machines.status(&device))
        .and_then(|status| machines.enforces(status.isolation))
        .is_none_or(|enforces| enforces.private_network)
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
