//! `demi host forward` (`port-forwarding.md` § From a conversation): lists,
//! adds and removes the user's port forwards. `list` needs no permission;
//! `add` and `remove` need Manage Port Forwards (`host.forward`), since a
//! forward outlives the conversation and lets any program on its listening
//! Host reach the address it connects to.

use std::rc::{Rc, Weak};

use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_runner_protocol::forward::ForwardRunState;
use demi_web_api_protocol::devices::{DeviceKind, DeviceState};
use demi_web_api_protocol::ids::DeviceId;
use demi_web_api_protocol::port_forwards::{ForwardAddress, PortForwardDto, PortForwardPatch};
use demi_web_api_protocol::text::Trimmed;
use garde::Validate;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::HostShard;
use crate::host_commands::{NoArgs, conversation_of, resolve_host, verb};

/// The category `add` and `remove` need.
pub const MANAGE: &str = "host.forward";
/// The category's action, which completes "Allow this conversation to …".
pub const MANAGE_ACTION: &str = "manage port forwards";
/// What a grant of the category allows.
pub const MANAGE_DESCRIPTION: &str = "Add and remove the user's port forwards. A forward outlives this conversation, and while it runs any program on the Host it listens on can reach the address it connects to.";

const SUMMARY: &str = "The user's port forwards between their Hosts, as ssh -L makes one: list, add and remove them.";

const LIST_SUMMARY: &str = "The user's port forwards: name, where it listens, where it connects, and what it is doing.";

const ADD_SUMMARY: &str = "Add a port forward: a program on the --listen Host that connects to 127.0.0.1:<port> reaches <address> as the --connect Host reaches it, such as `demi host forward add --listen Cloud:11434 --connect \"MacBook Pro:localhost:11434\" --name ollama`. It runs whenever its listening Host's runner runs, until it is removed.";

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
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<AddArgs>()
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

async fn list(shard: Rc<dyn HostShard>, _: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let forwards = shard.port_forwards().await.map_err(RpcError::Failed)?;
    if forwards.is_empty() {
        port.stdout("No port forwards.\n".to_owned()).await?;
        return Ok(0);
    }
    let control = shard.control();
    let failed = |error: demi_backend_database::StorageError| RpcError::Failed(error.to_string());
    let mut devices = control.paired_devices(shard.user().clone()).await.map_err(failed)?;
    devices.extend(control.managed_device(shard.user().clone()).await.map_err(failed)?);
    let name_of = |id: &DeviceId| {
        let name = devices
            .iter()
            .find(|device| device.id == *id)
            .map(|device| match device.kind {
                DeviceKind::Managed => "Cloud".to_owned(),
                DeviceKind::User => device.name.clone(),
            })
            .unwrap_or_else(|| id.to_string());
        (name, shard.devices().state(id) == DeviceState::Online)
    };
    let mut lines = Vec::new();
    for forward in &forwards {
        let listen = format!("{}:{}", name_of(&forward.listen_device).0, forward.listen_port);
        let connect = match &forward.connect_device {
            Some(device) => format!("{}:{}", name_of(device).0, forward.connect_address),
            None => format!("(no Host):{}", forward.connect_address),
        };
        lines.push(format!("{}  {listen} → {connect}  {}", forward.name, state_words(forward, &name_of)));
    }
    port.stdout(format!("{}\n", lines.join("\n"))).await?;
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
    port.stdout(format!(
        "Added {}: a program on {} that connects to 127.0.0.1:{} reaches {} on {}.\n",
        forward.name, listen.name, forward.listen_port, forward.connect_address, connect.name
    ))
    .await?;
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
