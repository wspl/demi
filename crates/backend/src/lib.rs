//! The hosted product's server (`backend.md`). The executable is
//! `demi-backend`; `Backend::start` with a `BackendConfig` is the entry point
//! for tests.

mod backend;
mod config;
pub mod families;

pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError};
pub use config::secret::{InstanceSecret, SecretError};
pub use config::{BackendConfig, Config, ConfigError};
