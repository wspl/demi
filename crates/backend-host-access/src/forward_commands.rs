//! `demi host forward` (`port-forwarding.md` § From a conversation): lists,
//! adds and removes the user's port forwards. `list` needs no permission;
//! `add` and `remove` need Manage Port Forwards (`host.forward`), since a
//! forward outlives the conversation and lets any program on its listening
//! Host reach the address it connects to.

use std::rc::{Rc, Weak};

use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_runner_protocol::forward::ForwardRunState;
use demi_web_api_protocol::devices::DeviceState;
use demi_web_api_protocol::ids::DeviceId;
use demi_web_api_protocol::port_forwards::{ForwardAddress, PortForwardDto, PortForwardPatch};
use demi_web_api_protocol::text::Trimmed;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::output::{Cell, Fields, MISSING, Table};

use crate::HostShard;
use crate::host_commands::{NoArgs, conversation_of, host_names, print_json, resolve_host, verb};

/// The category `add` and `remove` need.
pub const MANAGE: &str = "host.forward";
/// The category's action, which completes "Allow this conversation to …".
pub const MANAGE_ACTION: &str = "manage port forwards";
/// What a grant of the category allows.
pub const MANAGE_DESCRIPTION: &str = "Add and remove the user’s port forwards. A forward outlives this conversation, and while it runs any program on the Host it listens on can reach the address it connects to.";

const SUMMARY: &str = "The user's port forwards between their Hosts, as ssh -L makes one: list, add and remove them.";

const LIST_SUMMARY: &str = "The user's port forwards: id, name, where it listens (<host>:<port>), where it connects (<host>:<address>, - for a Host not chosen yet), and what it is doing.";

const ADD_SUMMARY: &str = "Add a port forward: a program on the --listen Host that connects to 127.0.0.1:<port> reaches <address> as the --connect Host reaches it, such as `demi host forward add --listen Cloud:11434 --connect \"MacBook Pro:localhost:11434\" --name ollama`. It runs whenever its listening Host's runner runs, until it is removed. Prints the new forward as list shows it, as labelled lines.";

const REMOVE_SUMMARY: &str = "Remove a port forward by its name: its listener closes and its connections end.";

/// The input of `demi host forward add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// <host>:<port>: the Host that listens, by name or id from demi host
    /// devices (Cloud for the user's Cloud), and the port it listens on, on
    /// 127.0.0.1
    #[schemars(length(min = 3))]
    listen: String,
    /// <host>:<address>: the Host that connects, and the host:port it
    /// connects to, such as "MacBook Pro:localhost:11434"
    #[schemars(length(min = 3))]
    connect: String,
    /// The forward's name, unique among the user's; Forward by default
    name: Option<String>,
}

/// The input of `demi host forward remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveArgs {
    /// The forward's name, from demi host forward list
    #[schemars(length(min = 1))]
    name: String,
}

/// The `forward` group, whose handlers act in `shard`.
pub fn forward_group(shard: Weak<dyn HostShard>) -> GroupBuilder {
    GroupBuilder::new("forward", SUMMARY)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<ListedForward>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<AddArgs>()
                .json_output::<ListedForward>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), add))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<RemoveArgs>()
                .positionals(["name"])
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard, remove))),
        )
}

/// What a forward does now, in the words of its row in Settings › Port
/// Forwarding (`port-forwarding.md` § What the user sees), from what its
/// Listen On runner announced and whether its Hosts are online.
fn state_words(forward: &PortForwardDto, name_of: &dyn Fn(&DeviceId) -> (String, bool)) -> String {
    if !forward.enabled {
        return "Off".to_owned();
    }
    let Some(connect) = &forward.connect_device else {
        return "Choose a Host to connect to.".to_owned();
    };
    for device in [&forward.listen_device, connect] {
        let (name, online) = name_of(device);
        if !online {
            return format!("{name} is offline");
        }
    }
    match &forward.state {
        Some(ForwardRunState::PortInUse) => {
            format!("Port {} is in use on {}", forward.listen_port, name_of(&forward.listen_device).0)
        }
        Some(ForwardRunState::Listening { direct, relay, .. }) => {
            let paths: Vec<String> = [("Direct", *direct), ("Relay", *relay)]
                .into_iter()
                .filter(|(_, count)| *count > 0)
                .map(|(path, count)| match count {
                    1 => format!("{path} · 1 connection"),
                    count => format!("{path} · {count} connections"),
                })
                .collect();
            if paths.is_empty() { "Idle".to_owned() } else { paths.join(", ") }
        }
        None => "Starting…".to_owned(),
    }
}

/// A forward as `list` and `add` print it, and their `--json`.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedForward {
    id: String,
    name: String,
    /// The Host that listens, by its name: `Cloud` for the Cloud.
    listen_host: String,
    listen_device: String,
    /// The port it listens on, on 127.0.0.1.
    listen_port: u16,
    /// The Host that connects; null while none is chosen.
    connect_host: Option<String>,
    connect_device: Option<String>,
    /// The host:port it connects to, as that Host reaches it.
    connect_address: String,
    enabled: bool,
    /// What it is doing, in the words of its row in Settings.
    state: String,
}

