//! The product's `demi conversation` group (`sessions-and-targets.md`
//! § What the agent can change): with the user's Organize Conversations, an
//! agent renames its conversation at once and lists the user's projects.
//! Where the conversation runs decides its project, so an agent moves it
//! into or out of one with `demi host move` (`host_commands.rs`).

use std::future::Future;
use std::rc::{Rc, Weak};

use demi_backend_database::conversation_index::RecordChange;
use demi_backend_host_access::host_commands::conversation_of;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_web_api_protocol::conversations::TITLE_MAX;
use demi_web_api_protocol::text::Trimmed;
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;

use super::history::history_group;
use crate::shard::Shard;

/// The permission category of the commands that organize the conversation.
pub(crate) const ORGANIZE: &str = "conversation.organize";

const ORGANIZE_ACTION: &str = "organize conversations";

const ORGANIZE_DESCRIPTION: &str = "Organize Conversations lets the agents of this conversation rename it and list your projects.";

const SUMMARY: &str = "This conversation in the user's sidebar: rename it, and list the user's projects; and your own history in it, which history searches and reads.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Renames this conversation and lists the user's projects, each a directory on a Host; `demi host move` to a project's directory puts this conversation in it. Use it when the user asks to organize the conversation. Its history commands search and read your own earlier history, such as what a compaction summarized.";

const RENAME_SUMMARY: &str = "Rename this conversation, as its title in the user's sidebar.";

const PROJECTS_SUMMARY: &str = "The user's projects: name, id, Host, directory.";


/// The input of `demi conversation rename`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameArgs {
    /// The new title
    #[schemars(length(min = 1, max = TITLE_MAX))]
    title: String,
}

/// The input of a leaf that takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// The `conversation` group, whose handlers act in `shard`.
pub(crate) fn conversation_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("conversation", SUMMARY)
        .index_entry(ENTRY)
        .permission(ORGANIZE, ORGANIZE_ACTION, ORGANIZE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("rename", RENAME_SUMMARY)
                .input::<RenameArgs>()
                .positionals(["title"])
                .permission(ORGANIZE)
                .bind(TypedRpc::new(verb(shard.clone(), rename))),
        )
        .leaf(
            LeafBuilder::rpc("projects", PROJECTS_SUMMARY)
                .input::<NoArgs>()
                .permission(ORGANIZE)
                .bind(TypedRpc::new(verb(shard.clone(), projects))),
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

async fn rename(shard: Rc<Shard>, call: Call<RenameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let title = Trimmed::from(call.args.title).into_string();
    if title.is_empty() {
        return Err(failed("the title is blank"));
    }
    shard
        .transition(&conversation, RecordChange::Title(title.clone()).into())
        .await
        .map_err(failed)?;
    port.stdout(format!("Renamed this conversation to \u{201c}{title}\u{201d}.\n"))
        .await?;
    Ok(0)
}

async fn projects(shard: Rc<Shard>, _call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let workspaces = shard
        .services()
        .control
        .workspaces(shard.user().clone())
        .await
        .map_err(failed)?;
    if workspaces.is_empty() {
        port.stdout("The user has no projects.\n").await?;
        return Ok(0);
    }
    let mut lines = Vec::new();
    for workspace in &workspaces {
        let host = shard.host_name(&workspace.device).await;
        lines.push(format!(
            "{}  {}  {host}  {}",
            workspace.name, workspace.id, workspace.path
        ));
    }
    port.stdout(format!("{}\n", lines.join("\n"))).await?;
    Ok(0)
}
