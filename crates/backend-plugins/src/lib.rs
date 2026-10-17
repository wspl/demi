//! The plugin host (`plugins.md` § The plugin host): the registry of the
//! backend's plugins with the checks of their manifests, and each user's
//! plugins, whose command set every node of the user's conversations starts
//! from, whose instances serve the user's requests, and whose port
//! operations the host answers.

mod commands;
mod oauth;
mod registry;
mod secrets;
mod user;
mod web;

pub use oauth::{Callback, CallbackError};
pub use web::client as web_client;
pub use registry::{Registry, RegistryError};
#[cfg(feature = "testing")]
pub mod testing;
pub use user::{
    ReadOn,
    ContextAsk, Lost, PageCall, PageCallError, PanelError, PluginToolset, ProductPort,
    SwitchError, UserPlugins,
};