impl ListedForward {
    /// `<host>:<port>`.
    fn listen(&self) -> String {
        format!("{}:{}", self.listen_host, self.listen_port)
    }

    /// `<host>:<address>`, the Host `-` while none is chosen.
    fn connect(&self) -> String {
        format!("{}:{}", self.connect_host.as_deref().unwrap_or(MISSING), self.connect_address)
    }
}

/// The user's forwards as `list` prints them, each Host by its name.
async fn listed(shard: &dyn HostShard, forwards: &[PortForwardDto]) -> Result<Vec<ListedForward>, RpcError> {
    let names = host_names(shard).await?;
    let name_of = |id: &DeviceId| (names.name(id), shard.devices().state(id) == DeviceState::Online);
    Ok(forwards
        .iter()
        .map(|forward| ListedForward {
            id: forward.id.to_string(),
            name: forward.name.clone(),
            listen_host: name_of(&forward.listen_device).0,
            listen_device: forward.listen_device.to_string(),
            listen_port: forward.listen_port,
            connect_host: forward.connect_device.as_ref().map(|device| name_of(device).0),
            connect_device: forward.connect_device.as_ref().map(ToString::to_string),
            connect_address: forward.connect_address.to_string(),
            enabled: forward.enabled,
            state: state_words(forward, &name_of),
        })
        .collect())
}

async fn list(shard: Rc<dyn HostShard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let forwards = shard.port_forwards().await.map_err(RpcError::Failed)?;
    let listed = listed(&*shard, &forwards).await?;
    if call.invocation.json {
        return print_json(&port, &listed).await;
    }
    let mut table = Table::new(["id", "name", "listen", "connect", "state"]);
    for forward in &listed {
        table.row::<Cell>([
            forward.id.as_str().into(),
            forward.name.as_str().into(),
            forward.listen().into(),
            forward.connect().into(),
            forward.state.as_str().into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

async fn add(shard: Rc<dyn HostShard>, call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let args = call.args;
    let (listen_host, listen_port) = args
        .listen
        .rsplit_once(':')
        .ok_or_else(|| RpcError::Failed("--listen takes <host>:<port>, such as Cloud:11434".into()))?;
    let listen_port: u16 = listen_port
        .parse()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| RpcError::Failed(format!("{listen_port}: a port is a number from 1 to 65535")))?;
    let (connect_host, address) = args.connect.split_once(':').ok_or_else(|| {
        RpcError::Failed("--connect takes <host>:<address>, such as \"MacBook Pro:localhost:11434\"".into())
    })?;
    let address = ForwardAddress::try_from(address.to_owned()).map_err(|error| RpcError::Failed(error.to_string()))?;
    let listen = resolve_host(&*shard, &conversation, listen_host).await?;
    let connect = resolve_host(&*shard, &conversation, connect_host).await?;
    let name = args
        .name
        .map(|name| {
            Trimmed::try_from(name)
                .ok()
                .filter(|name| !name.as_str().is_empty())
                .ok_or_else(|| RpcError::Failed("--name takes a name that is not blank".into()))
        })
        .transpose()?;
    let patch = PortForwardPatch {
        name,
        listen_device: Some(listen.device),
        listen_port: Some(listen_port),
        connect_device: Some(Some(connect.device)),
        connect_address: Some(address),
        enabled: Some(true),
    };
    patch
        .validate()
        .map_err(|report| RpcError::Failed(demi_web_api_protocol::port_forwards::refusal(&report)))?;
    let forward = shard.add_port_forward(patch).await.map_err(RpcError::Failed)?;
    let added = listed(&*shard, std::slice::from_ref(&forward))
        .await?
        .pop()
        .expect("a forward is listed");
    if call.invocation.json {
        return print_json(&port, &added).await;
    }
    let mut fields = Fields::new();
    fields
        .field("ID", added.id.as_str())
        .field("Name", added.name.as_str())
        .field("Listen", added.listen())
        .field("Connect", added.connect())
        .field("Enabled", added.enabled)
        .field("State", added.state.as_str());
    port.stdout(fields.render()).await?;
    Ok(0)
}

async fn remove(shard: Rc<dyn HostShard>, call: Call<RemoveArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let forwards = shard.port_forwards().await.map_err(RpcError::Failed)?;
    let forward = forwards
        .into_iter()
        .find(|forward| forward.name == call.args.name)
        .ok_or_else(|| {
            RpcError::Failed(format!("{}: no port forward of this name (see `demi host forward list`)", call.args.name))
        })?;
    shard.remove_port_forward(forward.id).await.map_err(RpcError::Failed)?;
    port.stdout(format!("Removed {}.\n", call.args.name)).await?;
    Ok(0)
}
