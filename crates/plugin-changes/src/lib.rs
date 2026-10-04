//! The `changes` plugin (`plugins.md` § Built-in plugins): its identity and its page
//! package, and nothing else. Its page, `@demicodes/plugin-changes`, shows
//! the work panel's Change view over the product's edit tracking and file
//! routes (`plugin-pages.md` § Registration).

use std::rc::Rc;

use demi_plugin_interface::{Manifest, NoRequests, Page, Plugin, PluginFactory, PluginId};

/// The plugin's factory.
pub struct Changes {
    manifest: Manifest,
}

impl Changes {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("changes").expect("a valid plugin id"),
            "Changes",
            "Shows the uncommitted changes in the conversation's working directory, and what each of the agent's commands changed, as diffs in the work panel.",
        );
        manifest.page = Some(Page::new("@demicodes/plugin-changes"));
        Self { manifest }
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
