//! The product's `demi host` group (`sessions-and-targets.md` § Attached
//! hosts, `commands.md` § Demi command inputs): `list` names the Hosts the
//! calling conversation reaches, `current` its primary Host, and `shell
//! --host` runs a script on one of them as one job there. A `shell`
//! job carries its invoking job's command context, caller and commands, and its standard input and output are the calling command's:
//! the relayed pipes' far ends become the job's device, so the bytes flow
//! between the two devices through the backend's pipes, never through a
//! runner socket. Like the agent's own jobs, its admission installs the
//! user's Host directories there before it starts and tells the shard when
//! it ended.

use std::collections::BTreeMap;
use std::future::Future;
use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_backend_database::conversation_index::ExecutionTarget;
use demi_backend_remote_host::{JobEnd, JobStart, Pipe, RemoteJob};
use demi_host_interface::{
    Call, GroupBuilder, LeafBuilder, ProcessEnd, RpcError, RpcInvocation, RpcPort, TypedRpc,
};
use demi_runner_protocol::wire::Signal;
use demi_shared_types::StreamKind;
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::access::{HostRole, ReachableHost};

/// How long a stopped `shell` waits for the far job's end after asking it
/// to terminate, before it kills it.
const ABORT_GRACE: Duration = Duration::from_secs(5);

const SUMMARY: &str =
    "The hosts this conversation reaches: list them, show the primary one, run a command on another.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Lists the computers this conversation can reach, its primary Host and other devices the user attached to it, and runs a command on another one. Use it when a task involves a second machine: fetching a file from the user's laptop, checking a server, or running something where a particular device is.";

const LIST_SUMMARY: &str = "Hosts this conversation can reach with `demi host shell --host`: name, id, online, the directory shells start in; the primary one marked.";

const CURRENT_SUMMARY: &str = "The primary host: where shell_exec runs.";

const SHELL_SUMMARY: &str = "Run a shell string in another host's bash: `demi host shell --host <name|id> <script>`. The script starts where the last shell on that host ended (its home before one ran) with this command's stdin and stdout, byte-faithfully and streaming, so archives pipe cleanly both ways (`demi host shell --host ci \"tar c -C /work .\" | tar x`, `tar c . | demi host shell --host ci \"tar x -C /work\"`). stderr and the exit code pass through.";

/// The input of a leaf that takes none, such as `demi host list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoArgs {}

/// The input of `demi host shell`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShellArgs {
    /// Host name or device id from demi host list
    host: String,
    /// One quoted shell script argument; stdin is streamed to the remote
    /// program, not read as script text
    script: String,
}

/// The `host` group, whose handlers act in `shard`.
pub fn host_group(shard: Weak<dyn HostShard>) -> GroupBuilder {
    GroupBuilder::new("host", SUMMARY)
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("current", CURRENT_SUMMARY)
                .input::<NoArgs>()
                .bind(TypedRpc::new(verb(shard.clone(), current))),
        )
        .leaf(
            LeafBuilder::rpc("shell", SHELL_SUMMARY)
                .input::<ShellArgs>()
                .positionals(["script"])
                .failure_output("writes the reason to stderr and exits non-zero (127 when the host cannot run bash)")
                .bind(TypedRpc::new(verb(shard, shell))),
        )
}

/// A handler over the live shard; a call after the shard closed fails.
pub(crate) fn verb<A, F, Fut>(
    shard: Weak<dyn HostShard>,
    run: F,
) -> impl Fn(Call<A>, RpcPort) -> LocalBoxFuture<'static, Result<u8, RpcError>>
where
    A: 'static,
    F: Fn(Rc<dyn HostShard>, Call<A>, RpcPort) -> Fut + Clone + 'static,
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

/// The conversation the invoking job belongs to.
pub fn conversation_of(invocation: &RpcInvocation) -> Result<ConversationId, RpcError> {
    ConversationId::try_from(invocation.context.conversation.as_str())
        .map_err(|_| RpcError::Failed("this session has no conversation".into()))
}

