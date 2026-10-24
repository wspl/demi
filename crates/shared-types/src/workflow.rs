//! A conversation's choice in its Workflow menu
//! (`crates/backend-user-shard/src/conversation/workflows.rs`), which the
//! conversation keeps as it keeps its model.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a conversation's subagents spawn with: the user's Subagents
/// settings, none, or the profiles of one of the project's workflows.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowChoice {
    /// The user's Subagents settings, which a new conversation starts with.
    #[default]
    Default,
    /// Subagents are off for this conversation.
    None,
    /// The workflow whose file is `.demi/workflows/<id>.yaml`: the
    /// project's, or with `worktree` the one of the linked worktree the
    /// conversation works in (`crates/backend-database/src/selection.rs`
    /// § Worktrees). A conversation in no worktree reads its own directory's
    /// either way.
    Workflow {
        #[garde(length(chars, min = 1, max = 255), custom(file_stem))]
        id: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[garde(skip)]
        worktree: bool,
    },
}

/// A workflow's id is a file's name without `.yaml`: no separator, and no
/// leading dot.
fn file_stem(id: &str, _: &()) -> garde::Result {
    if id.starts_with('.') || id.contains(['/', '\\', '\0']) {
        return Err(garde::Error::new("is not the name of a workflow file"));
    }
    Ok(())
}
