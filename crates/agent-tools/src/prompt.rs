//! A node's system prompt.
//!
//! It holds these layers, in this order:
//!
//! 1. the identity;
//! 2. the harness guide;
//! 3. the rules of the `shell` tool;
//! 4. the capability index of the node's commands;
//! 5. the model identity;
//! 6. the instructions a subagent profile added.
//!
//! The prompt is rendered once, when the node is assembled, and holds no time,
//! id, Host or state, so the prefix of every request changes only when the
//! node's command set changes, with a Demi release, or on a model switch.
//!
//! - What changes, or differs by Host, user or project, reaches the model as a
//!   `context` block appended to the transcript, never as a change to a layer.
//! - Details reach it when it asks, as a command's `--help` result.
//! - The prompt never lists what the user can change, such as the subagent
//!   profiles, the skills or the models, which would go stale and cost every
//!   request its tokens. It names the command that answers at call time.
//! - Every request carries the whole prompt, so it is kept short.
//!
//! Writing the layers:
//!
//! - The identity says what Demi is, not one job it does. Guidance that holds
//!   only for software is written as a working habit that applies to any work.
//! - A profile's instructions never replace the identity, so a profile's node
//!   is Demi with the same harness, tools, capabilities and model, told what
//!   its kind of work is.
//! - A rule belongs in the harness guide only when its moment arises during
//!   ordinary work rather than while the model reads one command's help;
//!   otherwise it belongs in that command's help.
//! - The model identity line is generated from the node's selection and never
//!   written by hand: a name that does not follow the configuration makes the
//!   model claim to be another.

use demi_agent_session::{PromptLayer, SystemPrompt};
use demi_host_interface::CapabilityIndex;

use crate::input::DESCRIPTION;

/// The rules of the `shell` tool, which every node's system prompt carries:
/// what the tool has and does, in as few words as carry it, never what the
/// model ought to do; each `demi shell` command's help says the rest.
const TOOL_RULES: &[&str] = &[
    "The shell tool:",
    "- Runs a script in your working directory; a call returns when it ends or after 10 s.",
    "- intervalMs: how often a running command reports, 15000–600000; null for a server, which reports only its end.",
    "- Commands in one response run in parallel.",
    "- Results and reports show new output, runningMs and idleMs (time without output).",
    "- `demi shell status|input|output|stop <commandId>`",
    "- A call never holds longer than 10 s, a loop or a sleep in it included: you wait by ending your turn, and a running command's report, a subagent's message or an end wakes you. The user reads only your replies.",
];

/// What the model identity line names of the model that serves a node,
/// from its model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelIdentity<'a> {
    /// The model's display name in its catalog.
    pub name: &'a str,
    /// The provider family of the entry that serves it.
    pub family: &'a str,
    pub id: &'a str,
}

/// The system prompt of a node by its layers.
///
/// The layers are the product's `identity`, the product's harness `guide`, the
/// rules of the `shell` tool, the capability `index` of its commands when it
/// has any, the line naming `model`, and last, under a heading of their own,
/// the instructions its subagent profile `added`, which never replace any of
/// the product's layers.
pub fn system_prompt(
    identity: &str,
    guide: &str,
    index: &CapabilityIndex,
    model: ModelIdentity<'_>,
    added: Option<&str>,
) -> SystemPrompt {
    let mut layers = Vec::new();
    for text in [identity, guide] {
        if !text.trim().is_empty() {
            layers.push(PromptLayer::Text(text.to_owned()));
        }
    }
    layers.push(PromptLayer::Text(format!(
        "{}\n- Tool description: {DESCRIPTION}",
        TOOL_RULES.join("\n")
    )));
    if !index.is_empty() {
        layers.push(PromptLayer::Commands(format!(
            "Capabilities:\n\n{}",
            index.text()
        )));
    }
    layers.push(PromptLayer::Text(format!(
        "This conversation runs on {} ({}, {}). If asked which model you are, answer with this.",
        model.name, model.family, model.id
    )));
    if let Some(added) = added.filter(|added| !added.trim().is_empty()) {
        layers.push(PromptLayer::Text(format!(
            "Instructions for this subagent:\n\n{added}"
        )));
    }
    SystemPrompt { layers }
}
