//! The `demi agent` command group (`subagents.md` § Model-facing surface):
//! its declarations, and the handlers that run each call against the live
//! tree of the invoking job's conversation. Lifecycle (`spawn`, `abort`,
//! `resume`) acts on the caller's own children; communication and reads
//! reach any live agent of the tree.

use std::rc::{Rc, Weak};

use demi_core::{NodeId, is_blank, trim};
use demi_shell::{
    Call, CommandSet, GroupBuilder, LeafBuilder, RegisterError, RpcError, RpcPort, TypedRpc,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    AgentServer,
    tree::{AgentSnapshot, StartInput, Tree, TreeEntry},
};
use crate::{AgentHarness, Profile};

/// The one description of a spawn's brief (`subagents.md` § Command help).
const SPAWN_PROMPT: &str = "The child's first user message and only task brief. The child starts with an empty transcript and cannot see this conversation: do not refer to prior turns, and do not paste this conversation or the product user's message unchanged. Include the goal for this child, applicable decisions and constraints, whether to edit or only report, how to verify, and every concrete identifier it needs (paths, ids, error text, commands already tried and their key results). State the exact shape of the last assistant text it should return.";

const SPAWN_SUMMARY: &str = "Start a child agent session and return its id immediately after creation. The child runs independently of this command. Completion arrives as a message to the parent, waking it when idle. When you have no independent work left, end your turn and let the completion message wake you. Do not poll agent list/show or schedule timed yield calls to wait for children. Use agent send to communicate and agent abort to stop it. Children can spawn children of their own.";

const SEND_SUMMARY: &str = "Deliver information to any live agent in the tree, or parent. A busy recipient incorporates it through internal steering; an idle recipient wakes. Returns after durable acceptance, without waiting for an answer. Use for interim information, questions, or blockers; your final answer is delivered automatically. Archived recipients must be reopened by their parent with resume.";

const ABORT_SUMMARY: &str = "Abort one of your own running children and its whole subtree. Siblings are untouched; only the spawning session may abort a child.";

const RESUME_SUMMARY: &str = "Revive one of your own archived children with a new user message on its preserved transcript. Return its id immediately after accepting the message; completion is delivered separately to the parent. Use agent send to communicate and agent abort to stop it. Archived ids are in agent list.";

const LIST_SUMMARY: &str = "Render the whole session tree from the root down, marking your own position. Live agents show phase, ages, execution, and activity; each node's archived (finished, revivable by its parent) children render beneath it. Every age is relative to now. A read, not a wait — not for polling loops.";

const SHOW_SUMMARY: &str = "Bounded snapshot of any live agent in the tree (root excluded): execution state, recent tool titles with durations, last assistant text. Every duration is relative to now — use the ages to tell motion from stall. Omits tool outputs, file contents, and older turns. A read, not a wait — not for polling loops.";

/// The input of `demi agent spawn`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct SpawnArgs {
    prompt: String,
    /// Stable id for this creation or resume request. Supply the same id and
    /// arguments to retry safely after an uncertain response; otherwise a new
    /// id is generated.
    #[schemars(length(min = 1, max = 128))]
    request_id: Option<String>,
    profile: Option<String>,
    /// Short UI title distinguishing concurrent children.
    description: Option<String>,
    /// Forbid this child from spawning subagents of its own; it can still
    /// send, list, and show.
    no_subagents: Option<bool>,
}

/// The input of `demi agent send`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SendArgs {
    /// Target agent id from the tree, or "parent" for the session that
    /// spawned this one
    id: String,
    /// Message body.
    message: String,
}

/// The input of `demi agent abort`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AbortArgs {
    /// subagentId from spawn stdout
    id: String,
}

/// The input of `demi agent resume`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ResumeArgs {
    /// subagentId of an archived child
    id: String,
    /// Stable id for this creation or resume request. Supply the same id and
    /// arguments to retry safely after an uncertain response; otherwise a new
    /// id is generated.
    #[schemars(length(min = 1, max = 128))]
    request_id: Option<String>,
    /// The reviving user message.
    message: String,
}

/// The input of `demi agent list`, which takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {}

/// The input of `demi agent show`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShowArgs {
    /// Agent id from the tree
    id: String,
}

/// `demi agent spawn --json` and `resume --json`.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Started {
    subagent_id: NodeId,
}

/// `demi agent send --json`.
#[derive(Serialize, JsonSchema)]
struct Sent {
    id: NodeId,
    accepted: bool,
}

/// `demi agent abort --json`.
#[derive(Serialize, JsonSchema)]
struct Aborted {
    id: NodeId,
    aborted: bool,
}

/// `demi agent list --json`.
#[derive(Serialize, JsonSchema)]
struct Listing {
    tree: Vec<TreeEntry>,
}

/// `demi agent show --json`.
#[derive(Serialize, JsonSchema)]
struct Shown {
    agent: AgentSnapshot,
}

