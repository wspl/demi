//! The product's `demi device` group: commands on the devices a conversation
//! reaches.
//!
//! - `list` lists the user's devices, each with what it is to the calling
//!   conversation.
//! - `current` names its primary device.
//! - `shell --device` runs a script on one of them as one job there, attaching
//!   it first when the conversation does not reach it.
//! - `install-command` prints the command that installs the runner on a
//!   computer.
//! - `pair` pairs the computer whose installer shows a code and attaches it.
//! - `attach` attaches a device at once.
//! - `detach` detaches a device at once, stopping the conversation's commands
//!   there.
//! - `move` makes a device and a directory where the conversation runs, at once,
//!   and answers once it is made.
//!
//! Bringing in a device the conversation does not reach asks the user for
//! that device (`bringsDevice`), and the call that brings it in attaches it.
//! Pairing asks nothing: the agent pairs a computer it already runs commands
//! on, and the pairing attaches it.
//!
//! A `shell` job carries its invoking job's command context, caller and
//! commands. Its standard input and output are the calling command's. A script
//! read from stdin leaves the job none. The relayed pipes' far ends become the
//! job's device, so the bytes flow between the two devices through the
//! backend's pipes, never through a runner socket, and move to the two
//! runners' direct channel when they may talk directly. Like the agent's own jobs,
//! its admission installs the user's device directories there before it starts
//! and tells the shard when it ended.
//!
//! An agent pairs a computer it reaches without the user typing the code:
//!
//! 1. It runs the command `install-command` prints, the one Add Device shows,
//!    on the computer.
//! 2. It reads the pairing code the installer shows.
//! 3. It passes the code to `pair`, which claims it for the conversation's user
//!    as Add Device does and attaches the new device.
//!
//! The exchange looks like this:
//!
//! ```text
//! agent                          the computer                   backend
//!   | ssh ... '<install command>' -> runner starts, waits
//!   | <-- "Enter this pairing code in Add Device: WCXP-..."
//!   | demi device pair WCXP-... -----------------------------------> claim
//!   |                              runner gets its device token <-+
//!   | <-- "Paired as DESKTOP-PLU; attached as DESKTOP-PLU"
//! ```

use std::collections::BTreeMap;
use std::future::Future;
use std::rc::{Rc, Weak};

use demi_runner_protocol::pairs::PairRoute;
use demi_backend_database::conversation_index::{
    AttachedDeviceRecord, ExecutionTarget, RecordChange,
};
use demi_runner_client::{JobEnd, JobStart, Pipe, RemoteJob, Withdrawal};
use demi_command_declarations::MAX_STDIN_BODY;
use demi_device_interface::{
    Call, GroupBuilder, LeafBuilder, ProcessEnd, RpcError,
    BroughtDevice, RpcInvocation, RpcPort, TypedRpc,
};
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_shared_types::{
    StreamKind,
    output::{Cell, Fields, Table, warning},
};
use demi_backend_runners::claims::ClaimRefusal;
use demi_backend_runners::install::install_commands;
use demi_web_api_protocol::devices::{DeviceKind, DeviceState};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::DeviceShard;
use crate::access::{DeviceRole, ReachableDevice};
use crate::transition::ChangeRefusal;

const SUMMARY: &str =
    "The devices this conversation reaches: list them, show the primary one, run a command on another, pair a computer, attach or detach one, move this conversation to another device, forward a port from one device to another.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Devices: list, run a command on another, attach, detach, move the conversation, forward ports, pair.";

const MOVE_SUMMARY: &str = "Make another device where this conversation runs, or another directory there: `demi device move <device> <directory>`, the device by name or id from `demi device list` (`Cloud` for the user's Cloud), the directory an absolute path that exists there. The move applies at once: commands already running finish where they are, every later command runs there, and the device it leaves stays attached.";

const ATTACH_SUMMARY: &str = "Attach one of the user's devices or the Cloud to this conversation, by name or id from `demi device list`, so `demi device shell --device` reaches it; `shell` and `move` attach one themselves.";

const INSTALL_COMMAND_SUMMARY: &str = "The command that installs the runner on a computer, as Add Device shows it; run it there and pass the pairing code it shows to `demi device pair`.";

const PAIR_SUMMARY: &str = "Pair the computer whose installer shows `<code>` with the user's account and attach it to this conversation, so `demi device shell --device` reaches it.";

const DETACH_SUMMARY: &str = "Detach an attached device, by name or id from `demi device list`, at once; this conversation's commands that run there end.";

