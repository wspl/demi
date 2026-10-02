//! The `browser` plugin (`plugins.md` § Built-in plugins): the `demi
//! browser` group, every leaf bound to an operation of the `demi.browser`
//! command package, which runs it on the Host; the `browser` user stream of
//! the live view; and the tab methods of the work panel's `browser` kind.

mod browser;
pub mod page;

use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::{PACKAGE, live};
use demi_plugin_interface::{
    CommandPlugin, Manifest, Placement, Plugin, PluginError, PluginFactory, PluginId, PluginPort,
    Reply, Request, Stream,
};
use futures_util::future::LocalBoxFuture;

/// The name of the live view's user stream.
pub const STREAM: &str = "browser";

/// The plugin's factory.
pub struct Browser {
    manifest: Manifest,
}

impl Browser {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("browser").expect("a valid plugin id"),
            "Conversation browser",
            "A browser on the conversation's Host that the agent drives with `demi browser` and the user watches in the work panel.",
        );
        manifest.commands = commands().manifest_commands();
        manifest.streams = vec![Stream {
            name: STREAM.into(),
            operation: NativeOperation {
                package: PACKAGE.into(),
                operation: live::OPERATION.into(),
            },
        }];
        manifest.page = Some(page::page());
        Self { manifest }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Browser {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance {
            commands: commands(),
        })
    }
}

/// The plugin's `demi browser` group.
fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![browser::browser_group()])
        .expect("the browser group is a valid declaration")
}

/// One user's browser plugin: its commands are all native, so only the tab
/// methods reach it.
struct Instance {
    commands: CommandPlugin,
}

impl Plugin for Instance {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::Command { invocation, .. } => {
                    self.commands.command(*invocation, &port).await
                }
                Request::PageCall { method, params, .. } => {
                    let result = page::call(&method, params, &port).await?;
                    Ok(Reply::Result { result })
                }
                Request::PageState { .. } => Err(PluginError::undeclared("page state")),
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
            }
        })
    }
}