/// The Hosts the calling conversation reaches.
pub async fn reachable(
    shard: &dyn HostShard,
    conversation: &ConversationId,
) -> Result<Vec<ReachableHost>, RpcError> {
    shard
        .conversation_hosts(conversation)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))
}

/// The Host of `hosts` that `wanted` names: its name, or else its device's
/// id.
pub(crate) fn named_host<'a>(
    hosts: &'a [ReachableHost],
    wanted: &str,
) -> Option<&'a ReachableHost> {
    hosts
        .iter()
        .find(|host| host.name == wanted)
        .or_else(|| hosts.iter().find(|host| host.device.as_str() == wanted))
}

fn online(shard: &dyn HostShard, host: &ReachableHost) -> &'static str {
    if shard.devices().online(&host.device) {
        "online"
    } else {
        "offline"
    }
}

async fn list(shard: Rc<dyn HostShard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let hosts = reachable(&*shard, &conversation).await?;
    if hosts.is_empty() {
        port.stdout("Cloud has not been allocated\n").await?;
        return Ok(0);
    }
    let lines: Vec<String> = hosts
        .iter()
        .map(|host| {
            let path = if host.path.is_empty() {
                "?"
            } else {
                &host.path
            };
            let role = match host.role {
                HostRole::Primary => "primary",
                HostRole::Attached => "attached",
            };
            format!(
                "{}  {}  {}  {path}  ({role})",
                host.name,
                host.device,
                online(&*shard, host)
            )
        })
        .collect();
    port.stdout(format!("{}\n", lines.join("\n"))).await?;
    Ok(0)
}

async fn current(
    shard: Rc<dyn HostShard>,
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
    let Some(device) = target.device() else {
        port.stdout(format!(
            "host: Cloud (not allocated), directory {}\n",
            target.path()
        ))
        .await?;
        return Ok(0);
    };
    let name = control
        .device(device.clone())
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?
        .map_or_else(|| device.to_string(), |device| device.name);
    let state = if shard.devices().online(device) {
        "online"
    } else {
        "offline"
    };
    let line = match &target {
        ExecutionTarget::Workspace { workspace_id, .. } => {
            let workspace = control
                .workspace(workspace_id.clone())
                .await
                .map_err(|error| RpcError::Failed(error.to_string()))?
                .map_or_else(|| workspace_id.to_string(), |workspace| workspace.name);
            format!(
                "host: workspace \"{workspace}\" — {} on device \"{name}\" ({state})\n",
                target.path()
            )
        }
        ExecutionTarget::Cloud { path, .. } | ExecutionTarget::Device { path, .. } => {
            let path = if path.is_empty() { "home" } else { path };
            format!("host: machine \"{name}\" ({device}, {state}) — {path}\n")
        }
    };
    port.stdout(line).await?;
    Ok(0)
}

async fn shell(
    shard: Rc<dyn HostShard>,
    call: Call<ShellArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let Call {
        args: ShellArgs {
            host: wanted,
            script,
        },
        invocation,
    } = call;
    if script.trim().is_empty() {
        port.stderr("usage: demi host shell --host <name|id> <script>\n")
            .await?;
        return Ok(2);
    }
    let conversation = conversation_of(&invocation)?;
    let hosts = reachable(&*shard, &conversation).await?;
    let Some(host) = named_host(&hosts, &wanted) else {
        port.stderr(format!(
            "host shell: host {wanted} is not reachable from this conversation (see `demi host list`)\n"
        ))
        .await?;
        return Ok(1);
    };
    match run_on_host(&*shard, &conversation, host, script, &invocation, &port).await {
        Ok(code) => Ok(code),
        Err(reason) => {
            port.stderr(format!("host shell: {reason}\n")).await?;
            Ok(1)
        }
    }
}

