//! `demi host expose`: lists, adds and removes the ports of the user's devices
//! that the backend's requests for the user may reach.
//!
//! `list` needs no permission. `add` and `remove` need Manage Exposed Ports
//! (`host.expose`).

use std::rc::{Rc, Weak};

use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_web_api_protocol::exposed_ports::{ExposedPortDto, NewExposedPort};
use demi_web_api_protocol::text::Trimmed;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::output::{Cell, Fields, MISSING, Table};

use crate::HostShard;
use crate::host_commands::{NoArgs, conversation_of, host_names, print_json, resolve_host, verb};

/// The category `add` and `remove` need.
pub const MANAGE: &str = "host.expose";
/// The category's action, which completes "Allow this conversation to …".
pub const MANAGE_ACTION: &str = "manage exposed ports";
/// What a grant of the category allows.
pub const MANAGE_DESCRIPTION: &str = "Expose ports of your devices and stop exposing them. Demi’s requests for you, such as a provider entry’s, can then reach an exposed port of the device.";

const SUMMARY: &str = "The ports of the user's devices that the backend's requests for the user, such as a provider entry's, may reach at http://<device id>:<port>: list, add and remove them.";

const LIST_SUMMARY: &str = "The exposed ports: the device, the port, its label and the address a provider entry's endpoint names it by, such as http://<device id>:11434/v1 with demi providers.";

const ADD_SUMMARY: &str = "Expose a port of a device, such as `demi host expose add \"MacBook Pro\" 11434 --label Ollama`: the backend's requests for the user to http://<device id>:<port> then reach 127.0.0.1:<port> on the device. Prints the exposed port as list shows it, as labelled lines.";

const REMOVE_SUMMARY: &str = "Stop exposing a port of a device: requests to it fail with \"Port <port> of <device> isn’t exposed.\"";

/// The input of `demi host expose add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// The device, by name or id from demi host devices (Cloud for the
    /// user's Cloud)
    #[schemars(length(min = 1))]
    device: String,
    /// The port, from 1 to 65535
    #[schemars(length(min = 1))]
    port: String,
    /// What runs on the port, such as Ollama
    label: Option<String>,
}

/// The input of `demi host expose remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveArgs {
    /// The device, by name or id from demi host devices
    #[schemars(length(min = 1))]
    device: String,
    /// The port, from demi host expose list
    #[schemars(length(min = 1))]
    port: String,
}

/// The `expose` group, whose handlers act in `shard`.
pub fn expose_group(shard: Weak<dyn HostShard>) -> GroupBuilder {
    GroupBuilder::new("expose", SUMMARY)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<ListedPort>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<AddArgs>()
                .positionals(["device", "port"])
                .json_output::<ListedPort>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), add))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<RemoveArgs>()
                .positionals(["device", "port"])
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard, remove))),
        )
}

/// An exposed port as `list` and `add` print it, and their `--json`.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedPort {
    id: String,
    /// The device, by its name: `Cloud` for the Cloud.
    device: String,
    device_id: String,
    /// Null while the user has not entered it on the device's page.
    port: Option<u16>,
    /// Empty for none.
    label: String,
    /// `http://<device id>:<port>`; null while the port is missing.
    address: Option<String>,
}

async fn listed(shard: &dyn HostShard, ports: &[ExposedPortDto]) -> Result<Vec<ListedPort>, RpcError> {
    let names = host_names(shard).await?;
    Ok(ports
        .iter()
        .map(|exposed| ListedPort {
            id: exposed.id.to_string(),
            device: names.name(&exposed.device_id),
            device_id: exposed.device_id.to_string(),
            port: exposed.port,
            label: exposed.label.clone(),
            address: exposed.address.clone(),
        })
        .collect())
}

/// A port as typed: a number from 1 to 65535.
fn port_of(text: &str) -> Result<u16, RpcError> {
    text.parse()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| RpcError::Failed(format!("{text}: a port is a number from 1 to 65535")))
}

async fn list(shard: Rc<dyn HostShard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let ports = shard.exposed_ports().await.map_err(RpcError::Failed)?;
    let listed = listed(&*shard, &ports).await?;
    if call.invocation.json {
        return print_json(&port, &listed).await;
    }
    let mut table = Table::new(["device", "port", "label", "address"]);
    for exposed in &listed {
        table.row::<Cell>([
            exposed.device.as_str().into(),
            exposed.port.map_or_else(|| MISSING.to_owned(), |port| port.to_string()).into(),
            exposed.label.as_str().into(),
            exposed.address.as_deref().unwrap_or(MISSING).into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

async fn add(shard: Rc<dyn HostShard>, call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let args = call.args;
    let number = port_of(&args.port)?;
    let device = resolve_host(&*shard, &conversation, &args.device).await?;
    let label = args.label.map(|label| {
        let Ok(label) = Trimmed::try_from(label);
        label
    });
    let new = NewExposedPort {
        device_id: device.device,
        port: Some(number),
        label,
    };
    new.validate()
        .map_err(|report| RpcError::Failed(demi_web_api_protocol::exposed_ports::refusal(&report)))?;
    let exposed = shard.add_exposed_port(new).await.map_err(RpcError::Failed)?;
    let added = listed(&*shard, std::slice::from_ref(&exposed))
        .await?
        .pop()
        .expect("an exposed port is listed");
    if call.invocation.json {
        return print_json(&port, &added).await;
    }
    let mut fields = Fields::new();
    fields
        .field("ID", added.id.as_str())
        .field("Device", added.device.as_str())
        .field("Port", number)
        .field("Label", added.label.as_str())
        .field("Address", added.address.as_deref());
    port.stdout(fields.render()).await?;
    Ok(0)
}

async fn remove(shard: Rc<dyn HostShard>, call: Call<RemoveArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let number = port_of(&call.args.port)?;
    let device = resolve_host(&*shard, &conversation, &call.args.device).await?;
    let ports = shard.exposed_ports().await.map_err(RpcError::Failed)?;
    let name = host_names(&*shard).await?.name(&device.device);
    let exposed = ports
        .into_iter()
        .find(|exposed| exposed.device_id == device.device && exposed.port == Some(number))
        .ok_or_else(|| {
            RpcError::Failed(format!("Port {number} of {name} isn’t exposed (see `demi host expose list`)"))
        })?;
    shard.remove_exposed_port(exposed.id).await.map_err(RpcError::Failed)?;
    port.stdout(format!("Port {number} of {name} is no longer exposed.\n")).await?;
    Ok(0)
}
