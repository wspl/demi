//! The `demi agent` command group: its declarations, and the handlers that run
//! each call against the live tree of the invoking job's conversation.
//!
//! - Lifecycle verbs act on the caller's own children.
//! - Communication and reads reach any live agent of the tree.
//! - `cwd` reads or moves the caller's own working directory.
//!
//! The model keeps the one `shell` tool. The group is grafted into every node's
//! command set, the root's included (the product never declares an `agent`
//! group, the plugin host refuses the name), and no command is stripped by
//! depth.
//!
//! Design decisions:
//!
//! - `spawn`, `fork` and `resume` answer once the child exists and its message
//!   is queued, because the supervisor owns the child independently of the
//!   invoking job. The outcome arrives as a message, so there is no `wait`
//!   verb.
//! - Briefs and messages come from stdin only, so a long text never meets
//!   quoting or argument limits.
//! - There is no `--model` option: a child's model comes from its profile or
//!   its parent.
//! - `--profile`'s help names no profile: help is part of the system prompt,
//!   rendered once per tree, and the profiles change with the user's settings.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use demi_agent_tools::{HostResolver, Toolset};
use demi_host_interface::{
    Call, CommandSet, DEMI_ROOT, GroupBuilder, LeafBuilder, RegisterError, RpcError, RpcPort,
    TypedRpc,
};
use demi_shared_types::{NodeId, is_blank, trim};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    AgentServer,
    profile_commands::{PROFILES_CATEGORY, profiles_group},
    workflow_commands::{WORKFLOW_CATEGORY, workflows_group},
    shell_output::shell_group,
    tree::{ShownAgent, StartInput, Tree, TreeEntry, list_text},
};

/// The one description of a spawn's brief.
const SPAWN_PROMPT: &str = "The child's first user message and only task brief. The child starts with an empty transcript and cannot see this conversation: do not refer to prior turns, and do not paste this conversation or the product user's message unchanged. Include the goal for this child, applicable decisions and constraints, whether to edit or only report, how to verify, and every concrete identifier it needs (paths, ids, error text, commands already tried and their key results). State the exact shape of the last assistant text it should return.";

const SPAWN_SUMMARY: &str = "Start a child agent session and return its id immediately after creation. The child runs independently of this command. Completion arrives as a message to the parent, waking it when idle. When you have no independent work left, end your turn and let the completion message wake you. Do not poll agent list/show to wait for children. Use agent send to communicate and agent abort to stop it. Children can spawn children of their own.";

const SEND_SUMMARY: &str = "Deliver information to any live agent in the tree, or parent. A busy recipient incorporates it through internal steering; an idle recipient wakes. Returns after durable acceptance, without waiting for an answer. A child's final answer is delivered automatically. Archived recipients must be reopened by their parent with resume.";

const ABORT_SUMMARY: &str = "Abort your own running children, each with its whole subtree. Siblings are untouched; only the spawning session may abort a child. --json takes one id.";

const RESUME_SUMMARY: &str = "Revive one of your own archived children with a new user message on its preserved transcript, which it keeps whole under the same number and description. It goes on with the profile it runs on, or with the one you name, whose model, effort, Context and instructions it takes from this round on, whichever vendor's model that is. Return its id immediately after accepting the message; completion is delivered separately to the parent. Use agent send to communicate and agent abort to stop it. Archived ids are in agent list.";

const FORK_SUMMARY: &str = "Start a new child from what one of your own children, running or archived, already knows: a copy of its history up to its latest completed answer, continued with your message on the profile you name. Use it to hand a child's findings to another profile without the work done again. The source is untouched and goes on if it runs. Return the new child's id immediately after creation; its completion arrives as a message to you, as a spawned child's does. To give the same child another round, on what it runs on or on another profile, use agent resume instead.";

/// How to write a fork's message, the field it reads from stdin.
const FORK_MESSAGE: &str = "The fork's next user message, which it reads after the source's history: what to do now with what the source found, such as \"Fix it as you described\", with what the source did not know (where to work, which branch, what to verify) and the exact shape of the last assistant text it should return.";

const LIST_SUMMARY: &str = "List the agents of this conversation's tree from the root down, each node's children after it: status, profile, execution, seconds since start and since the last event, and description; --all adds the archived children (finished, revivable by their parent). A read, not a wait — not for polling loops.";

