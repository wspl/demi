//! The `expose` plugin (`expose.md`): the feature on top of the backend's
//! exposes. It owns the `demi expose` commands with their numbers, the
//! one-hour policy, and the page state and methods of the conversation
//! header's expose menu; the records, their lifetime and the public relay
//! are the backend's, which it reaches through its port.

mod commands;
mod numbers;
pub mod page;

use std::rc::Rc;

use demi_plugin_interface::{
    CommandPlugin, Manifest, Plugin, PluginError, PluginFactory, PluginId, PluginPort, Reply,
    Request,
};
use futures_util::future::LocalBoxFuture;

/// How long an expose lives from its creation or its last renewal, in
/// seconds: one hour (`expose.md` § Lifetime).
const LIFETIME: u64 = 60 * 60;

/// The plugin's factory.
pub struct Expose {
    manifest: Manifest,
}

impl Expose {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("expose").expect("a valid plugin id"),
            "Expose",
            "Gives a service on one of your hosts a public URL for an hour, with demi expose.",
        );
        manifest.commands = commands::commands().manifest_commands();
        manifest.page = Some(page::page());
        Self { manifest }
    }
}

impl Default for Expose {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Expose {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance {
            commands: commands::commands(),
        })
    }
}

/// One user's expose plugin.
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
                    self.commands.check(&invocation)?;
                    let code = commands::run(*invocation, &port).await?;
                    Ok(Reply::Exit { code })
                }
                Request::PageState { .. } => Ok(Reply::State {
                    state: page::state(&port).await?,
                }),
                Request::PageCall { method, params, .. } => Ok(Reply::Result {
                    result: page::call(&method, params, &port).await?,
                }),
                Request::PanelTab { .. } => Ok(Reply::Done),
                Request::Topic { .. } => Err(PluginError::undeclared("topic")),
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
            }
        })
    }
}
