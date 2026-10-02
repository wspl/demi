//! The `file-browser` plugin (`plugins.md` § Built-in plugins): its id, name
//! and description and nothing else. Its page,
//! `@demicodes/plugin-file-browser`, shows the work panel's File view over
//! the product's file routes (`plugin-pages.md` § Registration).

use std::rc::Rc;

use demi_plugin_interface::{Manifest, NoRequests, Plugin, PluginFactory, PluginId};

/// The plugin's factory.
pub struct FileBrowser {
    manifest: Manifest,
}

impl FileBrowser {
    pub fn new() -> Self {
        Self {
            manifest: Manifest::new(
                PluginId::try_from("file-browser").expect("a valid plugin id"),
                "File browser",
                "Shows the conversation's files in the work panel's File view.",
            ),
        }
    }
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for FileBrowser {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(NoRequests)
    }
}
