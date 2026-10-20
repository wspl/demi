//! The `changes` plugin (`plugins.md` § Built-in plugins): its identity and its page
//! package, and nothing else. Its page, `@demicodes/plugin-changes`, shows
//! the work panel's Change view over the product's edit tracking and file
//! routes (`plugin-pages.md` § Registration).
//!
//! Edit tracking has this one consumer: the conversation's view of what the
//! agent changed (the file pills under a shell call and the Change view they
//! open). The model never receives it. The working tree view answers a
//! different question, what is uncommitted in the whole directory; the two
//! share the file entry shape and the diff view and nothing else. Every list
//! of changed files and every line count is computed once by the backend when
//! it stores a call's block, so the page derives nothing and reads a copy only
//! to draw a diff. With the plugin off, a pill is no control.

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
