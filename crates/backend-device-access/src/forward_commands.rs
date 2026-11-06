//! `demi device forward`: lists, adds and removes the user's port forwards.
//!
//! `list` needs nothing. `add` and `remove` act only on a forward whose devices
//! the conversation reaches, since a forward outlives the conversation and
//! lets any program on its listening device reach the address it connects to:
//! the user allowed each of those devices when it was attached.

use std::rc::{Rc, Weak};

use demi_device_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_runner_protocol::forward::ForwardRunState;
use demi_web_api_protocol::devices::DeviceState;
use demi_web_api_protocol::ids::DeviceId;
use demi_web_api_protocol::port_forwards::{ForwardAddress, PortForwardDto, PortForwardPatch};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::output::{Cell, Fields, MISSING, Table};

use crate::DeviceShard;
use crate::device_commands::{NoArgs, conversation_of, device_names, print_json, reachable, resolve_device, verb};

const SUMMARY: &str = "The user's port forwards between their devices, as ssh -L makes one: list, add and remove them.";

const LIST_SUMMARY: &str = "The user's port forwards: id, where it listens (<device>:<port>), where it connects (<device>:<address>, - for a device not chosen yet), and what it is doing.";

const ADD_SUMMARY: &str = "Add a port forward: a program on the --listen device that connects to 127.0.0.1:<port> reaches <address> as the --connect device reaches it, such as `demi device forward add --listen Cloud:11434 --connect \"MacBook Pro:localhost:11434\"`. It runs whenever its listening device's runner runs, until it is removed. Prints the new forward as list shows it, as labelled lines.";

const REMOVE_SUMMARY: &str = "Remove a port forward, by its id or where it listens: its listener closes and its connections end.";

/// The input of `demi device forward add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// <device>:<port>: the device that listens, by name or id from demi device
    /// list (Cloud for the user's Cloud), and the port it listens on, on
    /// 127.0.0.1
    #[schemars(length(min = 3))]
    listen: String,
    /// <device>:<address>: the device that connects, and the host:port it
    /// connects to, such as "MacBook Pro:localhost:11434"
    #[schemars(length(min = 3))]
    connect: String,
}

/// The input of `demi device forward remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveArgs {
    /// The forward's id from demi device forward list, or where it listens,
    /// <device>:<port>, when one forward listens there
    #[schemars(length(min = 1))]
    forward: String,
}

/// The `forward` group, whose handlers act in `shard`.
pub fn forward_group(shard: Weak<dyn DeviceShard>) -> GroupBuilder {
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
                .bind(TypedRpc::new(verb(shard.clone(), add))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<RemoveArgs>()
                .positionals(["forward"])
                .bind(TypedRpc::new(verb(shard, remove))),
        )
}