const LIST_SUMMARY: &str = "List the user's devices and the Cloud, each with what it is to this conversation: name, id, status (a stopped Cloud wakes for work), role (primary, attached, subagent for a device only a subagent works on, or none) and the directory commands start in. `demi device shell --device` and `demi device move` take any of them; one the conversation does not reach asks the user once.";

const CURRENT_SUMMARY: &str = "Show the primary device, where the shell tool's commands run: its name, id, status, project and directory.";

const SHELL_SUMMARY: &str = "Run a script in another device's bash, as ssh does: `demi device shell --device <name|id> <script>`, or without the script argument, the script on stdin, so one with quotes of its own needs no escaping (`demi device shell --device ci <<'EOF'` … `EOF`). Any device `demi device list` shows: one this conversation does not reach is attached first, which asks the user once. The script starts in that device's directory, which `demi device list` shows and no `cd` changes. With a script argument, this command's stdin and stdout are the script's, byte-faithfully and streaming, so archives pipe cleanly both ways (`demi device shell --device ci \"tar c -C /work .\" | tar x`, `tar c . | demi device shell --device ci \"tar x -C /work\"`); a script read from stdin has no stdin of its own. stderr and the exit code pass through.";

/// The input of a leaf that takes none, such as `demi device list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// The input of `demi device attach`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AttachArgs {
    /// Device name or id from demi device list
    #[expect(dead_code, reason = "the dispatch resolves it, as the call's `brought`")]
    device: String,
}

/// A system the runner installs on.
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum System {
    Linux,
    Macos,
    Windows,
}

/// The input of `demi device install-command`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InstallCommandArgs {
    /// Print only this system's command, bare, as `$(…)` takes it; without
    /// it, each system's, labelled
    system: Option<System>,
}

/// The input of `demi device pair`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PairArgs {
    /// The pairing code the computer's installer shows
    #[schemars(length(min = 1))]
    code: String,
}

/// The input of `demi device move`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveArgs {
    /// Device name or device id from demi device list; Cloud for the user's Cloud
    #[expect(dead_code, reason = "the dispatch resolves it, as the call's `brought`")]
    device: String,
    /// An absolute directory on that device, which must exist
    #[schemars(length(min = 1))]
    directory: String,
}

/// The input of `demi device detach`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DetachArgs {
    /// Device name or device id from demi device list
    device: String,
}

/// The input of `demi device shell`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShellArgs {
    /// Device name or device id from demi device list; Cloud for the user's Cloud
    #[expect(dead_code, reason = "the dispatch resolves it, as the call's `brought`")]
    device: String,
    /// One quoted shell script argument, to which stdin is streamed; without
    /// it, stdin is the script
    #[schemars(length(min = 1))]
    script: Option<String>,
}

/// The `device` group, whose handlers act in `shard`.
pub fn device_group(shard: Weak<dyn DeviceShard>) -> GroupBuilder {
    GroupBuilder::new("device", SUMMARY)
        .index_entry(ENTRY)
        .group(crate::forward_commands::forward_group(shard.clone()))
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<ListedDevice>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("current", CURRENT_SUMMARY)
                .input::<NoArgs>()
                .json_output::<CurrentDevice>()
                .bind(TypedRpc::new(verb(shard.clone(), current))),
        )
        .leaf(
            LeafBuilder::rpc("shell", SHELL_SUMMARY)
                .input::<ShellArgs>()
                .positionals(["script"])
                .brings_device("device")
                .failure_output("writes the reason to stderr and exits non-zero (127 when the device cannot run bash)")
                .bind(TypedRpc::new(verb(shard.clone(), shell))),
        )
        .leaf(
            LeafBuilder::rpc("install-command", INSTALL_COMMAND_SUMMARY)
                .input::<InstallCommandArgs>()
                .bind(TypedRpc::new(verb(shard.clone(), install_command))),
        )
        .leaf(
            LeafBuilder::rpc("pair", PAIR_SUMMARY)
                .input::<PairArgs>()
                .positionals(["code"])
                .bind(TypedRpc::new(verb(shard.clone(), pair))),
        )
        .leaf(
            LeafBuilder::rpc("attach", ATTACH_SUMMARY)
                .input::<AttachArgs>()
                .positionals(["device"])
                .brings_device("device")
                .bind(TypedRpc::new(verb(shard.clone(), attach))),
        )
        .leaf(
            LeafBuilder::rpc("detach", DETACH_SUMMARY)
                .input::<DetachArgs>()
                .positionals(["device"])
                .bind(TypedRpc::new(verb(shard.clone(), detach))),
        )
        .leaf(
            LeafBuilder::rpc("move", MOVE_SUMMARY)
                .input::<MoveArgs>()
                .positionals(["device", "directory"])
                .brings_device("device")
                .bind(TypedRpc::new(verb(shard, move_to))),
        )
}

