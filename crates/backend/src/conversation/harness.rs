//! The harness the backend's conversations run (`runtime.md` § Sessions and
//! turns). Interim: it stands in for the coding agent's harness,
//! `CodingHarness` of the `coding-agent` crate, which replaces it with its
//! system prompt and the `demi` commands, and the conversation's host access
//! gives its nodes their Hosts and shell environments. Until then a
//! conversation chats without a Host, so its shell tools answer that none is
//! there.

use std::rc::Rc;

use demi_agent::{AgentHarness, EnvironmentScope, PromptContext, ShellEnvironmentFactory};
use demi_host_remote::RemoteHost;
use demi_shell::{CommandSet, HostError, HostErrorKind, ShellEnvironment};
use futures_util::future::LocalBoxFuture;

/// The coding agent's name, which every checkpoint records.
const NAME: &str = "coding";

const SYSTEM_PROMPT: &str = "You are a coding agent. Answer the user's questions about their code.";

/// The interim conversation harness: the coding agent's name, a system
/// prompt, no commands, and no Host.
pub(crate) struct ConversationHarness {
    commands: Rc<CommandSet>,
}

impl ConversationHarness {
    pub(crate) fn new() -> Self {
        Self {
            commands: Rc::new(CommandSet::new()),
        }
    }
}

impl AgentHarness for ConversationHarness {
    type Host = RemoteHost;

    fn name(&self) -> &str {
        NAME
    }

    fn commands(&self) -> Rc<CommandSet> {
        self.commands.clone()
    }

    /// The harness has no commands, so their help is empty.
    async fn system_prompt(&self, _context: PromptContext<'_>, _commands: &str) -> String {
        SYSTEM_PROMPT.to_owned()
    }
}

/// The shell environments of the interim harness's nodes, which have no
/// Host to make them on. Interim: host-remote's factory over the
/// conversation's host access replaces it.
pub(crate) struct NoShellEnvironments;

impl ShellEnvironmentFactory<RemoteHost> for NoShellEnvironments {
    fn create<'a>(
        &'a self,
        _scope: EnvironmentScope<'a>,
        _host: Rc<RemoteHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        Box::pin(async {
            Err(HostError::new(
                HostErrorKind::Unavailable,
                "conversations reach no Host on this backend yet",
            ))
        })
    }
}
