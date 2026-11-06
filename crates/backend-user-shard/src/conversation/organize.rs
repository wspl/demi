//! The product's `demi conversation` group
//! (`crates/core-host-access/src/host_commands.rs`).
//!
//! The group asks no permission. An agent renames its conversation at once:
//! the title is the conversation's own, and the user renames it back as
//! easily. Listing the user's projects and conversations, reading one's
//! transcript (`history.rs`), sending a message to another's main agent and
//! starting a conversation reach only the user's own conversations, all of
//! which the user sees in the sidebar, stops and archives. An archived
//! conversation is listed nowhere and receives nothing, so an archive keeps
//! it out of the way. Where the conversation runs decides its project, so an
//! agent moves it into or out of one with `demi host move`
//! (`host_commands.rs`).
//!
//! A conversation an agent starts (`new`) is the user's own, as one they
//! started: it takes the starting conversation's place and settings, and its
//! first message is the brief, an agent's message from the starting
//! conversation as `send` delivers one, so its agent can answer it.

use std::future::Future;
use std::rc::{Rc, Weak};

use demi_core_store::conversation_index::{ConversationRecord, ConversationStart, Creation, RecordChange};
use demi_conversation_lane_protocol::ClientContent;
use demi_shared_types::WorkflowChoice;
use demi_web_api_protocol::conversations::{ConversationPatch, ConversationTarget, FieldResult};
use demi_core_host_access::host_commands::conversation_of;
use demi_core_host_access::root_of;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::output::{self, Cell, Table};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, Sender as AgentSender, SenderConversation, Timestamp};
use demi_web_api_protocol::devices::DeviceKind;
use demi_core_store::workspaces::WorkspaceRecord;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ProjectId, WorkspaceId};
use demi_web_api_protocol::search::{RESULTS_MAX, SearchText};
use demi_core_store::Selection;
use demi_web_api_protocol::text::Trimmed;
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::cost_report::{ConversationCost, CostArgs, cost};
use super::history::{history_group, shell_word};
use super::search::search;
use crate::shard::Shard;

const SUMMARY: &str = "This conversation in the user's sidebar: rename it, and list the user's projects and conversations; send a message to another conversation, or start a new one; what a conversation's calls to the model cost; and transcripts, which history searches and reads: an agent's of this conversation, or of another conversation of the user.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Renames the conversation, lists projects and the user's conversations, sends messages to other conversations, starts new conversations, shows what a conversation cost, searches and reads transcripts of any agent, including those of other conversations.";

const COST_SUMMARY: &str = "Print what this conversation's calls to model vendors cost, or another conversation's with --conversation <id>: every agent's and the calls of none, such as a title's. Prices are in dollars at API prices, which a subscription's calls are priced at too, and are no bill. It prints the calls, the price and the share of the turn calls' prompt tokens the cache served, then tables: all, the last day and the last hour; by agent, 0 the main agent, with its profile; by model; by purpose (turn, compaction, title, suggested_replies, image, video, audio); the media made with the vendor's own cost in its currency; the subscription accounts that answered and how often an agent's calls moved to another; the cache misses by why; the calls not answered by how they ended. A table with nothing to show is left out. Each table counts the tokens sent uncached (input), written to the cache, read from it and the output. `demi conversation history cost` breaks one agent's down by turn and call.";

const RENAME_SUMMARY: &str = "Rename this conversation, as its title in the user's sidebar.";

const PROJECTS_SUMMARY: &str = "List the user's projects of every workspace: id, workspace, Host, directory and name.";

const LIST_SUMMARY: &str = "List the user's conversations that are not archived, newest change first, twenty to a page, of every workspace: id, title, workspace, project and the time of the last change. With words, only those the app's Search finds with them in the title or the messages, fifty at most, which stderr says when more match. A conversation's id goes to history --conversation to read it, and to `send` to message it; stderr names the next page.";

const SEND_SUMMARY: &str = "Send a message to the main agent of another conversation of the user that is not archived, by its id from list. A busy recipient takes it into its running turn; an idle one wakes. Returns once it is accepted, without waiting for an answer: the recipient answers, if it does, by sending to this conversation.";

const NEW_SUMMARY: &str = "Start a new conversation of the user for a separate piece of work, which runs on its own in the user's sidebar: the brief on stdin is its first message, a message from this conversation, so its agent can answer with send. It starts in this conversation's Host and directory, or in --project's, with this conversation's model and workflow, and is titled from the brief unless --title names it. Returns once the brief is taken, with the new conversation's id.";

