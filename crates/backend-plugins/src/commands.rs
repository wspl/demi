//! The command set every node starts from: the
//! `demi` root with the product's groups and the plugins' groups, and the
//! plugins' roots, each `rpc` leaf served by a handler that forwards the
//! call to its plugin as a command request.
//!
//! A plugin of this repository adds named groups to `demi`; one from outside
//! it adds root commands of its own, such as `jira`, never a group of `demi`.
//! The groups of the agent runtime and the product are taken. Every
//! registered plugin is checked against every other, so any set of them a
//! user turns on is free of conflicts and a conflict is never found while a
//! conversation runs. A group with a `native` leaf whose package or operation
//! the startup catalog does not serve is left out whole, and the log says
//! which, so a plugin needs no check of its own for a deployment without,
//! say, a browser release. A group's permission categories are checked by the
//! backend's dispatch before a call reaches the plugin, which never sees a
//! permission.

use std::rc::Rc;

use demi_host_interface::{
    CommandSet, Declared, GroupBuilder, RpcError, RpcHandler, RpcInvocation, RpcPort,
};
use demi_plugin_interface::{DEMI_ROOT, DEMI_SUMMARY, Placement, Reply, Request};
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::LocalBoxFuture;

use crate::user::Shared;
use crate::{Registry, RegistryError};

/// The `demi` groups the agent runtime and the product own.
pub(crate) const TAKEN_GROUPS: &[&str] = &["agent", "shell", "host", "attachment", "conversation"];

/// The command set of the plugins of `registry` that `include` takes, by
/// their index, whose `rpc` leaves `user`'s instances serve; without them,
/// as the startup check composes it, a call is refused.
pub(crate) fn compose(
    registry: &Registry,
    user: Option<&Rc<Shared>>,
    product: Vec<GroupBuilder>,
    include: impl Fn(usize) -> bool,
) -> Result<CommandSet, RegistryError> {
    let mut set = CommandSet::new();
    let mut demi = !product.is_empty();
    if demi {
        let root = product.into_iter().fold(
            GroupBuilder::new(DEMI_ROOT, DEMI_SUMMARY),
            GroupBuilder::group,
        );
        set.register(root)
            .expect("the product's groups are a valid `demi` root");
    }
    for (index, registered) in registry.plugins.iter().enumerate() {
        if !include(index) {
            continue;
        }
        for commands in &registered.commands {
            let strip = match commands.placement {
                Placement::Demi => 1,
                Placement::Root => 0,
            };
            let handler = Rc::new(Forward {
                user: user.cloned(),
                plugin: index,
                strip,
            });
            let served = Declared::served(commands.tree.clone(), handler);
            let registered_tree = match commands.placement {
                Placement::Root => set.register(served),
                Placement::Demi if demi => set.graft(&[DEMI_ROOT], served),
                Placement::Demi => {
                    demi = true;
                    set.register(GroupBuilder::new(DEMI_ROOT, DEMI_SUMMARY).child(served))
                }
            };
            registered_tree.map_err(|error| RegistryError::Commands {
                plugin: registered.id().clone(),
                error,
            })?;
        }
    }
    Ok(set)
}

/// Forwards an `rpc` call to its plugin, its path from the plugin's own
/// tree, and the plugin's port operations to the call's port and the
/// user's plugin host.
struct Forward {
    user: Option<Rc<Shared>>,
    plugin: usize,
    /// How many names of the path come before the plugin's tree.
    strip: usize,
}

impl RpcHandler for Forward {
    fn call(
        &self,
        mut invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'_, Result<u8, RpcError>> {
        Box::pin(async move {
            let Some(user) = &self.user else {
                return Err(RpcError::Failed(
                    "no plugin instance serves the startup check".into(),
                ));
            };
            invocation.path.drain(..self.strip);
            let conversation =
                ConversationId::try_from(invocation.context.conversation.as_str()).ok();
            let cancel = port.cancellation();
            let request = Request::Command {
                user: user.user.clone(),
                invocation: Box::new(invocation),
            };
            let reply = user
                .request(self.plugin, request, conversation, Some(port), cancel)
                .await?;
            match reply {
                Reply::Exit { code } => Ok(code),
                reply => Err(RpcError::Failed(format!(
                    "the plugin answered a command with {reply:?}"
                ))),
            }
        })
    }
}