/// The `--profile` option's description, which names no profile: the help
/// is rendered once per tree, and the profiles change with the user's
/// settings.
const PROFILE_OPTION: &str = "The name of one of the user's subagent profiles. Without it the child inherits your model, prompt, Host and commands; `demi agent profiles list` lists the profiles, with when to use each, that may fit the task better.";

/// The `--profile` option's description of a resume, which names no profile
/// for the reason [`PROFILE_OPTION`]'s does not.
const RESUME_PROFILE_OPTION: &str = "The name of one of the user's subagent profiles to go on with. Without it the child goes on with the profile it runs on; `demi agent profiles list` lists the profiles, with when to use each.";

const SHOW_SUMMARY: &str = "Bounded snapshot of any agent in the tree (root excluded): for a live one, execution state, recent tool titles with durations, last assistant text; for an archived one, how and when it closed with its result or failure. Idle (s), the seconds since its last event, and Execution for (s) tell motion from stall. Omits tool outputs, file contents, and older turns. Several ids show each in order; --json takes one. A read, not a wait — not for polling loops.";

/// The input of `demi agent spawn`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct SpawnArgs {
    prompt: String,
    profile: Option<String>,
    /// Short UI title distinguishing concurrent children.
    description: Option<String>,
    /// The child's working directory, read against yours; yours when omitted.
    cwd: Option<String>,
}

/// The input of `demi agent cwd`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CwdArgs {
    /// The directory to work in from your next shell call on, read against your current one.
    dir: Option<String>,
}

const CWD_SUMMARY: &str = "Print your working directory, where your shell calls start, or change it to a directory of the primary Host.";

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
    /// subagentIds from spawn stdout, aborted in order
    #[schemars(length(min = 1))]
    id: Vec<u64>,
}

/// The input of `demi agent resume`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResumeArgs {
    /// subagentId of an archived child
    id: u64,
    /// The reviving user message.
    message: String,
    profile: Option<String>,
}

/// The input of `demi agent fork`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ForkArgs {
    /// subagentId of one of your children, running or archived, whose
    /// history the fork starts from
    id: u64,
    message: String,
    profile: Option<String>,
    /// Short UI title; the source's followed by " (fork)" when omitted.
    description: Option<String>,
}

/// The input of `demi agent list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    /// Also list the archived children, which only history keeps.
    all: Option<bool>,
}

/// The input of `demi agent show`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShowArgs {
    /// Agent numbers from the tree, shown in order
    #[schemars(length(min = 1))]
    id: Vec<u64>,
}

/// The group's entry in the model's capability index.
const AGENT_ENTRY: &str = "Runs subagents, each with a fresh context of its own, that work in parallel on parts of a task and report back; resumes or forks one onto another profile to go on from what it knows; sends them messages and checks on them. Use it when a task splits into independent pieces, such as several sites or libraries to look into; for a long investigation whose details would crowd your context; or for a task a subagent profile fits: run `demi agent profiles list` first and give each the one that fits. Also lists, checks and picks workflows. A single lookup or edit is faster by hand.";

/// `demi agent spawn --json`, `fork --json` and `resume --json`.
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


/// The command sets every tree's nodes share.
///
/// They are the product's toolset of each revision, and each set with the
/// runtime's groups grafted, by the product's set it was grafted on and the
/// spawning right. Each is kept only while a node holds it, so the last tree to
/// close frees it. The grafted groups hold nothing of a tree: their handlers
/// find the caller's tree from the invocation.
#[derive(Default)]
pub(crate) struct NodeCommandSets {
    toolsets: RefCell<Vec<HeldToolset>>,
    grafted: RefCell<Vec<Grafted>>,
}

/// A toolset some node holds, which the last node to drop it frees.
struct HeldToolset {
    revision: Rc<str>,
    commands: Weak<CommandSet>,
}

/// A set some node holds, which the last node to drop it frees.
struct Grafted {
    /// Weak: a node holds its inherited set itself. The allocation stays
    /// while this does, so no other set takes its address.
    inherited: Weak<CommandSet>,
    can_spawn: bool,
    commands: Weak<CommandSet>,
}

