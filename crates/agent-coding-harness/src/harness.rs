//! `CodingHarness`, the harness Demi's conversations run: the coding agent's
//! name, its system prompt, and the `demi` commands its shell offers. The
//! product says where a node's shell tools run and what tells a node that
//! changed; the harness adds nothing else to the agent runtime.

use std::rc::Rc;

use demi_agent_tools::{AgentHarness, PromptContext};
use demi_host_interface::{CommandSet, Host, HostError, RegisterError};

use crate::{DemiOptions, demi_root};

/// The coding agent's name, which every checkpoint records.
const NAME: &str = "coding";

/// Where a conversation's nodes run: the product's answer to the harness's
/// Host questions (`sessions-and-targets.md` § Host operations).
#[expect(
    async_fn_in_trait,
    reason = "a harness runs on the user's shard: its futures are never sent to another thread"
)]
pub trait HostResolver: 'static {
    type Host: Host;

    /// The Host a node's shell tools reach now: the conversation's current
    /// execution target.
    async fn host(&self, context: PromptContext<'_>) -> Result<Rc<Self::Host>, HostError>;

    /// The text that tells a node the conversation's execution context
    /// changed since it last looked, such as a target switch; none by
    /// default. `seen` is the text of each context block the node's
    /// transcript holds, oldest first.
    async fn context(&self, _context: PromptContext<'_>, _seen: &[&str]) -> Option<String> {
        None
    }
}

/// The coding agent: the `demi` root and a system prompt that teaches the
/// shell tools, over the product's Hosts.
pub struct CodingHarness<R> {
    hosts: R,
    commands: Rc<CommandSet>,
}

impl<R: HostResolver> CodingHarness<R> {
    /// A harness whose nodes run on `hosts` and whose shell offers the
    /// `demi` root `options` describes. It fails when a product group's name
    /// is taken.
    pub fn new(hosts: R, options: DemiOptions) -> Result<Self, RegisterError> {
        let mut commands = CommandSet::new();
        commands.register(demi_root(options))?;
        Ok(Self {
            hosts,
            commands: Rc::new(commands),
        })
    }
}

impl<R: HostResolver> AgentHarness for CodingHarness<R> {
    type Host = R::Host;

    fn name(&self) -> &str {
        NAME
    }

    async fn host(&self, context: PromptContext<'_>) -> Result<Rc<Self::Host>, HostError> {
        self.hosts.host(context).await
    }

    fn commands(&self) -> Rc<CommandSet> {
        self.commands.clone()
    }

    async fn system_prompt(&self, _context: PromptContext<'_>, commands: &str) -> String {
        system_prompt(commands)
    }

    async fn context(&self, context: PromptContext<'_>, seen: &[&str]) -> Option<String> {
        self.hosts.context(context, seen).await
    }
}

/// The system prompt, with the rendered help of the node's commands last.
fn system_prompt(commands: &str) -> String {
    let rules = [
        "Shell session rules:",
        "- Use shell_exec for commands. timeoutMs is required and is only an observation window, not a kill deadline; at timeoutMs the command keeps running and a commandId is returned while the turn continues.",
        "- Tool description: concise title for the concrete user-visible state/result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.",
        "- shell_exec returns shellId, commandId, stdout/stderr deltas, and status. Track commandId for all follow-up control.",
        "- If status is running, the command keeps running and the turn continues. Either call shell_status again to check, or call yield to end this turn and be woken later to check with commandId. shell_exec and shell_status never end the turn or schedule a wakeup on their own; only yield does.",
        "- Use shell_status for polling. It is non-blocking and reads new stdout/stderr since the last snapshot unless offsets are provided.",
        "- For dev servers, watch commands, previews, and other long-running processes that you need to observe and stop, run them in the foreground with a short timeoutMs, use shell_status to observe (and yield to wait between checks), and shell_abort by commandId to stop. Avoid starting them with \"&\" and avoid pkill/killall by process name.",
        "- After a long-running process has been verified and stopped, summarize the observed evidence instead of restarting it to demonstrate the same behavior again.",
        "- If a foreground process is running and you need a separate one-off command, such as curl against a dev server, call shell_exec without shellId; keep using the original commandId to status, write to, or abort the long-running process.",
        "- Prefer non-interactive CLI flags for scaffolds and package tools when available.",
        "- For underspecified scaffold requests, choose a reasonable non-interactive default and proceed unless the choice is destructive or impossible.",
        "- Send non-empty stdin with shell_write only when the running command is known to be waiting for specific input; include a newline such as \"Alice\\n\" for line-oriented prompts.",
        "- For interactive stdin, keep the reader inside one foreground system process such as sh -c, node, or python; do not rely on the session script builtin read across turns.",
        "- Use shell_abort only when intentionally stopping a foreground command, and pass commandId.",
    ]
    .join("\n");
    let mut sections = vec![
        "You are a coding agent. Use shell session tools to inspect, edit, test, and verify the workspace.".to_owned(),
        "Treat cwd as the task workspace. Create, edit, and verify task files there by default; do not create a separate project directory under /tmp or another absolute path unless the user asks for it or the workspace is unusable.".to_owned(),
        "Prefer registered commands for agent-specific state and audited workflows. Use normal system commands for ordinary shell work.".to_owned(),
        rules,
        "File references attached by the client are expanded before provider calls.".to_owned(),
    ];
    if !commands.trim().is_empty() {
        sections.push(format!("Registered commands:\n\n{commands}"));
    }
    sections.join("\n\n")
}
