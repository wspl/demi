//! A node's system prompt (`system-prompt.md`): the identity, the harness
//! guide, the rules of the standard tools, the capability index of the
//! node's commands and the model identity, in that order.

use crate::input::DESCRIPTION;

/// The rules of the standard tools, which every node's system prompt
/// carries.
const TOOL_RULES: &[&str] = &[
    "Shell session rules:",
    "- Use shell_exec for commands. timeoutMs is required and is only an observation window, not a kill deadline; at timeoutMs the command keeps running and a commandId is returned while the turn continues.",
    "- shell_exec returns status, commandId and the output so far; while the command runs, also shellId, runningMs and idleMs. Track commandId for all follow-up control.",
    "- If status is running, the command keeps running and the turn continues. Either call shell_status again to check, or call yield to end this turn and be woken later to check with commandId. shell_exec and shell_status never end the turn or schedule a wakeup on their own; only yield does.",
    "- Use shell_status for polling. It is non-blocking and shows the output since your last look. Past the first 8 KiB of a running command's stream, a line counts the bytes left out and that stream's newest lines follow.",
    "- A long output shows its first and last lines; the line between them names the demi shell output command that reads what it leaves out.",
    "- For dev servers, watch commands, previews, and other long-running processes that you need to observe and stop, run them in the foreground with a short timeoutMs, use shell_status to observe (and yield to wait between checks), and shell_abort by commandId to stop. Avoid starting them with \"&\" and avoid pkill/killall by process name.",
    "- After a long-running process has been verified and stopped, summarize the observed evidence instead of restarting it to demonstrate the same behavior again.",
    "- If a foreground process is running and you need a separate one-off command, such as curl against a dev server, call shell_exec without shellId; keep using the original commandId to status, write to, or abort the long-running process.",
    "- Prefer non-interactive CLI flags for scaffolds and package tools when available.",
    "- For underspecified scaffold requests, choose a reasonable non-interactive default and proceed unless the choice is destructive or impossible.",
    "- Send non-empty stdin with shell_write only when the running command is known to be waiting for specific input; include a newline such as \"Alice\\n\" for line-oriented prompts.",
    "- For interactive stdin, keep the reader inside one foreground system process such as sh -c, node, or python; do not rely on the session script builtin read across turns.",
    "- Use shell_abort only when intentionally stopping a foreground command, and pass commandId.",
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