/// A node's commands: its harness's, with the `demi agent` group grafted
/// under a `demi` group, replacing any `agent` child the harness declared,
/// or under a new `demi` root.
pub(crate) fn with_agent_group<H: AgentHarness>(
    harness_commands: &CommandSet,
    server: &Rc<AgentServer<H>>,
    can_spawn: bool,
    profiles: &[Profile],
) -> Result<CommandSet, RegisterError> {
    let mut commands = harness_commands.clone();
    let group = agent_group(Rc::downgrade(server), can_spawn, profiles);
    let has_demi = commands.declarations().any(|root| root.name() == "demi");
    if has_demi {
        commands.graft(&["demi"], group)?;
    } else {
        commands
            .register(GroupBuilder::new("demi", "Demi agent runtime commands.").group(group))?;
    }
    Ok(commands)
}

/// The `agent` group of a node: every verb, or communication and reads only
/// when the node may not spawn.
fn agent_group<H: AgentHarness>(
    server: Weak<AgentServer<H>>,
    can_spawn: bool,
    profiles: &[Profile],
) -> GroupBuilder {
    let summary = if can_spawn {
        "Agent tree: spawn and manage your own children; send, list and show any live agent."
    } else {
        "Agent tree communication: this session may not spawn subagents; send, list and show any live agent."
    };
    let names: Vec<&str> = profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect();
    let available = if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    };
    let mut group = GroupBuilder::new("agent", summary);
    if can_spawn {
        group = group.leaf(
            LeafBuilder::rpc("spawn", SPAWN_SUMMARY)
                .input::<SpawnArgs>()
                .describe("prompt", SPAWN_PROMPT)
                .describe(
                    "profile",
                    format!("Named subagent profile configured at harness assembly; omit to inherit the parent's model, prompt, Host and commands. Available: {available}."),
                )
                .stdin_field("prompt")
                .success_output("stdout is \"subagentId: <id>\"; creation succeeded, not necessarily execution")
                .failure_output("non-zero exit with the creation failure reason on stderr")
                .json_output::<Started>()
                .bind(TypedRpc::new(verb(server.clone(), spawn))),
        );
    }
    group = group.leaf(
        LeafBuilder::rpc("send", SEND_SUMMARY)
            .input::<SendArgs>()
            .positionals(["id"])
            .stdin_field("message")
            .json_output::<Sent>()
            .bind(TypedRpc::new(verb(server.clone(), send))),
    );
    if can_spawn {
        group = group
            .leaf(
                LeafBuilder::rpc("abort", ABORT_SUMMARY)
                    .input::<AbortArgs>()
                    .positionals(["id"])
                    .json_output::<Aborted>()
                    .bind(TypedRpc::new(verb(server.clone(), abort))),
            )
            .leaf(
                LeafBuilder::rpc("resume", RESUME_SUMMARY)
                    .input::<ResumeArgs>()
                    .positionals(["id"])
                    .stdin_field("message")
                    .json_output::<Started>()
                    .bind(TypedRpc::new(verb(server.clone(), resume))),
            );
    }
    group
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .json_output::<Listing>()
                .bind(TypedRpc::new(verb(server.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<ShowArgs>()
                .positionals(["id"])
                .json_output::<Shown>()
                .bind(TypedRpc::new(verb(server.clone(), show))),
        )
}

/// A verb's call against the live tree of the invoking job's conversation,
/// on behalf of the job's node.
struct Invoked<H: AgentHarness, A> {
    tree: Rc<Tree<H>>,
    server: Rc<AgentServer<H>>,
    caller: NodeId,
    json: bool,
    args: A,
}

/// Binds `run` to the tree and node a call names. A call from a job whose
/// tree is not live fails.
fn verb<H, A, F, Fut>(
    server: Weak<AgentServer<H>>,
    run: F,
) -> impl Fn(Call<A>, RpcPort) -> futures_util::future::LocalBoxFuture<'static, Result<u8, RpcError>>
where
    H: AgentHarness,
    A: 'static,
    F: Fn(Invoked<H, A>, RpcPort) -> Fut + Copy + 'static,
    Fut: std::future::Future<Output = Result<u8, RpcError>> + 'static,
{
    move |call: Call<A>, port: RpcPort| {
        let server = server.clone();
        Box::pin(async move {
            let server = server
                .upgrade()
                .ok_or_else(|| RpcError::Failed("the agent server is gone".into()))?;
            let context = &call.invocation.context;
            let root = NodeId::try_from(context.conversation.as_str())
                .map_err(|error| RpcError::Failed(error.to_string()))?;
            let caller = context
                .caller
                .node()
                .and_then(|node| NodeId::try_from(node).ok())
                .ok_or_else(|| RpcError::Failed("demi agent runs only in an agent's job".into()))?;
            let tree = server
                .tree(&root)
                .ok_or_else(|| RpcError::Failed(format!("the conversation {root} is not open")))?;
            let invoked = Invoked {
                tree,
                server,
                caller,
                json: call.invocation.json,
                args: call.args,
            };
            run(invoked, port).await
        })
    }
}

async fn spawn<H: AgentHarness>(
    call: Invoked<H, SpawnArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let prompt = trim(&call.args.prompt);
    if prompt.is_empty() {
        return fail(&port, "spawn", "prompt must not be empty").await;
    }
    let input = StartInput::Spawn {
        prompt: prompt.to_owned(),
        profile_name: call.args.profile,
        description: call.args.description.unwrap_or_default(),
        is_spawn_forbidden: call.args.no_subagents.unwrap_or(false),
    };
    let request = call.args.request_id.unwrap_or_else(|| call.server.new_id());
    match call.tree.start(&call.caller, input, request, &port).await {
        Ok(child) => started(&port, call.json, child).await,
        Err(error) => fail(&port, "spawn", &error).await,
    }
}

async fn resume<H: AgentHarness>(
    call: Invoked<H, ResumeArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let message = trim(&call.args.message);
    if message.is_empty() {
        return fail(&port, "resume", "message must not be empty").await;
    }
    let id = match NodeId::try_from(call.args.id.as_str()) {
        Ok(id) => id,
        Err(error) => return fail(&port, "resume", &error.to_string()).await,
    };
    let input = StartInput::Resume {
        id,
        message: message.to_owned(),
    };
    let request = call.args.request_id.unwrap_or_else(|| call.server.new_id());
    match call.tree.start(&call.caller, input, request, &port).await {
        Ok(child) => started(&port, call.json, child).await,
        Err(error) => fail(&port, "resume", &error).await,
    }
}

async fn send<H: AgentHarness>(call: Invoked<H, SendArgs>, port: RpcPort) -> Result<u8, RpcError> {
    if is_blank(&call.args.message) {
        return fail(&port, "send", "message must not be empty").await;
    }
    let message = trim(&call.args.message).to_owned();
    match call
        .tree
        .send_message(&call.caller, &call.args.id, message)
        .await
    {
        Ok(target) if call.json => {
            json(
                &port,
                &Sent {
                    id: target,
                    accepted: true,
                },
            )
            .await
        }
        Ok(target) => out(&port, format!("sent to {target}\n")).await,
        Err(error) => fail(&port, "send", &error).await,
    }
}

async fn abort<H: AgentHarness>(
    call: Invoked<H, AbortArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let id = call.args.id;
    let Some(child) = NodeId::try_from(id.as_str())
        .ok()
        .filter(|child| call.tree.is_child_of(child, &call.caller))
    else {
        return fail(
            &port,
            "abort",
            &format!("\"{id}\" is not one of your running children"),
        )
        .await;
    };
    call.tree.abort_child(&child).await;
    if call.json {
        return json(
            &port,
            &Aborted {
                id: child,
                aborted: true,
            },
        )
        .await;
    }
    out(&port, format!("aborted {child}\n")).await
}

async fn list<H: AgentHarness>(call: Invoked<H, ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let listing = match call.tree.listing().await {
        Ok(listing) => listing,
        Err(error) => return fail(&port, "list", &error).await,
    };
    if call.json {
        return json(
            &port,
            &Listing {
                tree: listing.entries(&call.caller),
            },
        )
        .await;
    }
    out(&port, format!("{}\n", listing.render(&call.caller))).await
}

async fn show<H: AgentHarness>(call: Invoked<H, ShowArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let id = call.args.id;
    let Some((snapshot, text)) = NodeId::try_from(id.as_str())
        .ok()
        .and_then(|child| call.tree.show(&child))
    else {
        return fail(&port, "show", &format!("no live agent \"{id}\"")).await;
    };
    if call.json {
        return json(&port, &Shown { agent: snapshot }).await;
    }
    out(&port, text).await
}

async fn started(port: &RpcPort, json_output: bool, child: NodeId) -> Result<u8, RpcError> {
    if json_output {
        return json(port, &Started { subagent_id: child }).await;
    }
    out(port, format!("subagentId: {child}\n")).await
}

async fn json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
    let text = serde_json::to_string(value).expect("a command's output serializes");
    out(port, format!("{text}\n")).await
}

async fn out(port: &RpcPort, text: String) -> Result<u8, RpcError> {
    port.stdout(text.into_bytes()).await?;
    Ok(0)
}

/// A verb's failure: `demi agent <verb>: <reason>` on stderr, exit 1.
async fn fail(port: &RpcPort, verb: &str, reason: &str) -> Result<u8, RpcError> {
    port.stderr(format!("demi agent {verb}: {reason}\n").into_bytes())
        .await?;
    Ok(1)
}
