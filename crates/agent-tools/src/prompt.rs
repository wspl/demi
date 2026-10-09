//! A node's system prompt (`system-prompt.md`): the identity, the harness
//! guide, the rules of the standard tools, the capability index of the
//! node's commands and the model identity, in that order.

use crate::input::DESCRIPTION;

/// The rules of the standard tools, which every node's system prompt
/// carries.
const TOOL_RULES: &[&str] = &[
    "Shell tools:",
    "- Use shell_exec to run a script. timeoutMs is how long to watch, not a deadline: when it passes, or when the user steers, the command keeps running and the result carries its commandId.",
    "- Commands in one response run at the same time. Put commands that depend on each other in one script joined with &&, or in separate responses.",
    "- A result shows the output since your last look at the command; while it runs, also runningMs and idleMs. A long output shows its first and last lines, and the line between names the demi shell output command that reads the rest. The output is already bounded this way, so do not pipe a command into head or tail to shorten it.",
    "- To look at a running command again, call shell_status with its commandId: with timeoutMs it waits up to that long for the command to end; without it, it looks at once. To answer a prompt the command waits for, pass stdin, with a newline for a line-based prompt.",
    "- To wait longer, or to let the user talk to you meanwhile, call yield with durationMs and the commandIds to wait for: you are woken when the first of them ends or the time passes. yield ends your turn, and the user reads only the text you wrote before it, so answer the user, or say what you are waiting for, before you yield.",
    "- Run dev servers, watchers and previews in the foreground with a short timeoutMs, not with \"&\". Stop a command with demi shell stop <commandId>, never with pkill or killall.",
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
/// standard tools, the capability `index` of its commands when it has any,
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
