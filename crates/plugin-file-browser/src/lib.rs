//! The `file-browser` plugin: its identity,
//! its page package, and the `attachment` panel kind its page shows. Its
//! page, `@demicodes/plugin-file-browser`, shows the work panel's File view
//! over the product's file routes (`crates/xtask/src/contracts/registry.rs`), and an
//! attachment's stored copy in a tab of the `attachment` kind, which the
//! page opens for an attachment block's card.

use std::rc::Rc;

use demi_plugin_interface::{
    Manifest, NoRequests, Page, Plugin, PluginError, PluginFactory, PluginId, PluginPort, Reply,
    Request,
};
use futures_util::future::LocalBoxFuture;

/// The panel kind of an attachment's tab, whose tabs the backend keeps.
const ATTACHMENT_KIND: &str = "attachment";

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
        manifest.page = Some(Page::new("@demicodes/plugin-file-browser").panel_kind(ATTACHMENT_KIND));
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
        Rc::new(Instance)
    }
}

/// One user's plugin. A tab of the `attachment` kind the user removed needs
/// nothing of it: the attachment stays in the conversation, and the tab held
/// only where its copy loads from. Any other request is refused, as one to a
/// plugin that declares nothing else.
struct Instance;

impl Plugin for Instance {
    fn call(&self, request: Request, port: PluginPort) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        match request {
            Request::PanelTab { .. } => Box::pin(async { Ok(Reply::Done) }),
            other => NoRequests.call(other, port),
        }
    }
}
