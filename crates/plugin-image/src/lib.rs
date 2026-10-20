//! The `image` plugin: the
//! `demi image` group, which makes and edits images through the port's
//! Providers service, reads an edit's images with the port's file reads,
//! writes the image's bytes to stdout, and says on stderr what came back
//! and what the result did not honour.
//!
//! No vendor names the model that made an image, so the model is the one
//! thing the result cannot show: ChatGPT's backend accepts any model name,
//! an invented one included, and returns the same image, so a `--model`
//! there is a request, not a guarantee, which the stderr line says.

mod commands;
mod image;

use std::rc::Rc;

use demi_plugin_interface::{
    CommandPlugin, Manifest, Plugin, PluginError, PluginFactory, PluginId, PluginPort, Reply,
    Request,
};

/// The plugin's factory.
pub struct Image {
    manifest: Manifest,
}

impl Image {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("image").expect("a valid plugin id"),
            "Image Generation",
            "Makes and edits images through your ChatGPT or Grok subscription with demi image.",
        );
        manifest.commands = commands::commands().manifest_commands();
        Self { manifest }
    }
}

impl Default for Image {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Image {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance(commands::commands()))
    }
}

/// One user's image plugin; it keeps nothing.
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
                    let code = image::run(*invocation, &port).await?;
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
