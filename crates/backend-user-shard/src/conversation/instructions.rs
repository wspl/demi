//! The instructions source: the product's side of the instructions block
//! (`crates/agent-server/src/instructions.rs`).
//!
//! It gives the block:
//!
//! - the user's personal instructions;
//! - the project's instruction files, which the shard keeps for each working
//!   directory and reads again when a runner's watch reports a change;
//! - for the root, the workflow the conversation chose
//!   (`crates/backend-user-shard/src/conversation/workflows.rs`): its
//!   instructions, and its profiles as the only subagents the root can spawn,
//!   so that work none of them is for stays the root's own without a spawn
//!   failing first.

use std::rc::{Rc, Weak};

use demi_agent_server::instructions::{self, NONE_LEFT, ProjectFile, WorkflowText, render};
use demi_agent_tools::{ContextAnswer, ContextSource, NodeContext, WORKFLOW_EXTENSION};
use demi_backend_host_access::conversation_of;
use demi_backend_host_access::kept::Found;
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_runner_protocol::files::{Subject, SubscriptionKind};
use demi_shared_types::{INSTRUCTIONS_SOURCE, TurnId, WorkflowChoice};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use super::project_files::KeptWorkflows;
use crate::shard::Shard;

/// The product's instructions source, asked after the execution context.
pub(crate) struct Instructions {
    pub(crate) shard: Weak<Shard>,
}

impl Instructions {
    pub(crate) fn new(shard: Weak<Shard>) -> Self {
        Self { shard }
    }
}

/// The instructions and profiles of the workflow `id` the conversation
/// chose, the worktree's with `worktree`, as the shard holds its workflows,
/// or that it spawns nothing while it cannot be used, as its spawns fail;
/// none before the shard holds them.
async fn workflow_text(shard: &Shard, conversation: &ConversationId, id: &str, worktree: bool) -> Option<WorkflowText> {
    let found = match shard.kept_workflows(conversation, worktree).await {
        Ok(KeptWorkflows::Found(found)) => found,
        Ok(KeptWorkflows::None) => return None,
        Err(error) => {
            tracing::warn!(event = "project_workflows.read_failed", %conversation, %error, "the project's workflows were not taken");
            return None;
        }
    };
    let file = found.files.iter().find(|file| file.id == id);
    if let Some(reason) = found.unavailable(id) {
        let path = file.map(|file| file.path.clone()).or_else(|| {
            found.directory.as_ref().map(|directory| format!("{directory}/{id}{WORKFLOW_EXTENSION}"))
        });
        return Some(WorkflowText {
            name: id.to_owned(),
            path: path.unwrap_or_default(),
            text: format!("You can spawn no subagents while this workflow cannot be used: {reason}."),
        });
    }
    let file = file.expect("an available workflow is listed");
    let workflow = file.read.as_ref().ok().expect("an available workflow reads");
    let mut text = workflow
        .instructions
        .as_deref()
        .map(str::trim_end)
        .filter(|instructions| !instructions.trim().is_empty())
        .map(|instructions| format!("{instructions}\n\n"))
        .unwrap_or_default();
    if workflow.profiles.is_empty() {
        text.push_str("You can spawn no subagents.");
    } else {
        text.push_str("The only subagents you can spawn, with demi agent spawn --profile <id>:");
        for profile in &workflow.profiles {
            text.push_str(&format!("\n- {}: {}", profile.id, profile.description));
        }
    }
    Some(WorkflowText {
        name: workflow.name.clone(),
        path: file.path.clone(),
        text,
    })
}

impl ContextSource for Instructions {
    fn name(&self) -> &str {
        INSTRUCTIONS_SOURCE
    }

    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        _: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            let personal = shard
                .services()
                .control
                .instructions(shard.user().clone())
                .await
                .map_err(|error| format!("the personal instructions cannot be read: {error}"))?;
            let cwd = shard
                .node_directory(node)
                .await
                .map_err(|error| format!("the working directory cannot be read: {error}"))?;
            let conversation = conversation_of(node.root);
            // The workflow's element is the root's alone. Its
            // discovery and the files' start together, so their first
            // searches share one hold.
            let chosen = if node.node == node.root {
                match shard.workflow_choice(&conversation).await? {
                    WorkflowChoice::Workflow { id, worktree } => Some((id, worktree)),
                    WorkflowChoice::Default | WorkflowChoice::None => None,
                }
            } else {
                None
            };
            let workflow = async {
                match &chosen {
                    Some((id, worktree)) => workflow_text(&shard, &conversation, id, *worktree).await,
                    None => None,
                }
            };
            let (files, workflow) = futures_util::join!(shard.kept_instructions(&conversation, &cwd), workflow);
            let answer = match render(&personal, &files, workflow.as_ref()) {
                Some(answer) => answer,
                None if seen.last().is_some_and(|newest| *newest != NONE_LEFT) => {
                    ContextAnswer::from(NONE_LEFT.to_owned())
                }
                None => return Ok(None),
            };
            Ok((seen.last() != Some(&answer.text.as_str())).then_some(answer))
        })
    }
}

/// The project files for a node in `cwd` on the conversation's Host
/// `device`, root first, whole, with what to watch
/// (`crates/agent-server/src/instructions.rs`).
pub(crate) async fn search_instructions(
    shard: &Shard,
    conversation: &ConversationId,
    device: &DeviceId,
    cwd: &str,
    cancel: &CancellationToken,
) -> Result<Found<Rc<[ProjectFile]>>, ReadFilesError> {
    let paths = shard
        .host_paths(conversation)
        .await
        .map_err(|error| ReadFilesError::Access(error.into()))?;
    let host = shard.host_shard();
    let found = instructions::search(paths, cwd, |reads| async move {
        let (answers, _) = host.read_files_on(conversation, Some(device), &reads, cancel).await?;
        Ok::<_, ReadFilesError>(answers)
    })
    .await?;
    for (path, reason) in &found.unreadable {
        tracing::info!(event = "instruction_file.unreadable", %conversation, %path, %reason, "a project instruction file cannot be read");
    }
    for path in &found.growing {
        tracing::info!(event = "instruction_file.left_out", %conversation, %path, reason = "growing", "a project instruction file kept growing while it was read; it is left out until it next changes");
    }
    let watch = found
        .watch
        .into_iter()
        .map(|path| Subject {
            kind: SubscriptionKind::File,
            path,
            suffix: None,
        })
        .collect();
    Ok(Found {
        value: found.files.into(),
        watch,
    })
}
