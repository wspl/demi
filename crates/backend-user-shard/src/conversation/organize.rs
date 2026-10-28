//! The product's `demi conversation` group
//! (`crates/backend-host-access/src/host_commands.rs`).
//!
//! An agent renames its conversation at once, without asking: the title is
//! the conversation's own, and the user renames it back as easily. Listing the user's projects only reads, so it asks no permission.
//! Listing the user's conversations, and reading one's transcript
//! (`history.rs`), asks the user's Read Your Other Conversations, since what
//! they read goes to this conversation's model provider. Sending a message to
//! another's main agent asks nothing: the recipient reads it as another
//! agent's words, in a view the user sees. An archived conversation is listed nowhere and receives
//! nothing, so an archive keeps it out of the way. Where the
//! conversation runs decides its project, so an agent moves it into or out of
//! one with `demi host move` (`host_commands.rs`).

use std::future::Future;
use std::rc::{Rc, Weak};

use demi_backend_database::conversation_index::RecordChange;
use demi_backend_host_access::host_commands::conversation_of;
use demi_backend_host_access::root_of;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::output::{self, Cell, Table};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, SenderConversation, Timestamp};
use demi_web_api_protocol::devices::DeviceKind;
use demi_backend_database::workspaces::WorkspaceRecord;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, WorkspaceId};
use demi_web_api_protocol::search::{RESULTS_MAX, SearchText};
use demi_backend_database::Selection;
use demi_web_api_protocol::text::Trimmed;
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::history::{history_group, shell_word};
use super::search::search;
use crate::shard::Shard;

/// The permission category of the commands that read the user's other
/// conversations.
pub(crate) const READ: &str = "conversation.read";

const READ_ACTION: &str = "read your other conversations";

const READ_DESCRIPTION: &str = "Read Your Other Conversations lets the agents of this conversation list your conversations and read what any of them holds. What they read goes to this conversation’s model provider.";

const SUMMARY: &str = "This conversation in the user's sidebar: rename it, and list the user's projects and conversations; send a message to another conversation; and transcripts, which history searches and reads: an agent's of this conversation, or of another conversation of the user.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Renames the conversation, lists projects and the user's conversations, sends messages to other conversations, searches and reads transcripts of any agent, including those of other conversations.";

const RENAME_SUMMARY: &str = "Rename this conversation, as its title in the user's sidebar.";

const PROJECTS_SUMMARY: &str = "List the user's projects of every workspace: id, workspace, Host, directory and name.";

const LIST_SUMMARY: &str = "List the user's conversations that are not archived, newest change first, twenty to a page, of every workspace: id, title, workspace, project and the time of the last change. With words, only those the app's Search finds with them in the title or the messages, fifty at most, which stderr says when more match. A conversation's id goes to history --conversation to read it, and to `send` to message it; stderr names the next page.";

const SEND_SUMMARY: &str = "Send a message to the main agent of another conversation of the user that is not archived, by its id from list. A busy recipient takes it into its running turn; an idle one wakes. Returns once it is accepted, without waiting for an answer: the recipient answers, if it does, by sending to this conversation.";

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

/// The input of a leaf that takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// The `conversation` group, whose handlers act in `shard`.
pub(crate) fn conversation_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("conversation", SUMMARY)
        .index_entry(ENTRY)
        .permission(READ, READ_ACTION, READ_DESCRIPTION)
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
                .permission(READ)
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
    let caller = call
        .invocation
        .caller
        .as_ref()
        .ok_or_else(|| failed("only an agent's command sends a message"))?;
    let content = call.args.message.trim();
    if content.is_empty() {
        return Err(refused("the message is empty"));
    }
    let target = call.args.conversation;
    let control = &shard.services().control;
    let recipient = match control.conversation(target.clone()).await.map_err(failed)? {
        Some(record) if record.owner == *shard.user() => record,
        // Another user's conversation is none of the caller's.
        _ => return Err(failed(format!("no conversation {target} (see `demi conversation list`)"))),
    };
    if recipient.id == here {
        return Err(failed("this is the sending conversation; `demi agent send` reaches its agents"));
    }
    if recipient.archived {
        return Err(failed(format!("conversation {target} is archived, and only the user restores it")));
    }
    let sending = control
        .conversation(here.clone())
        .await
        .map_err(failed)?
        .ok_or_else(|| failed("the sending conversation is gone"))?;
    let tree = shard
        .agent()
        .tree(&root_of(&here))
        .ok_or_else(|| failed("the sending conversation's agents are not running"))?;
    let (mut sender, origin) = tree.sender_of(&caller.node, &caller.tool_use_id).map_err(failed)?;
    sender.conversation = Some(SenderConversation {
        id: root_of(&here),
        title: sending.title.clone(),
    });
    let id = BlockId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
    let timestamp = shard.services().clock.now();
    let message = |recipient: &NodeId| AgentMessage {
        id: id.clone(),
        sender: Some(sender.clone()),
        recipient_id: recipient.clone(),
        timestamp,
        content: content.to_owned(),
        event: AgentMessageEvent::Message {},
        origin: origin.clone(),
    };
    let root = root_of(&recipient.id);
    shard
        .permission_shard()
        .admit(&recipient.id, &root, &message)
        .await
        .map_err(failed)?;
    tracing::info!(
        event = "agent_message.sent",
        conversation = %here,
        from_node = %caller.node,
        to_conversation = %recipient.id,
        kind = "conversation",
        message = %id,
        "an agent's message to another conversation was accepted"
    );
    let title = recipient.title.as_deref().unwrap_or("untitled");
    port.stdout(format!("Sent to \u{201c}{title}\u{201d} ({target}).\n")).await?;
    Ok(0)
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
        .services()
        .control
        .projects(shard.user().clone())
        .await
        .map_err(failed)?;
    let workspaces = shard.services().control.workspaces(shard.user().clone()).await.map_err(failed)?;
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
    let services = shard.services();
    let given = call.args.words.clone().unwrap_or_default();
    let words = given.join(" ");
    // Whether the app's Search stopped at its most: more may match.
    let mut capped = false;
    let mut records = if words.trim().is_empty() {
        services.control.conversations(shard.user().clone(), false).await.map_err(failed)?
    } else {
        let query = SearchText::try_from(words).map_err(|error| refused(error))?;
        let mut records = Vec::new();
        let results = search(services, shard.user(), &query, None).await.map_err(failed)?;
        capped = results.len() >= RESULTS_MAX;
        for found in results {
            if let Some(record) = services.control.conversation(found.conversation_id).await.map_err(failed)?
                && !record.archived
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
    let projects = services.control.projects(shard.user().clone()).await.map_err(failed)?;
    let workspaces = services.control.workspaces(shard.user().clone()).await.map_err(failed)?;
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
        match self.services().control.device(device.clone()).await {
            Ok(Some(record)) if record.kind == DeviceKind::Managed => "Cloud".to_owned(),
            Ok(Some(record)) => record.name,
            _ => device.to_string(),
        }
    }
}
