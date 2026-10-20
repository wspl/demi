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
//! A plugin lives while the user has it on (`decisions/0008-plugin-architecture.md`):
//!
//! ```text
//!              on                          off
//! instance     made at its first request   dropped with its tasks
//! commands     in every open tree          gone: a call fails as any
//!                                          unknown command does
//! Hosts        opened again on demand      its packages' services stop
//! panel tabs   opened again on demand      closed
//! data         kept either way
//! ```
//!
//! An open tree's commands follow a switch at once, but its system prompt,
//! which is rendered once, keeps listing the commands it opened with until it
//! opens again, which a reload offers.
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
    SwitchError, Switched, UserPlugins, VideoJob,
};