/// Runs `script` as one job on `target` through the conversation's host
/// access, with the calling command's standard input and output.
async fn run_on_host(
    shard: &dyn HostShard,
    conversation: &ConversationId,
    target: &ReachableHost,
    script: String,
    invocation: &RpcInvocation,
    port: &RpcPort,
) -> Result<u8, String> {
    let node = invocation
        .caller
        .as_ref()
        .map(|caller| caller.node.as_str())
        .ok_or("host shell runs for an agent")?;
    let commands = shard.commands().selection_of(node, conversation);
    let relayed = invocation
        .pipes
        .as_ref()
        .ok_or("cross-host execution requires a machine job")?;
    let stdout = shard
        .pipes()
        .pipe(&relayed.stdout)
        .ok_or("the command's standard output is gone")?;
    let stdin = match &relayed.stdin {
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
        .with_host(conversation, Some(&target.device), &cancel, async |host| {
            if port.is_cancelled() {
                return Ok(None);
            }
            if !host.host.online() {
                return Err(format!("host {device} is offline"));
            }
            // Dropped once the job ended, or when the call is stopped.
            let _ended = shard
                .begin_job(conversation, &target.device, host)
                .await
                .map_err(|error| error.message)?;
            let start = JobStart {
                script,
                cwd: target.path.clone(),
                env: BTreeMap::new(),
                context: invocation.context.clone(),
                caller: invocation.caller.clone(),
                commands,
                stdin: stdin.as_ref().map(Pipe::wire_ref),
                stdout: Some(stdout.wire_ref()),
            };
            let job = host
                .host
                .start_job(start)
                .await
                .map_err(|error| error.message)?;
            let ended = CancellationToken::new();
            let ending = async {
                let end = tokio::select! {
                    end = job.end() => end,
                    () = port.cancelled() => {
                        stop(&job, &stdout, stdin.as_ref()).await
                    }
                };
                ended.cancel();
                end
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
            // A caller without a pipe on its standard input types into the
            // job, until the job ends.
            let typing = async {
                if stdin.is_some() {
                    return;
                }
                tokio::select! {
                    () = forward_live_input(&job, port) => {}
                    () = ended.cancelled() => {}
                }
            };
            let (end, (), ()) = tokio::join!(ending, view, typing);
            // The end and the output are read, stopped or not: the runner
            // may remove the job's directory.
            job.release().await;
            Ok(Some(end))
        })
        .await
        .map_err(|error| error.to_string())??;
    // Stopped before it started.
    let Some(ran) = ran else {
        return Ok(130);
    };
    if target.role == HostRole::Attached
        && let Some(cwd) = ran.cwd.clone()
    {
        let changed = shard
            .control()
            .set_attached_cwd(conversation.clone(), target.device.clone(), cwd)
            .await
            .map_err(|error| error.to_string())?;
        if changed {
            shard.hosts_changed(&conversation);
        }
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
                .stderr(format!("host shell: {}{detail}\n", error.kind))
                .await;
            Ok(127)
        }
        _ if port.is_cancelled() => Ok(130),
        ProcessEnd::Exited(code) => Ok(u8::try_from(code).unwrap_or(1)),
        ProcessEnd::Signalled(_) => Ok(1),
        ProcessEnd::Lost(reason) => Err(reason),
    }
}

/// Stops the far job: asks it to terminate, and kills it when it has not
/// ended within the grace; its pipes fail with the command.
async fn stop(job: &RemoteJob, stdout: &Pipe, stdin: Option<&Pipe>) -> JobEnd {
    // A job that ended already needs no signal.
    let _ = job.kill(Signal::Terminate).await;
    stdout.fail("command aborted");
    if let Some(stdin) = stdin {
        stdin.fail("command aborted");
    }
    match tokio::time::timeout(ABORT_GRACE, job.end()).await {
        Ok(end) => end,
        Err(_) => {
            let _ = job.kill(Signal::Kill).await;
            job.end().await
        }
    }
}

/// Writes what the caller types into the job, and ends its input when the
/// caller's does.
async fn forward_live_input(job: &RemoteJob, port: &RpcPort) {
    loop {
        match port.read_live_stdin().await {
            Ok(Some(bytes)) => {
                if job.write_stdin(bytes).await.is_err() {
                    return;
                }
            }
            Ok(None) => {
                // The job may have ended already, which ends its input too.
                let _ = job.close_stdin().await;
                return;
            }
            // The call is over.
            Err(_) => return,
        }
    }
}