/// The input of `demi conversation rename`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameArgs {
    /// The new title
    #[schemars(length(min = 1))]
    title: String,
}

/// The input of `demi conversation send`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SendArgs {
    /// The conversation, by its id
    #[schemars(extend("patternDescription" = "a conversation's id, as demi conversation list shows it"))]
    conversation: ConversationId,
    /// The message
    message: String,
}

/// The input of `demi conversation new`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NewArgs {
    /// The brief, the new conversation's first message
    message: String,
    /// Its title; generated from the brief when absent
    #[schemars(length(min = 1))]
    title: Option<String>,
    /// The project it starts in, by its id from `demi conversation projects`; this conversation's Host and directory when absent
    project: Option<ProjectId>,
}

/// The input of a leaf that takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// The `conversation` group, whose handlers act in `shard`.
pub(crate) fn conversation_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("conversation", SUMMARY)
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc("rename", RENAME_SUMMARY)
                .input::<RenameArgs>()
                .positionals(["title"])
                .bind(TypedRpc::new(verb(shard.clone(), rename))),
        )
        .leaf(
            LeafBuilder::rpc("projects", PROJECTS_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<ListedProject>>()
                .bind(TypedRpc::new(verb(shard.clone(), projects))),
        )
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<ListArgs>()
                .positionals(["words"])
                .success_output("a table of the page's conversations; on stderr, the page and the list of the next page")
                .json_output::<Vec<ListedConversation>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("send", SEND_SUMMARY)
                .input::<SendArgs>()
                .positionals(["conversation"])
                .stdin_field("message")
                .success_output("the conversation the message reached")
                .bind(TypedRpc::new(verb(shard.clone(), send))),
        )
        .leaf(
            LeafBuilder::rpc("new", NEW_SUMMARY)
                .input::<NewArgs>()
                .stdin_field("message")
                .success_output("the conversation started, its title and id")
                .bind(TypedRpc::new(verb(shard.clone(), new))),
        )
        .leaf(
            LeafBuilder::rpc("cost", COST_SUMMARY)
                .input::<CostArgs>()
                .success_output("the conversation's totals as labelled lines, with a table for each way they are broken down")
                .json_output::<ConversationCost>()
                .bind(TypedRpc::new(verb(shard.clone(), cost))),
        )
        .group(history_group(shard))
}

/// A handler over the live shard, for the product's groups that act in it;
/// a call after the shard closed fails.
pub(crate) fn verb<A, F, Fut>(
    shard: Weak<Shard>,
    run: F,
) -> impl Fn(Call<A>, RpcPort) -> LocalBoxFuture<'static, Result<u8, RpcError>>
where
    A: 'static,
    F: Fn(Rc<Shard>, Call<A>, RpcPort) -> Fut + Clone + 'static,
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

pub(crate) fn failed(error: impl ToString) -> RpcError {
    RpcError::Failed(error.to_string())
}

/// Arguments a handler refuses, such as a blank title: a usage error, which
/// the dispatcher tells with the leaf's usage, exit 2 (`crates/command-declarations/src/parse.rs`).
pub(crate) fn refused(message: impl ToString) -> RpcError {
    RpcError::Usage(message.to_string())
}

async fn rename(shard: Rc<Shard>, call: Call<RenameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let title = Trimmed::from(call.args.title).into_string();
    if title.is_empty() {
        return Err(refused("the title is blank"));
    }
    shard
        .transition(&conversation, RecordChange::Title(title.clone()).into())
        .await
        .map_err(failed)?;
    port.stdout(format!("Renamed this conversation to \u{201c}{title}\u{201d}.\n"))
        .await?;
    Ok(0)
}

/// Sends the message to the main agent of the conversation `call` names, as
/// an agent's message from the calling agent (`demi agent send`), and answers
/// once it is in the recipient's checkpoint.
async fn send(shard: Rc<Shard>, call: Call<SendArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let here = conversation_of(&call.invocation)?;
    let content = call.args.message.trim();
    if content.is_empty() {
        return Err(refused("the message is empty"));
    }
    let target = call.args.conversation.clone();
    let store = shard.store();
    let recipient = match store.conversation(target.clone()).await.map_err(failed)? {
        Some(record) => record,
        // Another user's conversation is none of the caller's.
        _ => return Err(failed(format!("no conversation {target} (see `demi conversation list`)"))),
    };
    if recipient.id == here {
        return Err(failed("this is the sending conversation; `demi agent send` reaches its agents"));
    }
    if recipient.archived() {
        return Err(failed(format!("conversation {target} is archived, and only the user restores it")));
    }
    let sending = store.conversation(here.clone())
        .await
        .map_err(failed)?
        .ok_or_else(|| failed("the sending conversation is gone"))?;
    let sender = sender_of(&shard, &call, &sending)?;
    deliver(&shard, &sender, &recipient.id, content).await.map_err(failed)?;
    let title = recipient.title.as_deref().unwrap_or("untitled");
    port.stdout(format!("Sent to \u{201c}{title}\u{201d} ({target}).\n")).await?;
    Ok(0)
}

