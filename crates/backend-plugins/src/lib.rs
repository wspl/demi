//! The plugin host: the registry of the backend's plugins with the checks of
//! their manifests, and each user's plugins.
//!
//! A user's plugins provide:
//!
//! - The command set every node of the user's conversations starts from.
//! - The instances that serve the user's requests.
//! - The port operations the host answers.
//!
//! The composition root registers the plugins linked into the backend, as it
//! registers the provider families, and a user turns each on or off while the
//! backend runs.
//!
//! A plugin's commands reach a conversation only when its tree opens, though
//! every other contribution follows a switch at once or at the next request. A
//! node's capability index is part of its system prompt, which is rendered
//! once, and a change in the middle of a conversation would change what the
//! model was told it may run.
//!
//! Plugins are trusted with every user's data: a plugin serves every user of
//! the backend, and choosing the plugins is the deployment's decision, made
//! when it is built. The host must not:
//!
//! - See the user's `Shard` (what it needs of it is `PluginShard`).
//! - Reach a Host.
//! - Know an agent's session.
//! - Hold the logic of one plugin.

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
    SwitchError, UserPlugins, VideoJob,
};