/// A device a device name or id names, as `demi device move` and the dispatch
/// read it: the device, and the name the user knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedDevice {
    pub device: DeviceId,
    pub name: String,
    /// Whether it is the user's Cloud.
    pub cloud: bool,
}

/// The device `wanted` names, as a `bringsDevice` argument that names a device is
/// resolved, once, by the dispatch.
///
/// Of the devices it may stand for, the first in this order wins:
///
/// 1. A device of the conversation, by the name it goes by there or its id.
/// 2. A device, by its id.
/// 3. A paired device, by its name.
/// 4. The user's Cloud, by `Cloud`, which makes its device on its first use.
pub async fn resolve_device(
    shard: &dyn DeviceShard,
    conversation: &ConversationId,
    wanted: &str,
) -> Result<NamedDevice, RpcError> {
    devices_named(shard, conversation, wanted)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| RpcError::Failed(format!("{wanted}: no device of this name or id (see `demi device list`)")))
}

/// The device the dispatch resolved for the call's `bringsDevice` argument.
pub fn brought(invocation: &RpcInvocation) -> Result<&BroughtDevice, RpcError> {
    invocation
        .brought
        .as_ref()
        .ok_or_else(|| RpcError::Failed("the dispatch resolved no device for this call".into()))
}

/// Each of the user's devices `wanted` may name, in [`resolve_device`]'s order.
async fn devices_named(
    shard: &dyn DeviceShard,
    conversation: &ConversationId,
    wanted: &str,
) -> Result<Vec<NamedDevice>, RpcError> {
    let failed = |error: demi_backend_database::StorageError| RpcError::Failed(error.to_string());
    let control = shard.control();
    let mut found: Vec<NamedDevice> = Vec::new();
    let mut push = |named: NamedDevice| {
        if !found.iter().any(|each| each.device == named.device) {
            found.push(named);
        }
    };
    let cloud = control.managed_device(shard.user().clone()).await.map_err(failed)?;
    let is_cloud = |device: &DeviceId| cloud.as_ref().is_some_and(|cloud| cloud.id == *device);
    for reached in reachable(shard, conversation).await? {
        if reached.name == wanted || reached.device.as_str() == wanted {
            push(NamedDevice {
                cloud: is_cloud(&reached.device),
                name: if is_cloud(&reached.device) { "Cloud".to_owned() } else { reached.name },
                device: reached.device,
            });
        }
    }
    let paired = control.paired_devices(shard.user().clone()).await.map_err(failed)?;
    for device in paired.iter().filter(|device| device.id.as_str() == wanted || device.name == wanted) {
        push(NamedDevice {
            device: device.id.clone(),
            name: device.name.clone(),
            cloud: false,
        });
    }
    let cloud = match cloud {
        Some(cloud) if cloud.id.as_str() == wanted => Some(cloud),
        _ if wanted == "Cloud" => Some(shard.cloud_shard().cloud_device().await.map_err(failed)?),
        _ => None,
    };
    if let Some(cloud) = cloud {
        push(NamedDevice {
            device: cloud.id,
            name: "Cloud".to_owned(),
            cloud: true,
        });
    }
    Ok(found)
}

/// A handler over the live shard; a call after the shard closed fails.
pub(crate) fn verb<A, F, Fut>(
    shard: Weak<dyn DeviceShard>,
    run: F,
) -> impl Fn(Call<A>, RpcPort) -> LocalBoxFuture<'static, Result<u8, RpcError>>
where
    A: 'static,
    F: Fn(Rc<dyn DeviceShard>, Call<A>, RpcPort) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<u8, RpcError>> + 'static,
{
    move |call, port| {
        let shard = shard.upgrade();
        let run = run.clone();
        Box::pin(async move {
            let shard =
                shard.ok_or_else(|| RpcError::Failed("the backend is shutting down".into()))?;
            run(shard, call, port).await
        })
    }
}

/// The user's devices, each by the name `demi device list` gives it.
pub(crate) struct DeviceNames(Vec<demi_backend_database::devices::DeviceRecord>);

impl DeviceNames {
    /// The name of device `id`: `Cloud` for the Cloud, the id for a device the
    /// user no longer has.
    pub(crate) fn name(&self, id: &DeviceId) -> String {
        self.0
            .iter()
            .find(|device| device.id == *id)
            .map(|device| match device.kind {
                DeviceKind::Managed => "Cloud".to_owned(),
                DeviceKind::User => device.name.clone(),
            })
            .unwrap_or_else(|| id.to_string())
    }

