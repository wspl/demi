//! The plugin host: the registry of a program's plugins with the checks of
//! their manifests, and each user's plugins. Demi's backend runs one; any
//! program that runs Demi's plugins can.
//!
//! A user's plugins provide:
//!
//! - The command set every node of the user's conversations starts from.
//! - The instances that serve the user's requests.
//! - The port operations the host answers.
//!
//! The program registers the plugins linked into it, and a user turns each on
//! or off while it runs. What the host keeps, and the changes it tells the
//! user's pages of, go through the traits of `store` ([`store::PluginStore`],
//! [`store::PluginChanges`]), which the program implements; the rest of what
//! a plugin's port asks, such as a Host's files or a generated image, through
//! [`ProductPort`].
//!
//! A plugin lives while the user has it on:
//!
//! ```text
//!              on                          off
//! instance     made at its first request   dropped with its tasks
//! commands     in every open tree          gone: a call fails as any
//!                                          unknown command does
//! Hosts        opened again on demand      its packages' residents stop
//! panel tabs   opened again on demand      closed
//! data         kept either way
//! ```
//!
//! An open tree's commands follow a switch at once, but its system prompt,
//! which is rendered once, keeps listing the commands it opened with until it
//! opens again, which a reload offers.
//!
//! Plugins are trusted with every user's data: a plugin serves every user of
//! the program, and choosing the plugins is the deployment's decision, made
//! when it is built. The host must not:
//!
//! - See the program's state but through its store and its `ProductPort`.
//! - Reach a Host.
//! - Know an agent's session.
//! - Hold the logic of one plugin.

mod commands;
mod features;
mod oauth;
mod registry;
mod secrets;
pub mod store;
mod user;
mod web;

pub use oauth::{Callback, CallbackError};
pub use web::client as web_client;
pub use features::FeatureError;
pub use registry::{Registry, RegistryError};
pub use user::{
    ReadOn,
    ContextAsk, Lost, PageCall, PageCallError, PanelError, PluginToolset, ProductPort,
    SwitchError, Switched, UserPlugins, VideoJob,
};
