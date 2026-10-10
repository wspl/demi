//! A node's system prompt (`system-prompt.md`): the identity, the harness
//! guide, the rules of the `shell` tool, the capability index of the node's
//! commands and the model identity, in that order.

use crate::input::DESCRIPTION;

/// The rules of the `shell` tool, which every node's system prompt carries.
const TOOL_RULES: &[&str] = &[
    "The shell tool:",
    "- Use shell to run a script. Every call starts in the conversation's working directory. intervalMs says how to watch the command: a number from 15000 to 600000 for a command that ends, such as a build or a test suite; the call watches it that long, and if it still runs then, the call returns its commandId and the command reports to you every interval until it ends. null for a command that runs until it is stopped, such as a dev server or a watcher: the call returns once its start-up output is quiet, and the command reports only its end.",
    "- Commands in one response run at the same time. Put commands that depend on each other in one script joined with &&, or in separate responses.",
    "- A result shows the output since your last look at the command; while it runs, also runningMs and idleMs. A long output shows its first and last lines, and the line between names the demi shell output command that reads the rest. The output is already bounded this way, so do not pipe a command into head or tail to shorten it.",
    "- To look at a running command again, run demi shell status <commandId>; --wait <duration> waits up to that long for it to end first. To answer a prompt the command waits for, run demi shell input <commandId> with the answer on stdin, with a newline for a line-based prompt. Change how often a command reports with demi shell status <commandId> --interval <duration>, or --resident to hear only of its end. Stop a command with demi shell stop <commandId>, never with pkill or killall.",
    "- When nothing is left to do but wait for a command or a subagent, end your turn with a reply that says what you are waiting for: its report or its completion wakes you, and the user reads only what you wrote. Do not poll. To come back after a while for something no command stands for, start sleep with an intervalMs and end your turn.",
    "- Run dev servers, watchers and previews with intervalMs null, not with \"&\".",
    "- Once a long-running process has been checked and stopped, report what you observed instead of running it again to show the same thing.",
    "- Prefer non-interactive flags. For an underspecified scaffold, choose a reasonable non-interactive default unless the choice is destructive or impossible.",
    "- For interactive stdin, keep the reader inside one process such as sh -c, node or python; the shell's own read builtin does not keep its input across calls.",
];

/// What the model identity line names of the model that serves a node,
/// from its model selection (`system-prompt.md` § Model identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelIdentity<'a> {
    /// The model's display name in its catalog.
    pub name: &'a str,
    /// The provider family of the entry that serves it.
    pub family: &'a str,
    pub id: &'a str,
}

/// The system prompt of a node: its `identity` (the product's instructions
/// or a profile's), the product's harness `guide`, the rules of the
/// `shell` tool, the capability `index` of its commands when it has any,
/// and the line naming `model`, last.
pub fn system_prompt(
    identity: &str,
    guide: &str,
    index: &str,
    model: ModelIdentity<'_>,
) -> String {
    let mut sections = Vec::new();
    for layer in [identity, guide] {
        if !layer.trim().is_empty() {
            sections.push(layer.to_owned());
        }
    }
    sections.push(format!(
        "{}\n- Tool description: {DESCRIPTION}",
        TOOL_RULES.join("\n")
    ));
    if !index.trim().is_empty() {
        sections.push(format!("Capabilities:\n\n{index}"));
    }
    sections.push(format!(
        "This conversation runs on {} ({}, {}). If asked which model you are, answer with this.",
        model.name, model.family, model.id
    ));
    sections.join("\n\n")
}
