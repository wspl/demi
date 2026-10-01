//! The `browser` plugin (`plugins.md` § Built-in plugins): the `demi browser`
//! group, every leaf bound to an operation of the `demi.browser` command
//! package, which runs it on the Host.

mod browser;

use std::rc::Rc;

use demi_plugin_interface::{CommandPlugin, Manifest, Placement, Plugin, PluginFactory, PluginId};

/// The plugin's factory.
pub struct Browser {
    manifest: Manifest,
}

impl Browser {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(PluginId::try_from("browser").expect("a valid plugin id"));
        manifest.commands = commands().manifest_commands();
        Self { manifest }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Browser {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Its leaves are all native, so no request reaches the instance.
    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(commands())
    }
}

/// The plugin's `demi browser` group.
fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![browser::browser_group()])
        .expect("the browser group is a valid declaration")
}
