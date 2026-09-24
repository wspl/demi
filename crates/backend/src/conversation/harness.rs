//! The harness the backend's conversations run (`runtime.md` § Sessions and
//! turns). Interim: it stands in for the coding agent's harness,
//! `CodingHarness` of the `coding-agent` crate, which replaces its system
//! prompt and gives it the `demi` commands. A node's Host is the
//! conversation's current main Host, which the conversation's host access
//! resolves.

use std::rc::{Rc, Weak};

use demi_agent::{AgentHarness, PromptContext};
use demi_host_remote::RemoteHost;
use demi_shell::{CommandSet, GroupBuilder, HostError};

use super::conversation_of;
use crate::runner::host_commands::host_group;
use crate::shard::Shard;

/// The coding agent's name, which every checkpoint records.
const NAME: &str = "coding";

const SYSTEM_PROMPT: &str = "You are a coding agent. Answer the user's questions about their code.";

/// The interim conversation harness: the coding agent's name, a system
/// prompt, the backend's `demi host` commands, and the conversation's Host.
pub(crate) struct ConversationHarness {
    commands: Rc<CommandSet>,
    /// Weak: the shard owns the agent server that holds this harness.
    shard: Weak<Shard>,
}

impl ConversationHarness {
    /// The harness of `shard`'s conversations, whose `demi` commands are the
    /// backend's `host` group.
    pub(crate) fn new(shard: Weak<Shard>) -> Self {
        let mut commands = CommandSet::new();
        commands
            .register(GroupBuilder::new("demi", "Demi agent runtime commands.").group(host_group(shard.clone())))
            .expect("the demi host commands are valid");
        Self {
            commands: Rc::new(commands),
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

    /// The interim prompt leaves out the commands' help.
    async fn system_prompt(&self, _context: PromptContext<'_>, _commands: &str) -> String {
        SYSTEM_PROMPT.to_owned()
    }
}
