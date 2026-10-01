//! The plugin host (`plugins.md` § The plugin host): the registry of the
//! backend's plugins with the checks of their manifests, and each user's
//! plugins, whose command set every node of the user's conversations starts
//! from and whose instances serve the user's requests.

mod commands;
mod registry;

pub use registry::{Registry, RegistryError, UserPlugins};