impl NodeCommandSets {
    /// The toolset a node holds, when it is of `revision`.
    pub(crate) fn toolset(&self, revision: &str) -> Option<Toolset> {
        let mut toolsets = self.toolsets.borrow_mut();
        toolsets.retain(|held| held.commands.strong_count() > 0);
        let held = toolsets.iter().find(|held| &*held.revision == revision)?;
        Some(Toolset {
            commands: held.commands.upgrade()?,
            revision: held.revision.clone(),
        })
    }

    /// Keeps `composed` beside the toolsets of other revisions, unless
    /// another open composed the same revision first, whose toolset it
    /// answers then.
    pub(crate) fn hold_toolset(&self, composed: Toolset) -> Toolset {
        if let Some(held) = self.toolset(&composed.revision) {
            return held;
        }
        self.toolsets.borrow_mut().push(HeldToolset {
            revision: composed.revision.clone(),
            commands: Rc::downgrade(&composed.commands),
        });
        composed
    }

    /// The set a node grafted on `inherited` with `can_spawn` holds, if one
    /// still does.
    pub(crate) fn grafted(&self, inherited: &Rc<CommandSet>, can_spawn: bool) -> Option<Rc<CommandSet>> {
        let inherited = Rc::downgrade(inherited);
        let mut grafted = self.grafted.borrow_mut();
        grafted.retain(|held| held.commands.strong_count() > 0);
        grafted
            .iter()
            .find(|held| held.can_spawn == can_spawn && held.inherited.ptr_eq(&inherited))
            .and_then(|held| held.commands.upgrade())
    }

    pub(crate) fn hold_grafted(&self, inherited: &Rc<CommandSet>, can_spawn: bool, commands: &Rc<CommandSet>) {
        self.grafted.borrow_mut().push(Grafted {
            inherited: Rc::downgrade(inherited),
            can_spawn,
            commands: Rc::downgrade(commands),
        });
    }
}

/// A node's commands: the product's, with the `demi agent` and `demi shell`
/// groups grafted under a `demi` group, replacing any `agent` or `shell`
/// child the product declared, or under a new `demi` root.
pub(crate) fn with_runtime_groups<H: HostResolver>(
    product_commands: &CommandSet,
    server: &Rc<AgentServer<H>>,
    can_spawn: bool,
) -> Result<CommandSet, RegisterError> {
    let mut commands = product_commands.clone();
    let groups = [
        agent_group(Rc::downgrade(server), can_spawn),
        shell_group(Rc::downgrade(server)),
    ];
    let has_demi = commands.declarations().any(|root| root.name() == DEMI_ROOT);
    if has_demi {
        for group in groups {
            commands.graft(&[DEMI_ROOT], group)?;
        }
    } else {
        let demi = groups.into_iter().fold(
            GroupBuilder::new(DEMI_ROOT, "Demi agent runtime commands."),
            GroupBuilder::group,
        );
        commands.register(demi)?;
    }
    Ok(commands)
}

