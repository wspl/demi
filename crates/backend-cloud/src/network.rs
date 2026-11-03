//! Which private destinations Clouds reach: the master's setting, which the
//! control database stores as the master wrote it, as the machine manager
//! applies it (`crates/machine-manager-protocol/src/network.rs`).

use demi_backend_database::StorageError;
pub use demi_machine_manager_protocol::network::DestinationError;
use demi_machine_manager_protocol::{Destination, PrivateNetwork, SetPrivateNetworkParams};
use demi_web_api_protocol::settings::{CloudNetwork, PrivateNetworkAccess};

use crate::{CloudServices, ControlService};
use crate::client::MachinesError;

/// A destination of the setting that is not one, with the place it holds in
/// the list.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct InvalidDestination {
    pub index: usize,
    pub error: DestinationError,
}

/// The private network `setting` names, or the first destination it lists
/// that is not one. The list is read whatever the access, since another
/// access keeps it for when Only These Addresses is chosen again.
pub fn private_network(setting: &CloudNetwork) -> Result<PrivateNetwork, InvalidDestination> {
    let destinations = setting
        .destinations
        .iter()
        .enumerate()
        .map(|(index, written)| {
            written
                .parse::<Destination>()
                .map_err(|error| InvalidDestination { index, error })
        })
        .collect::<Result<_, _>>()?;
    Ok(match setting.access {
        PrivateNetworkAccess::Off => PrivateNetwork::Off,
        PrivateNetworkAccess::Entire => PrivateNetwork::Entire,
        PrivateNetworkAccess::Only => PrivateNetwork::Only(destinations),
    })
}

/// The private network the stored setting names. The backend stores only a
/// setting it read, so a destination that is no longer one is left out and
/// logged rather than keeping every Cloud from starting.
pub async fn stored_private_network(control: &ControlService) -> Result<PrivateNetwork, StorageError> {
    let mut setting = control.cloud_network().await?;
    setting.destinations.retain(|written| match written.parse::<Destination>() {
        Ok(_) => true,
        Err(error) => {
            tracing::error!(event = "cloud_network.destination_ignored", destination = written, %error, "a stored private destination is not one, and Clouds do not reach it");
            false
        }
    });
    Ok(private_network(&setting).expect("only destinations that parse are left"))
}

/// Has the manager apply `network` to every Cloud now.
pub async fn apply(cloud: &CloudServices, network: PrivateNetwork) -> Result<(), MachinesError> {
    cloud
        .machines
        .call(SetPrivateNetworkParams {
            private_network: network,
        })
        .await
}