/// The calling agent of `sending` as the sender of a message to another
/// conversation, checked before anything is sent: a side question's agent,
/// for one, sends none.
fn sender_of<A>(shard: &Shard, call: &Call<A>, sending: &ConversationRecord) -> Result<Sender, RpcError> {
    let caller = call
        .invocation
        .caller
        .as_ref()
        .ok_or_else(|| failed("only an agent's command sends a message"))?;
    let tree = shard
        .agent()
        .tree(&root_of(&sending.id))
        .ok_or_else(|| failed("the sending conversation's agents are not running"))?;
    let (mut sender, origin) = tree.sender_of(&caller.node, &caller.tool_use_id).map_err(failed)?;
    sender.conversation = Some(SenderConversation {
        id: root_of(&sending.id),
        title: sending.title.clone(),
    });
    Ok(Sender { sending: sending.id.clone(), node: caller.node.clone(), agent: sender, origin })
}

/// Who sends a message to another conversation, as [`sender_of`] checked it.
struct Sender {
    sending: ConversationId,
    node: NodeId,
    agent: AgentSender,
    origin: Option<BlockId>,
}

/// Admits `content` into the main agent of the user's conversation `to`, an
/// agent's message from `sender`, as `send` and `new` deliver one.
async fn deliver(shard: &Shard, sender: &Sender, to: &ConversationId, content: &str) -> Result<(), String> {
    let id = BlockId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
    let timestamp = shard.services().clock.now();
    let message = |recipient: &NodeId| AgentMessage {
        id: id.clone(),
        sender: Some(sender.agent.clone()),
        recipient_id: recipient.clone(),
        timestamp,
        content: content.to_owned(),
        event: AgentMessageEvent::Message {},
        origin: sender.origin.clone(),
    };
    shard.permission_shard().admit(to, &root_of(to), &message).await?;
    tracing::info!(
        event = "agent_message.sent",
        conversation = %sender.sending,
        from_node = %sender.node,
        to_conversation = %to,
        kind = "conversation",
        message = %id,
        "an agent's message to another conversation was accepted"
    );
    Ok(())
}