/// The `agent` group of a node: every verb, or communication and reads only
/// when the node may not spawn.
fn agent_group<H: HostResolver>(server: Weak<AgentServer<H>>, can_spawn: bool) -> GroupBuilder {
    let summary = if can_spawn {
        "Agent tree: spawn, fork and manage your own children; send, list and show any live agent."
    } else {
        "Agent tree communication: this session may not spawn subagents; send, list and show any live agent."
    };
    let mut group = GroupBuilder::new("agent", summary).index_entry(AGENT_ENTRY);
    for (category, action, description) in [PROFILES_CATEGORY, WORKFLOW_CATEGORY] {
        group = group.permission(category, action, description);
    }
    if can_spawn {
        group = group.leaf(
            LeafBuilder::rpc("spawn", SPAWN_SUMMARY)
                .input::<SpawnArgs>()
                .describe("prompt", SPAWN_PROMPT)
                .describe("profile", PROFILE_OPTION)
                .stdin_field("prompt")
                .success_output("stdout is \"subagentId: <id>\"; creation succeeded, not necessarily execution")
                .json_output::<Started>()
                .bind(TypedRpc::new(verb(server.clone(), spawn))),
        );
    }
    group = group.leaf(
        LeafBuilder::rpc("cwd", CWD_SUMMARY)
            .input::<CwdArgs>()
            .positionals(["dir"])
            .bind(TypedRpc::new(verb(server.clone(), cwd))),
    );
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
                    .describe("profile", RESUME_PROFILE_OPTION)
                    .stdin_field("message")
                    .json_output::<Started>()
                    .bind(TypedRpc::new(verb(server.clone(), resume))),
            )
            .leaf(
                LeafBuilder::rpc("fork", FORK_SUMMARY)
                    .input::<ForkArgs>()
                    .positionals(["id"])
                    .describe("message", FORK_MESSAGE)
                    .describe("profile", PROFILE_OPTION)
                    .stdin_field("message")
                    .success_output("stdout is \"subagentId: <id>\", the new child's; creation succeeded, not necessarily execution")
                    .json_output::<Started>()
                    .bind(TypedRpc::new(verb(server.clone(), fork))),
            );
    }
    group
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<ListArgs>()
                .json_output::<Vec<TreeEntry>>()
                .bind(TypedRpc::new(verb(server.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<ShowArgs>()
                .positionals(["id"])
                .json_output::<ShownAgent>()
                .bind(TypedRpc::new(verb(server.clone(), show))),
        )
        .group(profiles_group(server.clone()))
        .group(workflows_group(server))
}

/// A verb's call against the live tree of the invoking job's conversation,
/// on behalf of the job's node.
pub(super) struct Invoked<H: HostResolver, A> {
    pub(super) tree: Rc<Tree<H>>,
    /// The node whose job runs the command.
    pub(super) caller: NodeId,
    /// The node's `shell` call that started the job.
    pub(super) call: String,
    /// The invoking shell's directory, which a relative path an argument
    /// names is read against.
    pub(super) cwd: String,
    /// The command's path, which a failure of one of several values
    /// begins with (`crates/host-interface/src/rpc.rs`).
    pub(super) command: String,
    pub(super) json: bool,
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
            // The backend's record of the job names its node and call.
            let job = call.invocation.caller.clone().ok_or_else(|| {
                RpcError::Failed("the command runs only in an agent's job".into())
            })?;
            let tree = server
                .tree(&root)
                .ok_or_else(|| RpcError::Failed(format!("the conversation {root} is not open")))?;
            let invoked = Invoked {
                tree,
                caller: job.node,
                call: job.tool_use_id,
                cwd: call.invocation.cwd.clone(),
                command: call.invocation.path.join(" "),
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
        return refuse("prompt must not be empty");
    }
    let input = StartInput::Spawn {
        prompt: prompt.to_owned(),
        profile_name: call.args.profile,
        description: call.args.description.unwrap_or_default(),
        cwd: call.args.cwd,
    };
    match call.tree.start(&call.caller, input).await {
        Ok(child) => started(&port, call.json, child).await,
        Err(error) => fail(&error),
    }
}

/// Prints the caller's directory, or moves the caller to `dir`
/// (`crates/agent-server/src/node/mod.rs`).
async fn cwd<H: HostResolver>(call: Invoked<H, CwdArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let changed = match &call.args.dir {
        Some(dir) => call.tree.change_cwd(&call.caller, dir).await,
        None => call.tree.directory_of(&call.caller).await,
    };
    match changed {
        Ok(cwd) => out(&port, format!("{cwd}\n")).await,
        Err(error) => fail(&error),
    }
}

async fn resume<H: HostResolver>(
    call: Invoked<H, ResumeArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let message = trim(&call.args.message);
    if message.is_empty() {
        return refuse("message must not be empty");
    }
    let number = call.args.id;
    let id = match call.tree.child_of(&call.caller, number).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            let reason = format!("no archived subagent {number} (see `demi agent list`)");
            return fail(&reason);
        }
        Err(error) => return fail(&error),
    };
    let input = StartInput::Resume {
        id,
        message: message.to_owned(),
        profile_name: call.args.profile,
    };
    match call.tree.start(&call.caller, input).await {
        Ok(child) => started(&port, call.json, child).await,
        Err(error) => fail(&error),
    }
}

async fn fork<H: HostResolver>(call: Invoked<H, ForkArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let message = trim(&call.args.message);
    if message.is_empty() {
        return refuse("message must not be empty");
    }
    let input = StartInput::Fork {
        source: call.args.id,
        message: message.to_owned(),
        profile_name: call.args.profile,
        description: call.args.description,
    };
    match call.tree.start(&call.caller, input).await {
        Ok(child) => started(&port, call.json, child).await,
        Err(error) => fail(&error),
    }
}

