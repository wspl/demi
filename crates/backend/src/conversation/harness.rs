//! The harness the backend's conversations run (`runtime.md` § Sessions and
//! turns): the coding agent, `CodingHarness`, whose `demi` root carries the
//! backend's `host` group and, when the published `demi.builtin` package
//! serves them, the browser commands. A node's Host is the conversation's
//! current main Host, which the conversation's host access resolves.

use std::rc::{Rc, Weak};

use demi_agent::PromptContext;
use demi_builtin_protocol::browser;
use demi_coding_agent::{BUILTIN_PACKAGE, CodingHarness, DemiOptions, HostResolver};
use demi_host_remote::RemoteHost;
use demi_shell::HostError;

use super::conversation_of;
use crate::runner::host_commands::host_group;
use crate::runner::native::NativeCatalog;
use crate::shard::Shard;

/// The harness of a shard's conversations.
pub(crate) type ConversationHarness = CodingHarness<ShardHosts>;

/// The harness of `shard`'s conversations, whose commands bind to the
/// packages of `native`.
pub(crate) fn conversation_harness(shard: Weak<Shard>, native: &NativeCatalog) -> ConversationHarness {
    let options = DemiOptions {
        browser: native.serves(BUILTIN_PACKAGE, browser::OPERATIONS),
        extra: vec![host_group(shard.clone())],
    };
    CodingHarness::new(ShardHosts { shard }, options).expect("the backend's `demi` groups are named apart from the coding agent's")
}

/// Where a shard's conversations run: each conversation's current main
/// Host. Interim: a target switch's announcement comes with the switch.
pub(crate) struct ShardHosts {
    /// Weak: the shard owns the agent server that holds the harness.
    shard: Weak<Shard>,
}

impl HostResolver for ShardHosts {
    type Host = RemoteHost;

    async fn host(&self, context: PromptContext<'_>) -> Result<Rc<RemoteHost>, HostError> {
        let shard = self
            .shard
            .upgrade()
            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
        shard.conversation_host(&conversation_of(context.root)).await
    }
}
