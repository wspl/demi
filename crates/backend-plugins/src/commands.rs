//! The command set every node starts from (`plugins.md` § Commands): the
//! `demi` root with the product's groups and the plugins' groups, and the
//! plugins' roots, each `rpc` leaf served by a handler that forwards the
//! call to its plugin as a command request.

use std::rc::Rc;

use demi_host_interface::{
    CommandSet, Declared, GroupBuilder, PortError, RpcError, RpcHandler, RpcInvocation, RpcPort,
};
use demi_plugin_interface::{
    DEMI_ROOT, DEMI_SUMMARY, Placement, Plugin, PluginPort, PluginTransport, PortAnswer,
    PortMessage, Reply, Request,
};
use futures_util::future::LocalBoxFuture;

use crate::{Registry, RegistryError};

/// The `demi` groups the agent runtime and the product own.
pub(crate) const TAKEN_GROUPS: &[&str] = &["agent", "shell", "host"];

/// The command set of `registry`'s plugins for `user`, whose instances
/// serve their `rpc` leaves; with no instance, as the startup check
/// composes it, a call is refused.
pub(crate) fn compose(
    registry: &Registry,
    instances: &[Option<Rc<dyn Plugin>>],
    user: &str,
    product: Vec<GroupBuilder>,
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
    for ((plugin, trees), instance) in registry.plugins().zip(instances) {
        for commands in trees {
            let strip = match commands.placement {
                Placement::Demi => 1,
                Placement::Root => 0,
            };
            let handler = Rc::new(Forward {
                plugin: instance.clone(),
                user: user.to_owned(),
                strip,
            });
            let served = Declared::served(commands.tree.clone(), handler);
            let registered = match commands.placement {
                Placement::Root => set.register(served),
                Placement::Demi if demi => set.graft(&[DEMI_ROOT], served),
                Placement::Demi => {
                    demi = true;
                    set.register(GroupBuilder::new(DEMI_ROOT, DEMI_SUMMARY).child(served))
                }
            };
            registered.map_err(|error| RegistryError::Commands {
                plugin: plugin.clone(),
                error,
            })?;
        }
    }
    Ok(set)
}

/// Forwards an `rpc` call to its plugin, its path from the plugin's own
/// tree, and the plugin's port operations to the call's port.
struct Forward {
    plugin: Option<Rc<dyn Plugin>>,
    user: String,
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
            let Some(plugin) = &self.plugin else {
                return Err(RpcError::Failed(
                    "no plugin instance serves the startup check".into(),
                ));
            };
            invocation.path.drain(..self.strip);
            let cancel = port.cancellation();
            let port = PluginPort::new(Rc::new(CallPort(port)), cancel);
            let request = Request::Command {
                user: self.user.clone(),
                invocation,
            };
            match plugin.call(request, port).await {
                Ok(Reply::Exit { code }) => Ok(code),
                Err(error) => Err(error.into()),
            }
        })
    }
}

/// A plugin's port over the rpc call it serves.
struct CallPort(RpcPort);

impl PluginTransport for CallPort {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>> {
        Box::pin(async move {
            let PortMessage::Rpc { request } = message;
            let response = self.0.forward(request).await?;
            Ok(PortAnswer::Rpc { response })
        })
    }
}
