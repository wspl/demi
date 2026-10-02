//! The `todo` plugin (`plugins.md` § Built-in plugins): the `demi todo`
//! group, whose `rpc` handlers keep a task list in the invoking node's
//! command storage (`command-state-history.md`).

mod todo;

use std::rc::Rc;

use demi_plugin_interface::{CommandPlugin, Manifest, Placement, Plugin, PluginFactory, PluginId};

/// The plugin's factory.
pub struct Todo {
    manifest: Manifest,
}

impl Todo {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("todo").expect("a valid plugin id"),
            "Todo list",
            "A task list the agent keeps for each conversation, with `demi todo`.",
        );
        manifest.commands = commands().manifest_commands();
        Self { manifest }
    }
}

impl Default for Todo {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Todo {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(commands())
    }
}

/// The plugin's `demi todo` group.
fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![todo::todo_group()])
        .expect("the todo group is a valid declaration")
}
