//! A node's system prompt.
//!
//! It holds these layers, in this order:
//!
//! 1. the identity;
//! 2. the guidelines;
//! 3. the capability index of the node's commands;
//! 4. the model identity;
//! 5. the instructions a subagent profile added.
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
//! - Every request carries the whole prompt, so it is kept short, as pi's
//!   is: a fact is said once, where it belongs. What the `shell` tool does is
//!   its definition's alone; a result and a report carry data, not rules.
//!
//! Writing the layers:
//!
//! - The identity says what Demi is, not one job it does. Guidance that holds
//!   only for software is written as a working habit that applies to any work.
//! - A profile's instructions never replace the identity, so a profile's node
//!   is Demi with the same harness, tools, capabilities and model, told what
//!   its kind of work is.
//! - A guideline is only what the model cannot tell from its tools and what
//!   the product's display needs; anything about one command is that
//!   command's help.
//! - The model identity line is generated from the node's selection and never
//!   written by hand: a name that does not follow the configuration makes the
//!   model claim to be another.

use demi_agent_session::{PromptLayer, SystemPrompt};
use demi_host_interface::CapabilityIndex;

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
/// The layers are the product's `identity`, the product's `guide`, the
/// capability `index` of its commands when it has any, the line naming
/// `model`, and last, under a heading of their own,
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
    if !index.is_empty() {
        layers.push(PromptLayer::Commands(index.text()));
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
