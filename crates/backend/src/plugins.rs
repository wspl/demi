//! The built-in plugins, one per plugin crate (`plugins.md` § Built-in
//! plugins), in their order of registration: the list the composition root
//! registers. A plugin leaves a deployment by leaving this list.

use demi_plugin_browser::Browser;
use demi_plugin_changes::Changes;
use demi_plugin_expose::Expose;
use demi_plugin_file::File;
use demi_plugin_file_browser::FileBrowser;
use demi_plugin_interface::PluginFactory;
use demi_plugin_skills::Skills;
use demi_plugin_todo::Todo;

/// The plugins built into the backend.
pub fn builtin() -> Vec<Box<dyn PluginFactory>> {
    vec![
        Box::new(File::new()),
        Box::new(Todo::new()),
        Box::new(Browser::new()),
        Box::new(Expose::new()),
        Box::new(Skills::new()),
        Box::new(Changes::new()),
        Box::new(FileBrowser::new()),
    ]
}