    /// How the runners of devices `one` and `other` may talk; through the
    /// relay only when either is gone.
    fn pair_route(&self, one: &str, other: &str) -> PairRoute {
        let find = |id: &str| self.0.iter().find(|device| device.id.as_str() == id);
        find(one)
            .zip(find(other))
            .map_or(PairRoute::Relay, |(one, other)| one.pair_route(other))
    }
}

/// The user's devices, paired devices and the Cloud, for naming them.
pub(crate) async fn device_names(shard: &dyn DeviceShard) -> Result<DeviceNames, RpcError> {
    let control = shard.control();
    let failed = |error: demi_backend_database::StorageError| RpcError::Failed(error.to_string());
    let mut devices = control.paired_devices(shard.user().clone()).await.map_err(failed)?;
    devices.extend(control.managed_device(shard.user().clone()).await.map_err(failed)?);
    Ok(DeviceNames(devices))
}

pub fn conversation_of(invocation: &RpcInvocation) -> Result<ConversationId, RpcError> {
    ConversationId::try_from(invocation.context.conversation.as_str())
        .map_err(|_| RpcError::Failed("this session has no conversation".into()))
}

/// The devices the calling conversation reaches.
pub async fn reachable(
    shard: &dyn DeviceShard,
    conversation: &ConversationId,
) -> Result<Vec<ReachableDevice>, RpcError> {
    shard
        .conversation_devices(conversation)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))
}

/// The device of `devices` that `wanted` names: its name, or else its device's
/// id.
pub(crate) fn named_device<'a>(
    devices: &'a [ReachableDevice],
    wanted: &str,
) -> Option<&'a ReachableDevice> {
    devices
        .iter()
        .find(|reached| reached.name == wanted)
        .or_else(|| devices.iter().find(|reached| reached.device.as_str() == wanted))
}

/// A device as `demi device list` lists it.
#[derive(Serialize, JsonSchema)]
struct ListedDevice {
    /// The name `--device` takes: the one this conversation reaches it by,
    /// `Cloud` for the Cloud.
    name: String,
    /// Its device's id, which `--device` takes too.
    id: String,
    /// `online`, `updating`, `offline`, or `stopped` for a Cloud that wakes
    /// for work.
    status: &'static str,
    /// What it is to this conversation: `primary`, `attached`, or `subagent`
    /// for a device only a subagent works on; null for none.
    role: Option<&'static str>,
    /// Where a shell there starts; null for a device the conversation does
    /// not reach, or one whose directory is not known yet.
    directory: Option<String>,
}

fn role_word(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Primary => "primary",
        DeviceRole::Attached => "attached",
        DeviceRole::Agent => "subagent",
    }
}

/// What a device is to the user as `list` shows it: the Cloud, or a paired
/// device, with its state.
fn device_state(kind: DeviceKind, state: DeviceState) -> &'static str {
    match (kind, state) {
        (_, DeviceState::Online) => "online",
        (_, DeviceState::Updating) => "updating",
        (DeviceKind::Managed, DeviceState::Offline) => "stopped",
        (DeviceKind::User, DeviceState::Offline) => "offline",
    }
}

