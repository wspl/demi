//! The `demi agent workflows` group
//! (`crates/backend-user-shard/src/conversation/workflows.rs`).
//!
//! It lists the project's workflows as the conversation's Workflow menu lists
//! them, checks a file with the parser that reads them, and picks one for the
//! conversation as the menu does. Its help is where the file format is
//! written.
//!
//! A pick needs the user's permission in each conversation, the category
//! [`WORKFLOW_CATEGORY`] names, because a workflow applies only when the user
//! agrees in that conversation: a repository is often someone else's.

use std::rc::Weak;

use demi_agent_tools::{HostResolver, WORKFLOWS_HELP, WorkflowListing, parse_workflow};
use demi_host_interface::{GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::{
    WorkflowChoice,
    output::{Cell, Fields, Table, warning},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    AgentServer,
    commands::{Invoked, fail, json, out, verb},
};

/// The permission category `demi agent workflows use` needs, which the
/// `agent` group declares: its id, its action and what a grant allows.
pub(super) const WORKFLOW_CATEGORY: (&str, &str, &str) = (
    "agent.workflow",
    "change its workflow",
    "Choose the workflow this conversation works with, as the composer's Workflow menu does.",
);

/// The `use` values of the menu's two items that are no project workflow.
const GLOBAL: &str = "global";
const NONE: &str = "none";

const WORKFLOWS_GROUP: &str = "The project's workflows, groups of subagent profiles in .demi/workflows/*.yaml picked for a conversation in the composer's Workflow menu: list them, check a file, and pick one for this conversation. Its --help gives the file format and an example.";

const LIST_SUMMARY: &str = "List the workflows of this conversation's project as the composer's Workflow menu does, CURRENT naming the one this conversation uses; a warning on stderr says why a file cannot be used. A read.";

const CHECK_SUMMARY: &str = "Check a workflow file with the parser that reads it: prints its name and profiles, with a warning on stderr when the name does not read as a menu item, or fails naming the line and the field that are wrong. Run it on every workflow file you write or change before you report it.";

const USE_SUMMARY: &str = "Pick this conversation's workflow as the composer's Workflow menu does, which shows it at once: a workflow by its id, global for Global Workflow (the user's own profiles) or none for No Subagents. Spawns take its profiles from then on, and its instructions reach the leading agent from its next request. A workflow that cannot be used fails with why and the choice stays. Needs the user's permission.";

/// The input of `demi agent workflows use`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UseArgs {
    workflow: String,
}

/// The input of `demi agent workflows list`, which takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {}

/// The input of `demi agent workflows check`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckArgs {
    file: String,
}

