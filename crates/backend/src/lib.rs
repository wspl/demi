//! The hosted product's server (`backend.md`). The executable is
//! `demi-backend`; `Backend::start` with a `BackendConfig` is the entry point
//! for tests.

mod auth;
mod backend;
mod config;
mod edge;
mod settings;
mod shard;
mod storage;
mod sync;
mod vault;

pub use auth::email_change::{AccountMail, MailError, VerificationMail};
pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError};
pub use config::{BackendConfig, Config, ConfigError};
pub use shard::ShardPlacement;
pub use vault::secret::{InstanceSecret, SecretError};
