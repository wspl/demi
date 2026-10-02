//! A plugin: the factory every shard thread shares, and the instance it
//! makes for one user's shard.

use std::rc::Rc;

use demi_command_declarations::Node;
use demi_host_interface::{
    CommandSet, GroupBuilder, RegisterError, RpcError, RpcHandler, RpcInvocation, RpcPort,
};
use futures_util::future::LocalBoxFuture;

use crate::{
    Commands, DEMI_ROOT, DEMI_SUMMARY, Manifest, Placement, PluginError, PluginPort, Reply, Request,
};

/// A plugin as the backend registers it.
pub trait PluginFactory: Send + Sync {
    fn manifest(&self) -> &Manifest;

    /// The instance that serves one user's shard, on its thread.
    fn instance(&self) -> Rc<dyn Plugin>;
}

/// A plugin's instance for one user.
pub trait Plugin {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>>;
}

/// A plugin whose requests are its commands, served by a command set of its
/// own whose handlers act through the request's rpc port. The set is laid
/// out as the plugin host places the trees, so a group under `demi` is not a
/// root and may take a name a root may not, such as `file`.
pub struct CommandPlugin {
    commands: CommandSet,
    placement: Placement,
}

impl CommandPlugin {
    /// Serves `trees`, each placed at `placement`.
    pub fn new(placement: Placement, trees: Vec<GroupBuilder>) -> Result<Self, RegisterError> {
        let mut commands = CommandSet::new();
        match placement {
            Placement::Demi => {
                let root = trees.into_iter().fold(
                    GroupBuilder::new(DEMI_ROOT, DEMI_SUMMARY),
                    GroupBuilder::group,
                );
                commands.register(root)?;
            }
            Placement::Root => {
                for tree in trees {
                    commands.register(tree)?;
                }
            }
        }
        Ok(Self {
            commands,
            placement,
        })
    }

    /// Its trees as its manifest declares them.
    pub fn manifest_commands(&self) -> Vec<Commands> {
        let trees = self.commands.declarations();
        let trees: Vec<_> = match self.placement {
            Placement::Demi => trees
                .flat_map(|root| match root {
                    Node::Group(demi) => demi.subcommands.clone(),
                    Node::Leaf(_) => unreachable!("the demi root is a group"),
                })
                .collect(),
            Placement::Root => trees.cloned().collect(),
        };
        trees
            .into_iter()
            .map(|tree| Commands {
                placement: self.placement,
                tree,
            })
            .collect()
    }
}

impl CommandPlugin {
    /// Runs the command `invocation` names, its path from the plugin's own
    /// tree, through the request's rpc port.
    pub async fn command(
        &self,
        mut invocation: RpcInvocation,
        port: &PluginPort,
    ) -> Result<Reply, PluginError> {
        if self.placement == Placement::Demi {
            invocation.path.insert(0, DEMI_ROOT.to_owned());
        }
        let code = self.commands.dispatch(invocation, port.rpc()).await?;
        Ok(Reply::Exit { code })
    }

    /// Checks that `invocation`, its path from the plugin's own tree, names
    /// an `rpc` leaf whose input its arguments fit, for a plugin that
    /// answers its leaves itself with the whole port ([`PortHandled`]).
    pub fn check(&self, invocation: &RpcInvocation) -> Result<(), PluginError> {
        let mut invocation = invocation.clone();
        if self.placement == Placement::Demi {
            invocation.path.insert(0, DEMI_ROOT.to_owned());
        }
        self.commands.check(&invocation)?;
        Ok(())
    }
}

/// The handler of an `rpc` leaf its plugin answers itself, with the
/// request's whole port rather than its rpc port: the plugin checks the
/// call with [`CommandPlugin::check`] and runs it, so this handler is never
/// called.
pub struct PortHandled;

impl RpcHandler for PortHandled {
    fn call(
        &self,
        invocation: RpcInvocation,
        _: RpcPort,
    ) -> LocalBoxFuture<'_, Result<u8, RpcError>> {
        let named = invocation.path.join(" ");
        Box::pin(async move {
            Err(RpcError::Failed(format!(
                "\"{named}\" is answered by its plugin, not dispatched"
            )))
        })
    }
}

/// A plugin whose requests are its commands only.
impl Plugin for CommandPlugin {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::Command { invocation, .. } => self.command(*invocation, &port).await,
                Request::PageState { .. }
                | Request::PageCall { .. }
                | Request::PanelTab { .. }
                | Request::Topic { .. } => Err(PluginError::undeclared("page")),
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
            }
        })
    }
}

/// The instance of a plugin that declares nothing but its identity, such as
/// one that only shows a page (`plugin-pages.md` § Registration): no request
/// reaches it, and one that did would be refused.
pub struct NoRequests;

impl Plugin for NoRequests {
    fn call(
        &self,
        request: Request,
        _: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        let what = match request {
            Request::Command { .. } => "command",
            Request::PageState { .. }
            | Request::PageCall { .. }
            | Request::PanelTab { .. }
            | Request::Topic { .. } => "page",
            Request::Context { .. } => "context source",
        };
        Box::pin(async move { Err(PluginError::undeclared(what)) })
    }
}
