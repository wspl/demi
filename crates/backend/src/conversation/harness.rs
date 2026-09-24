//! The harness the backend's conversations run (`runtime.md` § Sessions and
//! turns). Interim: it stands in for the coding agent's harness,
//! `CodingHarness` of the `coding-agent` crate, which replaces its system
//! prompt and gives it the `demi` commands. A node's Host is the
//! conversation's current main Host, which the conversation's host access
//! resolves.

use std::rc::{Rc, Weak};

use demi_agent::{AgentHarness, PromptContext};
use demi_host_remote::RemoteHost;
use demi_shell::{CommandSet, HostError};

use super::conversation_of;
use crate::shard::Shard;

/// The coding agent's name, which every checkpoint records.
const NAME: &str = "coding";

const SYSTEM_PROMPT: &str = "You are a coding agent. Answer the user's questions about their code.";

/// The interim conversation harness: the coding agent's name, a system
/// prompt, no commands, and the conversation's Host.
pub(crate) struct ConversationHarness {
    commands: Rc<CommandSet>,
    /// Weak: the shard owns the agent server that holds this harness.
    shard: Weak<Shard>,
}

impl ConversationHarness {
    pub(crate) fn new(shard: Weak<Shard>) -> Self {
        Self {
            commands: Rc::new(CommandSet::new()),
            shard,
        }
    }
}

impl AgentHarness for ConversationHarness {
    type Host = RemoteHost;

    fn name(&self) -> &str {
        NAME
    }

    async fn host(&self, context: PromptContext<'_>) -> Result<Rc<RemoteHost>, HostError> {
        let shard = self
            .shard
            .upgrade()
            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
        shard.conversation_host(&conversation_of(context.root)).await
    }

    fn commands(&self) -> Rc<CommandSet> {
        self.commands.clone()
    }

    /// The harness has no commands, so their help is empty.
    async fn system_prompt(&self, _context: PromptContext<'_>, _commands: &str) -> String {
        SYSTEM_PROMPT.to_owned()
    }
}