/// Starts a conversation of the user with the brief, where the calling
/// conversation works or in the project `--project` names, and answers once
/// the brief is taken. Once the conversation exists, a step that fails after
/// it names it, so the agent sends the brief again with `send` rather than
/// starting another.
async fn new(shard: Rc<Shard>, call: Call<NewArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let here = conversation_of(&call.invocation)?;
    let content = call.args.message.trim().to_owned();
    if content.is_empty() {
        return Err(refused("the brief is empty"));
    }
    let title = call.args.title.as_deref().map(str::trim).filter(|title| !title.is_empty());
    let store = shard.store();
    let starting = store.conversation(here.clone())
        .await
        .map_err(failed)?
        .ok_or_else(|| failed("the starting conversation is gone"))?;
    let sender = sender_of(&shard, &call, &starting)?;
    let (workspace, target, workflow) = match &call.args.project {
        Some(id) => {
            let projects = store.projects().await.map_err(failed)?;
            let project = projects
                .iter()
                .find(|project| project.id == *id)
                .ok_or_else(|| failed(format!("no project {id} (see `demi conversation projects`)")))?;
            let same = matches!(&starting.target, Selection::Project { project_id, .. } if project_id == id);
            // A workflow is a file of its project: another project's
            // conversation takes the user's settings instead, unless
            // subagents were off.
            let workflow = match &starting.workflow {
                WorkflowChoice::Workflow { .. } if !same => WorkflowChoice::Default,
                workflow => workflow.clone(),
            };
            (project.workspace.clone(), ConversationTarget::Project { project_id: id.clone() }, workflow)
        }
        None => (starting.workspace.clone(), place_of(&shard, &starting).await?, starting.workflow.clone()),
    };
    let start = ConversationStart {
        workspace,
        title: title.map(str::to_owned),
        pinned: false,
        target,
        model: starting.model.clone(),
        workflow: workflow.clone(),
    };
    let id = ConversationId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is a conversation's id");
    let record = match shard.create_started(id.clone(), start).await.map_err(failed)? {
        Creation::Created(record) => record,
        created => return Err(failed(format!("the conversation was not created: {created:?}"))),
    };
    let unsent = |step: &str, reason: String| {
        failed(format!(
            "started conversation {id}, but {step}: {reason}; send it the brief with `demi conversation send {id}`"
        ))
    };
    // A new conversation in a linked worktree reads a workflow from the
    // worktree, as its menu lists them before it exists; this one keeps the
    // choice its starting conversation made.
    if record.workflow != workflow {
        let patch = ConversationPatch { workflow: Some(workflow), ..ConversationPatch::default() };
        let update = shard
            .apply_patch(&record.id, patch)
            .await
            .map_err(|error| unsent("its workflow was not kept", error.to_string()))?;
        let refused = update.iter().flat_map(|update| &update.results).find_map(|result| match result {
            FieldResult::Failed { message, .. } => Some(message.clone()),
            _ => None,
        });
        if let Some(message) = refused {
            return Err(unsent("its workflow was not kept", message));
        }
    }
    // The title is the brief's start until one is generated; a failed write
    // of it leaves the placeholder, as for any first message.
    if title.is_none()
        && let Err(error) = shard
            .title_first_message(&record, &[ClientContent::Text { text: content.clone() }])
            .await
    {
        tracing::warn!(event = "conversation.title_failed", conversation = %id, %error, "a started conversation was not titled");
    }
    deliver(&shard, &sender, &record.id, &content)
        .await
        .map_err(|reason| unsent("the brief was not delivered", reason))?;
    let shown = match title {
        Some(title) => Some(title.to_owned()),
        None => store.conversation(record.id.clone()).await.ok().flatten().and_then(|record| record.title),
    };
    let shown = shown.unwrap_or_else(|| "untitled".to_owned());
    port.stdout(format!("Started \u{201c}{shown}\u{201d} ({id}).\n")).await?;
    Ok(0)
}

/// Where `starting` works, as a new conversation is to start there: its
/// Host and directory, the worktree it works in included, and on the Cloud
/// its own directory, as a Fork keeps it (`fork.rs`).
async fn place_of(shard: &Shard, starting: &ConversationRecord) -> Result<ConversationTarget, RpcError> {
    let resolved = shard.host_shard().resolve_target(starting).await.map_err(failed)?;
    Ok(match &starting.target {
        Selection::Cloud { path: None } => ConversationTarget::Cloud {
            path: Some(resolved.path().to_owned()),
        },
        // A worktree is named by its directory, which selects it in its
        // project: the Cloud's as a Cloud directory.
        Selection::Project { device_id, worktree: Some(worktree), .. } => {
            let device = shard.store().device(device_id.clone()).await.map_err(failed)?;
            match device {
                Some(device) if device.kind == DeviceKind::Managed => ConversationTarget::Cloud {
                    path: Some(worktree.path.clone()),
                },
                _ => ConversationTarget::Device {
                    device_id: device_id.clone(),
                    path: worktree.path.clone(),
                },
            }
        }
        selection => selection.shown(&resolved),
    })
}

/// A project as `demi conversation projects` lists it.
#[derive(Serialize, JsonSchema)]
struct ListedProject {
    id: String,
    name: String,
    /// The name of the workspace it is in.
    workspace: String,
    /// The name of the Host it is on.
    host: String,
    /// Its directory on that Host.
    directory: String,
}