/// Lists the user's devices, those the conversation reaches first, in the
/// order of their roles.
async fn list(shard: Rc<dyn DeviceShard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let reached = reachable(&*shard, &conversation).await?;
    let control = shard.control();
    let failed = |error: demi_backend_database::StorageError| RpcError::Failed(error.to_string());
    let cloud = control
        .managed_device(shard.user().clone())
        .await
        .map_err(failed)?;
    let paired = control
        .paired_devices(shard.user().clone())
        .await
        .map_err(failed)?;
    if cloud.is_none() {
        let note = "the Cloud is not created yet; it is made when a conversation first runs there";
        port.stderr(warning(&call.invocation.path.join(" "), note)).await?;
    }
    let mut listed: Vec<(Option<DeviceRole>, ListedDevice)> = cloud
        .iter()
        .chain(&paired)
        .map(|device| {
            let reach = reached.iter().find(|reach| reach.device == device.id);
            let name = match (reach, device.kind) {
                (Some(reach), DeviceKind::User) => reach.name.clone(),
                (None, DeviceKind::User) => device.name.clone(),
                (_, DeviceKind::Managed) => "Cloud".to_owned(),
            };
            (
                reach.map(|reach| reach.role),
                ListedDevice {
                    name,
                    id: device.id.to_string(),
                    status: device_state(device.kind, shard.devices().state(&device.id)),
                    role: reach.map(|reach| role_word(reach.role)),
                    directory: reach.map(|reach| reach.path.clone()).filter(|path| !path.is_empty()),
                },
            )
        })
        .collect();
    listed.sort_by_key(|(role, _)| match role {
        Some(DeviceRole::Primary) => 0,
        Some(DeviceRole::Attached) => 1,
        Some(DeviceRole::Agent) => 2,
        None => 3,
    });
    let listed: Vec<ListedDevice> = listed.into_iter().map(|(_, listed)| listed).collect();
    if call.invocation.json {
        return print_json(&port, &listed).await;
    }
    let mut table = Table::new(["name", "id", "status", "role", "directory"]);
    for device in &listed {
        table.row::<Cell>([
            device.name.as_str().into(),
            device.id.as_str().into(),
            device.status.into(),
            device.role.into(),
            device.directory.as_deref().into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

/// Prints `value` as one JSON value.
pub(crate) async fn print_json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
    let text = serde_json::to_string(value).map_err(|error| RpcError::Failed(error.to_string()))?;
    port.stdout(format!("{text}\n")).await?;
    Ok(0)
}

async fn attach(
    shard: Rc<dyn DeviceShard>,
    call: Call<AttachArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let Call { invocation, .. } = call;
    let conversation = conversation_of(&invocation)?;
    let named = brought(&invocation)?;
    let device = DeviceId::try_from(named.device.clone())
        .map_err(|_| RpcError::Failed("the dispatch resolved no device".into()))?;
    let attached = attach_device(&*shard, &conversation, &device, &named.name).await?;
    port.stdout(format!(
        "Attached {} as {attached}; run commands there with `demi device shell --device {attached} <script>`.\n",
        named.name
    ))
    .await?;
    Ok(0)
}

/// Attaches the user's device `device`, named `device_name`, to the
/// conversation, and answers the name the conversation reaches it by: the
/// device's, with a suffix when a device attached before goes by it already.
async fn attach_device(
    shard: &dyn DeviceShard,
    conversation: &ConversationId,
    device: &DeviceId,
    device_name: &str,
) -> Result<String, RpcError> {
    let change = RecordChange::Attach(AttachedDeviceRecord {
        device: device.clone(),
        cwd: None,
    });
    match shard.transition(conversation, change.into()).await {
        Ok(()) => {}
        Err(ChangeRefusal::DeviceIsPrimary) => {
            return Err(RpcError::Failed(format!(
                "{device_name}: this conversation's primary device already"
            )));
        }
        Err(refusal) => return Err(RpcError::Failed(format!("{device_name}: {refusal}"))),
    }
    let devices = reachable(shard, conversation).await?;
    Ok(devices
        .iter()
        .find(|reached| reached.device == *device)
        .map_or(device_name, |reached| reached.name.as_str())
        .to_owned())
}

async fn install_command(
    shard: Rc<dyn DeviceShard>,
    call: Call<InstallCommandArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let backend = shard
        .public_url()
        .get()
        .ok_or_else(|| RpcError::Failed("the backend is not listening yet".into()))?;
    let commands = install_commands(backend.url());
    let text = match call.args.system {
        Some(System::Linux) => commands.linux,
        Some(System::Macos) => commands.macos,
        Some(System::Windows) => commands.windows,
        None => format!(
            "linux: {}\nmacos: {}\nwindows: {}",
            commands.linux, commands.macos, commands.windows
        ),
    };
    port.stdout(format!("{text}\n")).await?;
    Ok(0)
}

async fn pair(shard: Rc<dyn DeviceShard>, call: Call<PairArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let Call {
        args: PairArgs { code },
        invocation,
    } = call;
    let conversation = conversation_of(&invocation)?;
    let device = match shard.claims().claim(shard.control(), shard.user(), &code).await {
        Ok(device) => device,
        Err(ClaimRefusal::UnknownCode) => {
            return Err(RpcError::Failed(format!(
                "{code}: no computer waits under this pairing code; it expired or was never issued. The installer shows a new code when one expires: read the newest one it shows and pass that."
            )));
        }
        Err(ClaimRefusal::TooManyAttempts) => {
            return Err(RpcError::Failed(
                "Too many pairing codes were tried in the last minute; wait a minute, then pass the newest code the installer shows.".into(),
            ));
        }
        Err(ClaimRefusal::Storage(error)) => return Err(RpcError::Failed(error.to_string())),
    };
    let attached = attach_device(&*shard, &conversation, &device.id, &device.name)
        .await
        .map_err(|error| {
            RpcError::Failed(format!(
                "Paired as {}, but it could not be attached: {error}",
                device.name
            ))
        })?;
    port.stdout(format!(
        "Paired as {}; attached as {attached}.\n",
        device.name
    ))
    .await?;
    Ok(0)
}

async fn detach(
    shard: Rc<dyn DeviceShard>,
    call: Call<DetachArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let devices = reachable(&*shard, &conversation).await?;
    let wanted = &call.args.device;
    let Some(reached) = named_device(&devices, wanted) else {
        return Err(RpcError::Failed(format!(
            "{wanted}: not attached to this conversation (see `demi device list`)"
        )));
    };
    if reached.role == DeviceRole::Primary {
        return Err(RpcError::Failed(format!(
            "{}: this conversation's primary device, which cannot be detached",
            reached.name
        )));
    }
    if reached.role == DeviceRole::Agent {
        return Err(RpcError::Failed(format!(
            "{}: not attached to this conversation; a subagent works there",
            reached.name
        )));
    }
    shard
        .transition(&conversation, RecordChange::Detach(reached.device.clone()).into())
        .await
        .map_err(|refusal| RpcError::Failed(format!("{}: {refusal}", reached.name)))?;
    port.stdout(format!("{} is detached.\n", reached.name)).await?;
    Ok(0)
}

/// `demi device move`: moves the conversation to the directory on the device
/// through the one move operation, which checks the directory, and answers
/// once it is made. A command stopped before the move's commit starts moves
/// nothing.
async fn move_to(shard: Rc<dyn DeviceShard>, call: Call<MoveArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let Call {
        args: MoveArgs { directory, .. },
        invocation,
    } = call;
    let conversation = conversation_of(&invocation)?;
    let brought = brought(&invocation)?;
    let device = DeviceId::try_from(brought.device.clone())
        .map_err(|_| RpcError::Failed("the dispatch resolved no device".into()))?;
    let target = if brought.cloud {
        ConversationTarget::Cloud {
            path: Some(directory),
        }
    } else {
        ConversationTarget::Device {
            device_id: device,
            path: directory,
        }
    };
    let cancel = CancellationToken::new();
    let moving = shard.move_conversation(&conversation, target, &cancel);
    tokio::pin!(moving);
    let moved = loop {
        tokio::select! {
            moved = &mut moving => break moved,
            () = port.cancelled(), if !cancel.is_cancelled() => cancel.cancel(),
        }
    };
    match moved.map_err(|refusal| RpcError::Failed(refusal.to_string()))? {
        None => port.stdout("This conversation runs there already.\n").await?,
        Some(switched) => {
            port.stdout(format!("Moved to {} ({}).\n", brought.name, switched.switch.to.path()))
                .await?;
        }
    }
    Ok(0)
}

async fn current(
    shard: Rc<dyn DeviceShard>,
    call: Call<NoArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let control = shard.control();
    let record = shard
        .owned_conversation(&conversation)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    let target = shard
        .resolve_target(&record)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    let directory = Some(target.path().to_owned()).filter(|path| !path.is_empty());
    let shown = match target.device() {
        None => CurrentDevice {
            name: "Cloud".to_owned(),
            id: None,
            status: "not allocated",
            project: None,
            directory,
        },
        Some(device) => {
            let name = control
                .device(device.clone())
                .await
                .map_err(|error| RpcError::Failed(error.to_string()))?
                .map_or_else(|| device.to_string(), |device| device.name);
            let project = match &target {
                ExecutionTarget::Project { project_id, .. } => Some(
                    control
                        .project(project_id.clone())
                        .await
                        .map_err(|error| RpcError::Failed(error.to_string()))?
                        .map_or_else(|| project_id.to_string(), |project| project.name),
                ),
                ExecutionTarget::Cloud { .. } | ExecutionTarget::Device { .. } => None,
            };
            CurrentDevice {
                name,
                id: Some(device.to_string()),
                status: if shard.devices().online(device) { "online" } else { "offline" },
                project,
                directory,
            }
        }
    };
    if call.invocation.json {
        return print_json(&port, &shown).await;
    }
    let mut fields = Fields::new();
    fields
        .field("Name", shown.name.as_str())
        .field("ID", shown.id.as_deref())
        .field("Status", shown.status)
        .field("Project", shown.project.as_deref())
        .field("Directory", shown.directory.as_deref());
    port.stdout(fields.render()).await?;
    Ok(0)
}

/// `demi device current`: the primary device.
#[derive(Serialize, JsonSchema)]
struct CurrentDevice {
    name: String,
    /// Its device's id; null for a Cloud not allocated yet.
    id: Option<String>,
    /// `online`, `offline` or `not allocated`.
    status: &'static str,
    /// The project the conversation is in; null for none.
    project: Option<String>,
    /// Where its commands start; null for the home directory.
    directory: Option<String>,
}

async fn shell(
    shard: Rc<dyn DeviceShard>,
    call: Call<ShellArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let Call {
        args: ShellArgs { script, .. },
        invocation,
    } = call;
    // Without a script argument stdin is the script, as `ssh host bash -s`
    // takes it, and the job gets no stdin of its own (`crates/command-declarations/src/lib.rs`). It is not the declaration's stdin field: that one,
    // given as its positional, would leave stdin unread, where a script
    // argument streams it to the far program.
    let (script, streams_stdin) = match script {
        Some(script) => (script, true),
        None => (script_from_stdin(&port).await?, false),
    };
    let conversation = conversation_of(&invocation)?;
    let wanted = brought(&invocation)?;
    // A device only a subagent works on is not the conversation's own: the
    // dispatch asked for it as for any other, and attaching it keeps the
    // answer.
    let reached = |devices: &[ReachableDevice]| {
        devices
            .iter()
            .position(|each| each.device.as_str() == wanted.device && each.role != DeviceRole::Agent)
    };
    let mut devices = reachable(&*shard, &conversation).await?;
    if reached(&devices).is_none() {
        let device = DeviceId::try_from(wanted.device.clone())
            .map_err(|_| RpcError::Failed("the dispatch resolved no device".into()))?;
        attach_device(&*shard, &conversation, &device, &wanted.name).await?;
        devices = reachable(&*shard, &conversation).await?;
    }
    let Some(target) = reached(&devices).map(|at| &devices[at]) else {
        return Err(RpcError::Failed(format!(
            "{}: not reachable from this conversation (see `demi device list`)",
            wanted.name
        )));
    };
    run_on_device(
        &*shard,
        &conversation,
        target,
        script,
        streams_stdin,
        &invocation,
        &port,
    )
    .await
    .map_err(RpcError::Failed)
}

/// The whole of the call's stdin, as a script, up to the limit of a stdin
/// body; reading stops at the limit, so an endless producer is not drained.
async fn script_from_stdin(port: &RpcPort) -> Result<String, RpcError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = port.read_stdin().await? {
        if bytes.len() + chunk.len() > MAX_STDIN_BODY {
            return Err(RpcError::Failed(
                "the script on stdin exceeds 1 MiB: copy a long one to the device as a file and run that".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err(RpcError::Failed(
            "no script: give it as an argument or on stdin, as a quoted heredoc".into(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| RpcError::Failed("the script on stdin is not UTF-8 text".into()))
}

/// Runs `script` as one job on `target` through the conversation's device
/// access, with the calling command's standard output, and its standard
/// input when `streams_stdin` says the script did not come from there.
async fn run_on_device(
    shard: &dyn DeviceShard,
    conversation: &ConversationId,
    target: &ReachableDevice,
    script: String,
    streams_stdin: bool,
    invocation: &RpcInvocation,
    port: &RpcPort,
) -> Result<u8, String> {
    let node = invocation
        .caller
        .as_ref()
        .map(|caller| caller.node.as_str())
        .ok_or("device shell runs for an agent")?;
    let commands = shard.commands().selection_of(node, conversation);
    let relayed = invocation
        .pipes
        .as_ref()
        .ok_or("cross-device execution requires a machine job")?;
    let stdout = shard
        .pipes()
        .pipe(&relayed.stdout)
        .ok_or("the command's standard output is gone")?;
    let stdin = match relayed.stdin.as_ref().filter(|_| streams_stdin) {
        Some(id) => Some(
            shard
                .pipes()
                .pipe(id)
                .ok_or("the command's standard input is gone")?,
        ),
        None => None,
    };
    let device = target.device.as_str();
    if let Some(stdin) = &stdin {
        stdin.sink_to(device).map_err(|error| error.to_string())?;
    }
    stdout
        .source_from(device)
        .map_err(|error| error.to_string())?;
    // Between two devices whose runners may talk directly, the bytes move to
    // their direct channel once it is up.
    if invocation.device != device {
        let names = device_names(shard).await.map_err(|error| error.to_string())?;
        let route = names.pair_route(&invocation.device, device);
        if route != PairRoute::Relay {
            stdout.may_go_direct(route);
            if let Some(stdin) = &stdin {
                stdin.may_go_direct(route);
            }
        }
    }
    let withdrawal = Rc::new(Withdrawal::default());
    // The admission's waits end with the call.
    let cancel = CancellationToken::new();
    shard.tasks().spawn_local({
        let port = port.clone();
        let cancel = cancel.clone();
        async move {
            port.cancelled().await;
            cancel.cancel();
        }
    });
    let ran = shard
        .with_device(conversation, Some(&target.device), &cancel, async |admitted| {
            if port.is_cancelled() {
                return Ok(None);
            }
            if !admitted.device.online() {
                // The primary device, admitted while its runner is away,
                // answers with its offline error as an attached one is
                // refused with it.
                return Err(match shard.control().device(target.device.clone()).await {
                    Ok(Some(record)) => shard.offline(&record).message,
                    _ => format!("{} is offline", target.name),
                });
            }
            // The detach of its device ends it: the conversation holds it as
            // it holds an agent's own job there.
            let _held = shard
                .conversations()
                .slot(conversation)
                .hold_job(target.device.clone(), true, None, withdrawal.clone());
            // A detach of the device that committed since the admission, or
            // that comes while the directories install, ends the call here,
            // before the job starts.
            shard
                .reaches(conversation, &target.device, &withdrawal)
                .await
                .map_err(|error| error.message)?;
            shard
                .install_directories(conversation, &target.device, admitted)
                .await
                .map_err(|error| error.message)?;
            shard
                .reaches(conversation, &target.device, &withdrawal)
                .await
                .map_err(|error| error.message)?;
            let start = JobStart {
                id: demi_runner_client::RemoteDevice::job_id(),
                script,
                cwd: target.path.clone(),
                env: BTreeMap::new(),
                context: invocation.context.clone(),
                // No result attaches the far job's media: its stdout is the
                // calling command's.
                viewable: None,
                caller: invocation.caller.clone(),
                commands,
                stdin: stdin.as_ref().map(Pipe::wire_ref),
                stdout: Some(stdout.wire_ref()),
                // The far job stops as the call that started it does.
                stop: cancel.clone(),
            };
            let job = admitted
                .device
                .start_job(start)
                .await
                .map_err(|error| error.message)?;
            let ending = async {
                tokio::select! {
                    end = job.end() => end,
                    () = port.cancelled() => {
                        stop(&job, &stdout, stdin.as_ref()).await
                    }
                    reason = withdrawal.asked() => {
                        stdout.fail(&reason);
                        if let Some(stdin) = &stdin {
                            stdin.fail(&reason);
                        }
                        job.stop_unless_away(&reason).await
                    }
                }
            };
            // The far job's view carries the first bytes of its standard
            // error; its standard output is the pipe. A message without bytes
            // says only that the stream grows.
            let view = async {
                while let Some(output) = job.next_output().await {
                    if output.stream == StreamKind::Stderr && !output.bytes.is_empty() {
                        // A caller that went away reads nothing more.
                        let _ = port.stderr(output.bytes).await;
                    }
                }
            };
            let (end, ()) = tokio::join!(ending, view);
            // The end and the output are read, stopped or not: the runner
            // may remove the job's directory.
            job.release().await;
            Ok(Some(end))
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|ran| ran);
    // A far job that never started never takes its pipes: they end with
    // why, or the calling command would read them until their arrival
    // deadline.
    let ran = ran.inspect_err(|reason| {
        stdout.fail(reason);
        if let Some(stdin) = &stdin {
            stdin.fail(reason);
        }
    })?;
    // Stopped before it started.
    let Some(ran) = ran else {
        return Ok(130);
    };
    // Its device was detached: the caller reads why as the call's failure.
    if let Some(reason) = withdrawal.reason() {
        return Err(reason);
    }
    match ran.status {
        ProcessEnd::NotStarted(error) => {
            let detail = error
                .detail
                .map(|detail| format!(" — {detail}"))
                .unwrap_or_default();
            // Bash could not run the script at all: 127, as a shell answers.
            // A caller that went away reads nothing more.
            let _ = port
                .stderr(format!(
                    "{}: {}{detail}\n",
                    invocation.path.join(" "),
                    error.kind
                ))
                .await;
            Ok(127)
        }
        _ if port.is_cancelled() => Ok(130),
        ProcessEnd::Exited(code) => Ok(u8::try_from(code).unwrap_or(1)),
        ProcessEnd::Signalled(_) => Ok(1),
        ProcessEnd::Lost(reason) => Err(reason),
    }
}

/// Stops the far job as a stop does (`RemoteJob::stop`), except that a runner
/// that is away ends it as lost: the call has to end, and its caller reads
/// why. Its pipes fail with the command.
async fn stop(job: &RemoteJob, stdout: &Pipe, stdin: Option<&Pipe>) -> JobEnd {
    stdout.fail("command aborted");
    if let Some(stdin) = stdin {
        stdin.fail("command aborted");
    }
    job.stop_unless_away("the device's runner went away while the command ran").await
}

