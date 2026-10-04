//! The `demi agent` command group (`subagents.md` § Model-facing surface):
//! its declarations, and the handlers that run each call against the live
//! tree of the invoking job's conversation. Lifecycle (`spawn`, `abort`,
//! `resume`) acts on the caller's own children; communication and reads
//! reach any live agent of the tree.

use std::rc::{Rc, Weak};

use demi_agent_tools::HostResolver;
use demi_host_interface::{
    Call, CommandSet, GroupBuilder, LeafBuilder, RegisterError, RpcError, RpcPort, TypedRpc,
};
use demi_shared_types::{NodeId, Profile, is_blank, trim};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    AgentServer,
    shell_output::shell_group,
    tree::{AgentSnapshot, StartInput, Tree, TreeEntry},
};

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
    /// Target agent number from the tree, or "parent" for the session that
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
    id: u64,
}

/// The input of `demi agent resume`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResumeArgs {
    /// subagentId of an archived child
    id: u64,
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
    /// Agent number from the tree
    id: u64,
}

/// `demi agent spawn --json` and `resume --json`.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Started {
    subagent_id: u64,
}

/// `demi agent send --json`.
#[derive(Serialize, JsonSchema)]
struct Sent {
    id: u64,
    accepted: bool,
}

/// `demi agent abort --json`.
#[derive(Serialize, JsonSchema)]
struct Aborted {
    id: u64,
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

/// A node's commands: the product's, with the `demi agent` and `demi shell`
/// groups grafted under a `demi` group, replacing any `agent` or `shell`
/// child the product declared, or under a new `demi` root.
pub(crate) fn with_runtime_groups<H: HostResolver>(
    product_commands: &CommandSet,
    server: &Rc<AgentServer<H>>,
    can_spawn: bool,
    profiles: &[Profile],
) -> Result<CommandSet, RegisterError> {
    let mut commands = product_commands.clone();
    let groups = [
        agent_group(Rc::downgrade(server), can_spawn, profiles),
        shell_group(Rc::downgrade(server)),
    ];
    let has_demi = commands.declarations().any(|root| root.name() == "demi");
    if has_demi {
        for group in groups {
            commands.graft(&["demi"], group)?;
        }
    } else {
        let demi = groups.into_iter().fold(
            GroupBuilder::new("demi", "Demi agent runtime commands."),
            GroupBuilder::group,
        );
        commands.register(demi)?;
    }
    Ok(commands)
}

/// The `agent` group of a node: every verb, or communication and reads only
/// when the node may not spawn.
fn agent_group<H: HostResolver>(
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
                    format!("Named subagent profile; omit to inherit the parent's model, prompt, Host and commands. Available: {available}."),
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
pub(super) struct Invoked<H: HostResolver, A> {
    pub(super) tree: Rc<Tree<H>>,
    caller: NodeId,
    json: bool,
    pub(super) args: A,
}

/// Binds `run` to the tree and node a call names. A call from a job whose
/// tree is not live fails.
pub(super) fn verb<H, A, F, Fut>(
    server: Weak<AgentServer<H>>,
    run: F,
) -> impl Fn(Call<A>, RpcPort) -> futures_util::future::LocalBoxFuture<'static, Result<u8, RpcError>>
where
    H: HostResolver,
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
            // The backend's record of the job names its node.
            let caller = call
                .invocation
                .caller
                .as_ref()
                .map(|caller| caller.node.clone())
                .ok_or_else(|| {
                    RpcError::Failed("the command runs only in an agent's job".into())
                })?;
            let tree = server
                .tree(&root)
                .ok_or_else(|| RpcError::Failed(format!("the conversation {root} is not open")))?;
            let invoked = Invoked {
                tree,
                caller,
                json: call.invocation.json,
                args: call.args,
            };
            run(invoked, port).await
        })
    }
}

async fn spawn<H: HostResolver>(
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
    match call.tree.start(&call.caller, input).await {
        Ok(child) => started(&port, call.json, child).await,
        Err(error) => fail(&port, "spawn", &error).await,
    }
}

async fn resume<H: HostResolver>(
    call: Invoked<H, ResumeArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let message = trim(&call.args.message);
    if message.is_empty() {
        return fail(&port, "resume", "message must not be empty").await;
    }
    let number = call.args.id;
    let id = match call.tree.child_of(&call.caller, number).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            let reason = format!("no archived subagent {number} (see `demi agent list`)");
            return fail(&port, "resume", &reason).await;
        }
        Err(error) => return fail(&port, "resume", &error).await,
    };
    let input = StartInput::Resume {
        id,
        message: message.to_owned(),
    };
    match call.tree.start(&call.caller, input).await {
        Ok(child) => started(&port, call.json, child).await,
        Err(error) => fail(&port, "resume", &error).await,
    }
}

async fn send<H: HostResolver>(call: Invoked<H, SendArgs>, port: RpcPort) -> Result<u8, RpcError> {
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

async fn abort<H: HostResolver>(
    call: Invoked<H, AbortArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let number = call.args.id;
    let Some(child) = call.tree.live_child_of(&call.caller, number) else {
        return fail(
            &port,
            "abort",
            &format!("{number} is not one of your running children"),
        )
        .await;
    };
    call.tree.abort_child(&child).await;
    if call.json {
        return json(
            &port,
            &Aborted {
                id: number,
                aborted: true,
            },
        )
        .await;
    }
    out(&port, format!("aborted {number}\n")).await
}

async fn list<H: HostResolver>(call: Invoked<H, ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
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

async fn show<H: HostResolver>(call: Invoked<H, ShowArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let number = call.args.id;
    let Some((snapshot, text)) = call.tree.show(number) else {
        return fail(&port, "show", &format!("no live agent {number}")).await;
    };
    if call.json {
        return json(&port, &Shown { agent: snapshot }).await;
    }
    out(&port, text).await
}

async fn started(port: &RpcPort, json_output: bool, child: u64) -> Result<u8, RpcError> {
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
