//! The `demi agent workflows` group (`subagents.md` § Workflows of a
//! project): the project's workflows as the conversation's Workflow menu
//! lists them, and a file checked with the parser that reads them. Its help
//! is where the file format is written.

use std::rc::Weak;

use demi_agent_tools::{HostResolver, WORKFLOWS_HELP, WorkflowListing, parse_workflow};
use demi_host_interface::{GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::WorkflowChoice;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    AgentServer,
    commands::{Invoked, fail, json, out, verb},
};

const WORKFLOWS_GROUP: &str = "The project's workflows, groups of subagent profiles in .demi/workflows/*.yaml that the user picks for a conversation: list them and check a file. Its --help gives the file format and an example.";

const LIST_SUMMARY: &str = "List the workflows of this conversation's project as the composer's Workflow menu does: the one this conversation uses marked with *, and a file that does not read with why. A read.";

const CHECK_SUMMARY: &str = "Check a workflow file with the parser that reads it: prints its name and profiles, or fails naming the line and the field that are wrong. Run it on every workflow file you write or change before you report it.";

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

/// The `workflows` group of every node: reads, which need no permission.
pub(super) fn workflows_group<H: HostResolver>(server: Weak<AgentServer<H>>) -> GroupBuilder {
    GroupBuilder::new("workflows", WORKFLOWS_GROUP)
        .details(WORKFLOWS_HELP)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<ListArgs>()
                .json_output::<WorkflowListing>()
                .bind(TypedRpc::new(verb(server.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("check", CHECK_SUMMARY)
                .input::<CheckArgs>()
                .describe("file", "The workflow file, such as .demi/workflows/review.yaml; relative to your working directory.")
                .positionals(["file"])
                .bind(TypedRpc::new(verb(server, check))),
        )
}

async fn list<H: HostResolver>(call: Invoked<H, ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let listing = match call.tree.workflows().await {
        Ok(listing) => listing,
        Err(error) => return fail(&error),
    };
    if call.json {
        return json(&port, &listing).await;
    }
    out(&port, render(&listing)).await
}

async fn check<H: HostResolver>(call: Invoked<H, CheckArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let file = &call.args.file;
    let text = match call.tree.workflow_file(&call.caller, file).await {
        Ok(text) => text,
        Err(error) => return fail(&error),
    };
    match parse_workflow(&text) {
        Ok(workflow) => {
            let profiles: Vec<&str> = workflow.profiles.iter().map(|profile| profile.id.as_str()).collect();
            let profiles = if profiles.is_empty() {
                "no profiles".to_owned()
            } else {
                format!("profiles {}", profiles.join(", "))
            };
            out(&port, format!("{file} reads: {}, {profiles}.\n", workflow.name)).await
        }
        Err(error) => fail(&format!("{file} does not read: {error}")),
    }
}

/// The text `demi agent workflows list` prints: what the conversation uses,
/// then a workflow per line, its id padded to one column.
fn render(listing: &WorkflowListing) -> String {
    let chosen_id = match &listing.chosen {
        WorkflowChoice::Workflow { id } => Some(id.as_str()),
        WorkflowChoice::Default | WorkflowChoice::None => None,
    };
    let uses = match &listing.chosen {
        WorkflowChoice::Default => "Default, the user's Settings > Subagents".to_owned(),
        WorkflowChoice::None => "No Subagents".to_owned(),
        WorkflowChoice::Workflow { id } => {
            match listing.workflows.iter().find(|workflow| workflow.id == *id) {
                Some(workflow) => match &workflow.name {
                    Some(name) => format!("the workflow {name} ({id})"),
                    None => format!("the workflow {id}, which does not read"),
                },
                None => format!("the workflow {id}, which is no longer in the project"),
            }
        }
    };
    let Some(directory) = &listing.directory else {
        return format!(
            "This project has no workflows: no directory from the working directory up to the repository's root has .demi/workflows/. This conversation uses {uses}.\n"
        );
    };
    let mut text = format!("Workflows in {directory}; this conversation uses {uses}.\n");
    let width = listing
        .workflows
        .iter()
        .map(|workflow| workflow.id.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    for workflow in &listing.workflows {
        let mark = if Some(workflow.id.as_str()) == chosen_id { '*' } else { ' ' };
        let shown = match (&workflow.name, &workflow.error) {
            (_, Some(error)) => format!("does not read: {error}"),
            (Some(name), None) => name.clone(),
            (None, None) => String::new(),
        };
        text.push_str(&format!("{mark} {:width$}{shown}\n", workflow.id));
    }
    text
}