async fn projects(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let kept = shard
        .store().projects()
        .await
        .map_err(failed)?;
    let workspaces = shard.store().workspaces().await.map_err(failed)?;
    let mut projects = Vec::new();
    for project in &kept {
        projects.push(ListedProject {
            id: project.id.to_string(),
            name: project.name.clone(),
            workspace: workspace_name(&workspaces, &project.workspace),
            host: shard.host_name(&project.device).await,
            directory: project.path.clone(),
        });
    }
    if call.invocation.json {
        let text = serde_json::to_string(&projects).map_err(failed)?;
        port.stdout(text + "\n").await?;
        return Ok(0);
    }
    let mut table = Table::new(["id", "workspace", "host", "directory", "name"]);
    for project in &projects {
        table.row([
            project.id.as_str(),
            project.workspace.as_str(),
            project.host.as_str(),
            project.directory.as_str(),
            project.name.as_str(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

/// The input of `demi conversation list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    /// Words the title or the messages hold, as the app's Search finds them
    words: Option<Vec<String>>,
    /// The page, from 1; twenty conversations to a page
    #[schemars(range(min = 1))]
    page: Option<u64>,
}

/// A conversation as `demi conversation list` lists it.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedConversation {
    id: ConversationId,
    /// Null while it has none.
    title: Option<String>,
    /// The name of the workspace it is in.
    workspace: String,
    /// The name of its project; null when it runs in none.
    project: Option<String>,
    /// When it last changed.
    changed_at: Timestamp,
}

/// How many conversations a page of `list` shows.
const LISTED_PER_PAGE: usize = 20;

async fn list(shard: Rc<Shard>, call: Call<ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let given = call.args.words.clone().unwrap_or_default();
    let words = given.join(" ");
    // Whether the app's Search stopped at its most: more may match.
    let mut capped = false;
    let mut records = if words.trim().is_empty() {
        shard.store().conversations(false).await.map_err(failed)?
    } else {
        let query = SearchText::try_from(words).map_err(|error| refused(error))?;
        let mut records = Vec::new();
        let results = search(shard.store(), &query, None).await.map_err(failed)?;
        capped = results.len() >= RESULTS_MAX;
        for found in results {
            if let Some(record) = shard.store().conversation(found.conversation_id).await.map_err(failed)?
                && !record.archived()
            {
                records.push(record);
            }
        }
        records
    };
    records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id)));
    let pages = records.len().div_ceil(LISTED_PER_PAGE).max(1);
    let page = usize::try_from(call.args.page.unwrap_or(1)).unwrap_or(usize::MAX);
    if page > pages {
        return Err(failed(format!("the list has {pages} pages")));
    }
    let projects = shard.store().projects().await.map_err(failed)?;
    let workspaces = shard.store().workspaces().await.map_err(failed)?;
    let project = |selection: &Selection| match selection {
        Selection::Project { project_id, .. } => projects
            .iter()
            .find(|project| project.id == *project_id)
            .map(|project| project.name.clone()),
        _ => None,
    };
    let listed: Vec<ListedConversation> = records
        .iter()
        .skip((page - 1) * LISTED_PER_PAGE)
        .take(LISTED_PER_PAGE)
        .map(|record| ListedConversation {
            id: record.id.clone(),
            title: record.title.clone(),
            workspace: workspace_name(&workspaces, &record.workspace),
            project: project(&record.target),
            changed_at: record.updated_at,
        })
        .collect();
    if page < pages {
        let quoted: Vec<String> = given.iter().map(|word| shell_word(word)).collect();
        let words = if quoted.is_empty() { String::new() } else { format!(" -- {}", quoted.join(" ")) };
        let command = call.invocation.path.join(" ");
        port.stderr(output::note(
            &command,
            &format!("page {page} of {pages}; next: demi conversation list --page {}{words}", page + 1),
        ))
        .await?;
    }
    if capped {
        let command = call.invocation.path.join(" ");
        port.stderr(output::note(
            &command,
            &format!("the search found its most, {RESULTS_MAX} conversations; more may match, so add words to narrow it"),
        ))
        .await?;
    }
    if call.invocation.json {
        let text = serde_json::to_string(&listed).map_err(failed)?;
        port.stdout(text + "\n").await?;
        return Ok(0);
    }
    let mut table = Table::new(["id", "title", "workspace", "project", "changed"]);
    for conversation in &listed {
        table.row::<Cell>([
            conversation.id.as_str().into(),
            conversation.title.as_deref().into(),
            conversation.workspace.as_str().into(),
            conversation.project.as_deref().into(),
            conversation.changed_at.into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

/// The name of the workspace `id` of `workspaces`, as Demi's commands say it.
fn workspace_name(workspaces: &[WorkspaceRecord], id: &WorkspaceId) -> String {
    workspaces
        .iter()
        .find(|workspace| workspace.id == *id)
        .map_or_else(|| id.to_string(), |workspace| workspace.shown_name().to_owned())
}

impl Shard {
    /// A device's name, or Cloud for the user's Cloud, as Demi's commands
    /// name it.
    pub(crate) async fn host_name(&self, device: &DeviceId) -> String {
        match self.store().device(device.clone()).await {
            Ok(Some(record)) if record.kind == DeviceKind::Managed => "Cloud".to_owned(),
            Ok(Some(record)) => record.name,
            _ => device.to_string(),
        }
    }
}
