//! The `video` plugin: the
//! `demi video` group, which asks the port's Providers service for one
//! video's job, says on stderr how the job moves, checks the MP4 that
//! comes back, writes it to stdout, and says what came back and what the
//! result did not honour.

mod commands;
mod video;

use std::rc::Rc;

use demi_plugin_interface::{
    CommandPlugin, Manifest, Plugin, PluginError, PluginFactory, PluginId, PluginPort, Reply, Request,
};

/// The plugin's factory.
pub struct Video {
    manifest: Manifest,
}

impl Video {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("video").expect("a valid plugin id"),
            "Video Generation",
            "Makes videos through your Grok subscription, an xAI, Google, MiniMax, BytePlus ModelArk or Volcengine Ark key or a compatible API with demi video.",
        );
        manifest.commands = commands::commands().manifest_commands();
        Self { manifest }
    }
}

impl Default for Video {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Video {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance(commands::commands()))
    }
}

/// One user's video plugin; it keeps nothing.
struct Instance(CommandPlugin);

impl Plugin for Instance {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> futures_util::future::LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::Command { invocation, .. } => {
                    self.0.check(&invocation)?;
                    let code = video::run(*invocation, &port).await?;
                    Ok(Reply::Exit { code })
                }
                Request::PageState { .. } | Request::PageCall { .. } | Request::PanelTab { .. } => {
                    Err(PluginError::undeclared("page"))
                }
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
                Request::ProjectSearch { .. } => Err(PluginError::undeclared("project search")),
                Request::PackageMessage { .. } | Request::PackageLost { .. } => {
                    Err(PluginError::undeclared("package messages"))
                }
            }
        })
    }
}