/// The `workflows` group of every node: reads, which need no permission,
/// and the pick, under the category the `agent` group declares.
pub(super) fn workflows_group<H: HostResolver>(server: Weak<AgentServer<H>>) -> GroupBuilder {
    let (category, _, _) = WORKFLOW_CATEGORY;
    GroupBuilder::new("workflows", WORKFLOWS_GROUP)
        .details(WORKFLOWS_HELP)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<ListArgs>()
                .json_output::<Vec<ListedEntry>>()
                .bind(TypedRpc::new(verb(server.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("check", CHECK_SUMMARY)
                .input::<CheckArgs>()
                .describe("file", "The workflow file, such as .demi/workflows/review.yaml, absolute or relative to the shell's current directory.")
                .positionals(["file"])
                .json_output::<Checked>()
                .bind(TypedRpc::new(verb(server.clone(), check))),
        )
        .leaf(
            LeafBuilder::rpc("use", USE_SUMMARY)
                .input::<UseArgs>()
                .describe("workflow", "A workflow's id, as demi agent workflows list lists it; global for Global Workflow; none for No Subagents.")
                .positionals(["workflow"])
                .json_output::<Used>()
                .permission(category)
                .bind(TypedRpc::new(verb(server, use_workflow))),
        )
}

/// `demi agent workflows use --json`: the conversation's choice now.
#[derive(Serialize, JsonSchema)]
struct Used {
    /// The workflow's id, global or none.
    workflow: String,
    /// What the Workflow menu shows.
    name: String,
}

/// One workflow of `demi agent workflows list --json`.
#[derive(Serialize, JsonSchema)]
struct ListedEntry {
    id: String,
    /// What the menu shows; null for a file that cannot be used.
    name: Option<String>,
    /// The file on the Host.
    path: String,
    /// Whether this conversation uses it.
    current: bool,
    /// Why it cannot be used; null while it can.
    unavailable: Option<String>,
}

/// `demi agent workflows check --json`: what the file reads as.
#[derive(Serialize, JsonSchema)]
struct Checked {
    file: String,
    name: String,
    profiles: Vec<CheckedProfile>,
    /// Why the name does not read as a menu item; null when it does.
    warning: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct CheckedProfile {
    id: String,
    description: String,
}

async fn list<H: HostResolver>(call: Invoked<H, ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let listing = match call.tree.workflows().await {
        Ok(listing) => listing,
        Err(error) => return fail(&error),
    };
    for note in notes(&listing) {
        port.stderr(warning(&call.command, &note)).await?;
    }
    let chosen = match &listing.chosen {
        WorkflowChoice::Workflow { id } => Some(id.as_str()),
        WorkflowChoice::Default | WorkflowChoice::None => None,
    };
    let entries: Vec<ListedEntry> = listing
        .workflows
        .iter()
        .map(|workflow| ListedEntry {
            id: workflow.id.clone(),
            name: workflow.name.clone(),
            path: workflow.path.clone(),
            current: Some(workflow.id.as_str()) == chosen,
            unavailable: workflow.unavailable.clone(),
        })
        .collect();
    if call.json {
        return json(&port, &entries).await;
    }
    let mut table = Table::new(["id", "current", "status", "path", "name"]);
    for entry in &entries {
        table.row::<Cell>([
            entry.id.as_str().into(),
            entry.current.into(),
            if entry.unavailable.is_some() { "unavailable" } else { "available" }.into(),
            entry.path.as_str().into(),
            entry.name.as_deref().into(),
        ]);
    }
    out(&port, table.render()).await
}

async fn use_workflow<H: HostResolver>(call: Invoked<H, UseArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let workflow = call.args.workflow.clone();
    let choice = match workflow.as_str() {
        GLOBAL => WorkflowChoice::Default,
        NONE => WorkflowChoice::None,
        id => WorkflowChoice::Workflow { id: id.to_owned() },
    };
    let name = match call.tree.choose_workflow(choice).await {
        Ok(Some(name)) => name,
        Ok(None) if workflow == GLOBAL => "Global Workflow".to_owned(),
        Ok(None) => "No Subagents".to_owned(),
        Err(error) => return fail(&error),
    };
    let used = Used { workflow, name };
    if call.json {
        return json(&port, &used).await;
    }
    let mut fields = Fields::new();
    fields
        .field("Workflow", used.workflow.as_str())
        .field("Name", used.name.as_str());
    out(&port, fields.render()).await
}

async fn check<H: HostResolver>(call: Invoked<H, CheckArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let file = &call.args.file;
    let text = match call.tree.workflow_file(&call.cwd, file).await {
        Ok(text) => text,
        Err(error) => return fail(&error),
    };
    let workflow = match parse_workflow(&text) {
        Ok(workflow) => workflow,
        Err(error) => return fail(&format!("{file} does not read: {error}")),
    };
    let checked = Checked {
        file: file.clone(),
        name: workflow.name.clone(),
        profiles: workflow
            .profiles
            .iter()
            .map(|profile| CheckedProfile {
                id: profile.id.clone(),
                description: profile.description.clone(),
            })
            .collect(),
        warning: workflow.name_warning(),
    };
    if let Some(note) = &checked.warning {
        port.stderr(warning(&call.command, note)).await?;
    }
    if call.json {
        return json(&port, &checked).await;
    }
    let mut profiles = Table::new(["id", "description"]);
    for profile in &checked.profiles {
        profiles.row([profile.id.as_str(), profile.description.as_str()]);
    }
    let mut fields = Fields::new();
    fields
        .field("File", checked.file.as_str())
        .field("Name", checked.name.as_str())
        .table("Profiles", &profiles);
    out(&port, fields.render()).await
}

/// What `demi agent workflows list` tells on stderr: that the project has
/// no workflows or cannot use them, that this conversation's workflow is no
/// longer in the project, and why each file that cannot be used cannot.
fn notes(listing: &WorkflowListing) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(directory) = &listing.directory else {
        notes.push("this project has no workflows: no directory from the working directory up to the repository's root has .demi/workflows/".to_owned());
        return notes;
    };
    if let Some(reason) = &listing.unavailable {
        notes.push(format!("the workflows in {directory} cannot be used: {reason}"));
    }
    if let WorkflowChoice::Workflow { id } = &listing.chosen
        && !listing.workflows.iter().any(|workflow| workflow.id == *id)
    {
        notes.push(format!("this conversation's workflow {id} is no longer in the project"));
    }
    for workflow in &listing.workflows {
        if let Some(reason) = &workflow.unavailable {
            notes.push(format!("{} cannot be used: {reason}", workflow.id));
        }
    }
    notes
}