async fn send<H: HostResolver>(call: Invoked<H, SendArgs>, port: RpcPort) -> Result<u8, RpcError> {
    if is_blank(&call.args.message) {
        return refuse("message must not be empty");
    }
    let message = trim(&call.args.message).to_owned();
    match call
        .tree
        .send_message(&call.caller, &call.call, &call.args.id, message)
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
        Err(error) => fail(&error),
    }
}

async fn abort<H: HostResolver>(
    call: Invoked<H, AbortArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    one_for_json(&call, call.args.id.len())?;
    let mut failed = false;
    for &number in &call.args.id {
        let Some(child) = call.tree.live_child_of(&call.caller, number) else {
            failed = true;
            fail_one(&port, &call.command, number, "not one of your running children").await?;
            continue;
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
        out(&port, format!("aborted {number}\n")).await?;
    }
    Ok(u8::from(failed))
}

async fn list<H: HostResolver>(call: Invoked<H, ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let listing = match call.tree.listing().await {
        Ok(listing) => listing,
        Err(error) => return fail(&error),
    };
    let entries = listing.entries(&call.caller, call.args.all == Some(true));
    if call.json {
        return json(&port, &entries).await;
    }
    out(&port, list_text(&entries)).await
}

async fn show<H: HostResolver>(call: Invoked<H, ShowArgs>, port: RpcPort) -> Result<u8, RpcError> {
    one_for_json(&call, call.args.id.len())?;
    let mut failed = false;
    for &number in &call.args.id {
        let (snapshot, text) = match call.tree.show(number).await {
            Ok(Some(shown)) => shown,
            Ok(None) => {
                failed = true;
                fail_one(&port, &call.command, number, "no such subagent").await?;
                continue;
            }
            Err(error) => {
                failed = true;
                fail_one(&port, &call.command, number, &error).await?;
                continue;
            }
        };
        if call.json {
            return json(&port, &snapshot).await;
        }
        out(&port, text).await?;
    }
    Ok(u8::from(failed))
}

async fn started(port: &RpcPort, json_output: bool, child: u64) -> Result<u8, RpcError> {
    if json_output {
        return json(port, &Started { subagent_id: child }).await;
    }
    out(port, format!("subagentId: {child}\n")).await
}

pub(super) async fn json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
    let text = serde_json::to_string(value).expect("a command's output serializes");
    out(port, format!("{text}\n")).await
}

pub(super) async fn out(port: &RpcPort, text: String) -> Result<u8, RpcError> {
    port.stdout(text.into_bytes()).await?;
    Ok(0)
}

/// Arguments a verb refuses, such as options that do not go together or an
/// empty message: a usage error, which the dispatcher tells with the leaf's
/// usage, exit 2 (`crates/command-declarations/src/parse.rs`).
pub(super) fn refuse(reason: &str) -> Result<u8, RpcError> {
    Err(RpcError::Usage(reason.to_owned()))
}

/// A verb's failure, which the dispatcher tells after the command's path:
/// `demi agent <verb>: <reason>` on stderr, exit 1.
pub(super) fn fail(reason: &str) -> Result<u8, RpcError> {
    Err(RpcError::Failed(reason.to_owned()))
}

/// The failure of one of several ids, told at once as GNU tools tell
/// theirs, `demi agent show: 9: no such subagent`; the verb goes on with
/// the next.
pub(super) async fn fail_one(
    port: &RpcPort,
    command: &str,
    id: impl std::fmt::Display,
    reason: &str,
) -> Result<(), RpcError> {
    port.stderr(format!("{command}: {id}: {reason}\n").into_bytes())
        .await?;
    Ok(())
}

/// Refuses `--json` with several ids, whose output is one JSON value, as a
/// usage error.
pub(super) fn one_for_json<H: HostResolver, A>(call: &Invoked<H, A>, ids: usize) -> Result<(), RpcError> {
    if call.json && ids > 1 {
        return Err(RpcError::Usage("--json takes one id; give the others a call each".into()));
    }
    Ok(())
}
