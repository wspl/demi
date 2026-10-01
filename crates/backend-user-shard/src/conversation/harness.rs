//! The harness the backend's conversations run (`runtime.md` § Sessions and
//! turns): the coding agent, `CodingHarness`, whose `demi` root carries the
//! backend's `host` group and, when the published `demi.file` and
//! `demi.browser` packages serve them, the file and browser commands. A node's Host is the
//! conversation's current main Host, which the conversation's host access
//! resolves.

use std::rc::{Rc, Weak};

use demi_agent_coding_harness::{CodingHarness, DemiOptions, HostResolver};
use demi_agent_tools::PromptContext;
use demi_backend_remote_host::RemoteHost;
use demi_backend_runners::native::NativeCatalog;
use demi_command_package_browser_protocol::{PACKAGE as BROWSER, browser};
use demi_command_package_file_protocol::{OPERATIONS as FILE_OPERATIONS, PACKAGE as FILE};
use demi_host_interface::HostError;

use crate::shard::Shard;
use demi_backend_host_access::host_commands::host_group;
use demi_backend_host_access::{HostShard, conversation_of};

/// The harness of a shard's conversations.
pub(crate) type ConversationHarness = CodingHarness<ShardHosts>;

/// The harness of `shard`'s conversations, whose commands bind to the
/// packages of `native`.
pub(crate) fn conversation_harness(
    shard: Weak<Shard>,
    native: &NativeCatalog,
) -> ConversationHarness {
    let hosts: Weak<dyn HostShard> = shard.clone();
    let options = DemiOptions {
        file: native.serves(FILE, FILE_OPERATIONS),
        browser: native.serves(BROWSER, browser::OPERATIONS),
        extra: vec![host_group(hosts)],
    };
    CodingHarness::new(ShardHosts { shard }, options)
        .expect("the backend's `demi` groups are named apart from the coding agent's")
}

/// Where a shard's conversations run: each conversation's current main
/// Host, and the context block that tells a node the conversation's
/// execution context changed.
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
        shard
            .host_shard()
            .conversation_host(&conversation_of(context.root))
            .await
    }

    async fn context(&self, context: PromptContext<'_>, seen: &[&str]) -> Option<String> {
        let shard = self.shard.upgrade()?;
        match shard
            .execution_context(&conversation_of(context.root), seen)
            .await
        {
            Ok(text) => text,
            Err(error) => {
                // Nothing records the block as seen, so the node reads it
                // before its next request.
                tracing::warn!(
                    error = &error as &dyn std::error::Error,
                    "the execution context cannot be read"
                );
                None
            }
        }
    }
}