/// What a forward does now, in the words of its row in Settings › Port
/// Forwarding (`crates/runner-protocol/src/forward.rs`), from what its
/// Listen On runner announced and whether its devices are online.
fn state_words(forward: &PortForwardDto, name_of: &dyn Fn(&DeviceId) -> (String, bool)) -> String {
    if !forward.enabled {
        return "Off".to_owned();
    }
    let Some(connect) = &forward.connect_device else {
        return "Choose a device to connect to.".to_owned();
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
    /// The device that listens, by its name: `Cloud` for the Cloud.
    listen_device: String,
    listen_device_id: String,
    /// The port it listens on, on 127.0.0.1.
    listen_port: u16,
    /// The device that connects; null while none is chosen.
    connect_device: Option<String>,
    connect_device_id: Option<String>,
    /// The host:port it connects to, as that device reaches it.
    connect_address: String,
    enabled: bool,
    /// What it is doing, in the words of its row in Settings.
    state: String,
}

impl ListedForward {
    /// `<device>:<port>`.
    fn listen(&self) -> String {
        format!("{}:{}", self.listen_device, self.listen_port)
    }

    /// `<device>:<address>`, the device `-` while none is chosen.
    fn connect(&self) -> String {
        format!("{}:{}", self.connect_device.as_deref().unwrap_or(MISSING), self.connect_address)
    }
}

/// The user's forwards as `list` prints them, each device by its name.
async fn listed(shard: &dyn DeviceShard, forwards: &[PortForwardDto]) -> Result<Vec<ListedForward>, RpcError> {
    let names = device_names(shard).await?;
    let name_of = |id: &DeviceId| (names.name(id), shard.devices().state(id) == DeviceState::Online);
    Ok(forwards
        .iter()
        .map(|forward| ListedForward {
            id: forward.id.to_string(),
            listen_device: name_of(&forward.listen_device).0,
            listen_device_id: forward.listen_device.to_string(),
            listen_port: forward.listen_port,
            connect_device: forward.connect_device.as_ref().map(|device| name_of(device).0),
            connect_device_id: forward.connect_device.as_ref().map(ToString::to_string),
            connect_address: forward.connect_address.to_string(),
            enabled: forward.enabled,
            state: state_words(forward, &name_of),
        })
        .collect())
}

async fn list(shard: Rc<dyn DeviceShard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let forwards = shard.port_forwards().await.map_err(RpcError::Failed)?;
    let listed = listed(&*shard, &forwards).await?;
    if call.invocation.json {
        return print_json(&port, &listed).await;
    }
    let mut table = Table::new(["id", "listen", "connect", "state"]);
    for forward in &listed {
        table.row::<Cell>([
            forward.id.as_str().into(),
            forward.listen().into(),
            forward.connect().into(),
            forward.state.as_str().into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

/// Refuses a forward with a device among `ends` the conversation does not
/// reach, naming the first such and how to attach it.
async fn ends_reached<'a>(
    shard: &dyn DeviceShard,
    conversation: &demi_web_api_protocol::ids::ConversationId,
    ends: impl IntoIterator<Item = &'a DeviceId>,
) -> Result<(), RpcError> {
    let devices = reachable(shard, conversation).await?;
    let names = device_names(shard).await?;
    for end in ends {
        if !devices.iter().any(|reached| reached.device == *end) {
            let name = names.name(end);
            return Err(RpcError::Failed(format!(
                "{name}: this conversation does not reach it; `demi device attach \"{name}\"` attaches it, which asks the user first"
            )));
        }
    }
    Ok(())
}

async fn add(shard: Rc<dyn DeviceShard>, call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let args = call.args;
    let (listen_name, listen_port) = args
        .listen
        .rsplit_once(':')
        .ok_or_else(|| RpcError::Failed("--listen takes <device>:<port>, such as Cloud:11434".into()))?;
    let listen_port: u16 = listen_port
        .parse()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| RpcError::Failed(format!("{listen_port}: a port is a number from 1 to 65535")))?;
    let (connect_name, address) = args.connect.split_once(':').ok_or_else(|| {
        RpcError::Failed("--connect takes <device>:<address>, such as \"MacBook Pro:localhost:11434\"".into())
    })?;
    let address = ForwardAddress::try_from(address.to_owned()).map_err(|error| RpcError::Failed(error.to_string()))?;
    let listen = resolve_device(&*shard, &conversation, listen_name).await?;
    let connect = resolve_device(&*shard, &conversation, connect_name).await?;
    ends_reached(&*shard, &conversation, [&listen.device, &connect.device]).await?;
    let patch = PortForwardPatch {
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
        .field("Listen", added.listen())
        .field("Connect", added.connect())
        .field("Enabled", added.enabled)
        .field("State", added.state.as_str());
    port.stdout(fields.render()).await?;
    Ok(0)
}

async fn remove(shard: Rc<dyn DeviceShard>, call: Call<RemoveArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let named = call.args.forward;
    let forwards = shard.port_forwards().await.map_err(RpcError::Failed)?;
    let mut found: Vec<PortForwardDto> = forwards.iter().filter(|forward| forward.id.as_str() == named).cloned().collect();
    if found.is_empty()
        && let Some((listen_name, listen_port)) = named.rsplit_once(':')
        && let Ok(listen_port) = listen_port.parse::<u16>()
    {
        let conversation = conversation_of(&call.invocation)?;
        // A device the user has none of names no forward, which the answer
        // below says of the whole argument.
        if let Ok(listen) = resolve_device(&*shard, &conversation, listen_name).await {
            found = forwards
                .into_iter()
                .filter(|forward| forward.listen_device == listen.device && forward.listen_port == listen_port)
                .collect();
        }
    }
    let forward = match found.len() {
        0 => {
            return Err(RpcError::Failed(format!(
                "{named}: no port forward has this id or listens there (see `demi device forward list`)"
            )));
        }
        1 => found.remove(0),
        _ => {
            let ids: Vec<&str> = found.iter().map(|forward| forward.id.as_str()).collect();
            return Err(RpcError::Failed(format!(
                "{named}: {} port forwards listen there; remove one by its id: {}",
                ids.len(),
                ids.join(", ")
            )));
        }
    };
    let conversation = conversation_of(&call.invocation)?;
    ends_reached(
        &*shard,
        &conversation,
        std::iter::once(&forward.listen_device).chain(forward.connect_device.as_ref()),
    )
    .await?;
    let removed = listed(&*shard, std::slice::from_ref(&forward))
        .await?
        .pop()
        .expect("a forward is listed");
    shard.remove_port_forward(forward.id).await.map_err(RpcError::Failed)?;
    port.stdout(format!("Removed {} → {}.\n", removed.listen(), removed.connect())).await?;
    Ok(0)
}
