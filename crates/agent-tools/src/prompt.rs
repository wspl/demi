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

/// The rules of the `shell` tool, which every node's system prompt carries.
///
/// Waiting is ending the turn: a call that watches a `sleep` holds the turn
/// while the user watches the agent wait, and the work's end wakes the model
/// anyway. Piping into `head` or `tail` is discouraged because it hides the
/// command's exit status behind the pipe's, hides a long command's output until
/// it ends and turns a returned image into a fragment of its bytes.
const TOOL_RULES: &[&str] = &[
    "The shell tool:",
    "- Use shell to run a script. Every call starts in your working directory. intervalMs says how often a command still running when its call returns reports to you: a number from 15000 to 600000 for a command that ends, such as a build or a test suite, which reports every interval until it ends, and when it ends. null for a command that runs until it is stopped, such as a dev server or a watcher: the call returns once its start-up output is quiet, and the command reports only its end.",
    "- Commands in one response run at the same time. Put commands that depend on each other in one script joined with &&, or in separate responses.",
    "- A result shows the output since your last look at the command; while it runs, also runningMs and idleMs. A long output shows its first and last lines, and the line between names the demi shell output command that reads the rest. The output is already bounded this way, so do not pipe a command into head or tail to shorten it.",
    "- To look at a running command again, run demi shell status <commandId>; --wait <duration> waits up to that long for it to end first. To answer a prompt the command waits for, run demi shell input <commandId> with the answer on stdin, with a newline for a line-based prompt. Change how often a command reports with demi shell status <commandId> --interval <duration>, or --resident to hear only of its end. Stop a command with demi shell stop <commandId>, never with pkill or killall.",
    "- When nothing is left to do but wait for a command or a subagent, end your turn with a reply that says what you are waiting for: its report or its completion wakes you, and the user reads only what you wrote. Do not poll. To come back after a while for something no command stands for, start sleep with intervalMs null, which returns at once, and end your turn: the sleep's end wakes you. In a script, never sleep a fixed time before looking for something; wait for the thing itself, such as a line in a log (tail -f app.log | grep -m1 ready) or the command whose end you need.",
    "- Run dev servers, watchers and previews with intervalMs null, not with \"&\".",
    "- Once a long-running process has been checked and stopped, report what you observed instead of running it again to show the same thing.",
    "- Prefer non-interactive flags. For an underspecified scaffold, choose a reasonable non-interactive default unless the choice is destructive or impossible.",
    "- For interactive stdin, keep the reader inside one process such as sh -c, node or python; the shell's own read builtin does not keep its input across calls.",
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
