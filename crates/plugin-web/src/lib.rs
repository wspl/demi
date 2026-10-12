//! The `web` plugin (`plugins.md` § Built-in plugins, `web.md`): the `demi
//! web` group. For now it holds only its identity and the group's `fetch`
//! leaf, bound to the `demi.fetch` command package, which runs it on the
//! Host; `search`, the plugin's secrets, its sign-in to Exa and its settings
//! page are still to come.

mod web;

use std::rc::Rc;

use demi_plugin_interface::{CommandPlugin, Manifest, Placement, Plugin, PluginFactory, PluginId};

/// The plugin's factory.
pub struct Web {
    manifest: Manifest,
}

impl Web {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("web").expect("a valid plugin id"),
            "Web",
            "Reads web pages from the conversation's Host with demi web fetch.",
        );
        manifest.commands = commands().manifest_commands();
        Self { manifest }
    }
}

impl Default for Web {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Web {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Its only leaf is native, so no request reaches the instance.
    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(commands())
    }
}

/// The plugin's `demi web` group.
fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![web::web_group()])
        .expect("the web group is a valid declaration")
}
