//! The `audio` plugin: the
//! `demi audio` group, which asks the port's Providers service for one
//! file of speech, music or a sound effect, wraps a vendor's raw PCM as
//! WAV, checks the file it reads, writes it to stdout, and says what came
//! back and what the call could not send; and lists a speech model's
//! voices and the audio models.
//!
//! Raw PCM is wrapped as WAV only with the rate, channels and sample size
//! its vendor states, each read and checked where the answer is decoded; one
//! it states wrongly or not at all fails the call, never guessed. Making a
//! voice, a clone or a designed one, is not a Demi command: each vendor asks
//! for the speaker's consent its own way, on its own site.

mod audio;
mod commands;

use std::rc::Rc;

use demi_plugin_interface::{
    CommandPlugin, Manifest, Plugin, PluginError, PluginFactory, PluginId, PluginPort, Reply, Request,
};

/// The plugin's factory.
pub struct Audio {
    manifest: Manifest,
}

impl Audio {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("audio").expect("a valid plugin id"),
            "Audio Generation",
            "Makes speech, music and sound effects through an OpenAI, Google, ElevenLabs, MiniMax, BytePlus Speech or Doubao Speech key or a compatible API with demi audio.",
        );
        manifest.commands = commands::commands().manifest_commands();
        Self { manifest }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Audio {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance(commands::commands()))
    }
}

/// One user's audio plugin; it keeps nothing.
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
                    let code = audio::run(*invocation, &port).await?;
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
