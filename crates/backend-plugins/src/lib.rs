//! The plugin host (`plugins.md` § The plugin host): the registry of the
//! backend's plugins with the checks of their manifests, and each user's
//! plugins, whose command set every node of the user's conversations starts
//! from, whose instances serve the user's requests, and whose port
//! operations the host answers.

mod commands;
mod registry;
mod user;

pub use registry::{Registry, RegistryError};
pub use user::{
    ContextAsk, PageCall, PageCallError, PanelError, PluginToolset, ProductPort, SwitchError,
    UserPlugins,
};
