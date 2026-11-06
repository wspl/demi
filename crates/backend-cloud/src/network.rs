//! What Clouds reach: the master's network policy, which the control
//! database stores as the master wrote it, turned into the rules the machine
//! manager applies (`crates/machine-manager-protocol/src/network.rs`).

use demi_core_store::StorageError;
pub use demi_machine_manager_protocol::network::AllowedError;
use demi_machine_manager_protocol::{
    Allowed, NetworkRules, SetNetworkRulesParams,
    network::DEVELOPER_SERVICES,
};
use demi_web_api_protocol::settings::{CloudNetwork, CustomNetworkPolicy, NetworkPolicy};

use crate::{CloudServices, ControlService};
use crate::client::MachinesError;

/// An entry of Custom's list that is not one, with the place it holds in
/// the list.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct InvalidAllowed {
    pub index: usize,
    pub error: AllowedError,
}

/// The rules `policy` gives, or the first entry of Custom's list that is not
/// one. The list is read whatever the policy, since a common policy keeps it
/// for when Custom is chosen again.
pub fn network_rules(policy: NetworkPolicy, custom: &CustomNetworkPolicy) -> Result<NetworkRules, InvalidAllowed> {
    let allowed = custom
        .allowed
        .iter()
        .enumerate()
        .map(|(index, written)| written.parse::<Allowed>().map_err(|error| InvalidAllowed { index, error }))
        .collect::<Result<_, _>>()?;
    let groups = |internet, private_network, developer_services| NetworkRules {
        internet,
        private_network,
        developer_services,
        allowed: Vec::new(),
    };
    Ok(match policy {
        NetworkPolicy::NoNetwork => groups(false, false, false),
        NetworkPolicy::DeveloperServices => groups(false, false, true),
        NetworkPolicy::InternetOnly => groups(true, false, false),
        NetworkPolicy::InternetAndPrivateNetwork => groups(true, true, false),
        NetworkPolicy::Custom => NetworkRules {
            internet: custom.internet,
            private_network: custom.private_network,
            developer_services: custom.developer_services,
            allowed,
        },
    })
}

/// Settings › Cloud's network policy as the page reads it.
pub async fn setting(control: &ControlService) -> Result<CloudNetwork, StorageError> {
    let (policy, custom) = control.network_policy().await?;
    Ok(CloudNetwork {
        policy,
        custom,
        developer_services: DEVELOPER_SERVICES.iter().map(|name| (*name).to_owned()).collect(),
    })
}

/// The rules the stored policy gives. The backend stores only a policy it
/// read, so an entry that is no longer one is left out and logged rather
/// than keeping every Cloud from starting.
pub async fn stored_network_rules(control: &ControlService) -> Result<NetworkRules, StorageError> {
    let (policy, mut custom) = control.network_policy().await?;
    custom.allowed.retain(|written| match written.parse::<Allowed>() {
        Ok(_) => true,
        Err(error) => {
            tracing::error!(event = "cloud_network.entry_ignored", entry = written, %error, "a stored entry of the network policy is not one, and Clouds do not reach it");
            false
        }
    });
    Ok(network_rules(policy, &custom).expect("only entries that parse are left"))
}

/// Has the manager apply `rules` to every Cloud now.
pub async fn apply(cloud: &CloudServices, rules: NetworkRules) -> Result<(), MachinesError> {
    cloud.machines.call(SetNetworkRulesParams { network_rules: rules }).await
}
