//! The `file-browser` plugin (`plugins.md` § Built-in plugins): its identity and
//! its page package, and nothing else. Its page,
//! `@demicodes/plugin-file-browser`, shows the work panel's File view over
//! the product's file routes (`plugin-pages.md` § Registration).

use std::rc::Rc;

use demi_plugin_interface::{Manifest, NoRequests, Page, Plugin, PluginFactory, PluginId};

/// The plugin's factory.
pub struct FileBrowser {
    manifest: Manifest,
}

impl FileBrowser {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("file-browser").expect("a valid plugin id"),
            "File Browser",
            "Opens the conversation's files in the work panel to read them, with a tree of the working directory.",
        );
        manifest.page = Some(Page::new("@demicodes/plugin-file-browser"));
        Self { manifest }
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
