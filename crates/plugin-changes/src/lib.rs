//! The `changes` plugin (`plugins.md` § Built-in plugins): its id, name and
//! description and nothing else. Its page, `@demicodes/plugin-changes`, shows
//! the work panel's Change view over the product's edit tracking and file
//! routes (`plugin-pages.md` § Registration).

use std::rc::Rc;

use demi_plugin_interface::{Manifest, NoRequests, Plugin, PluginFactory, PluginId};

/// The plugin's factory.
pub struct Changes {
    manifest: Manifest,
}

impl Changes {
    pub fn new() -> Self {
        Self {
            manifest: Manifest::new(
                PluginId::try_from("changes").expect("a valid plugin id"),
                "Changes",
                "Shows what the conversation's commands changed, in the work panel's Change view.",
            ),
        }
    }
}

impl Default for Changes {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Changes {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(NoRequests)
    }
}
