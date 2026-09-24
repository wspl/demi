//! The hosted product's server (`backend.md`). The executable is
//! `demi-backend`; `Backend::start` with a `BackendConfig` is the entry point
//! for tests.

mod auth;
mod backend;
mod config;
mod conversation;
mod edge;
mod llm;
mod runner;
mod settings;
mod shard;
mod storage;
mod sync;
mod usage;
mod vault;

pub use auth::email_change::{AccountMail, MailError, VerificationMail};
pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError};
pub use config::{BackendConfig, Config, ConfigError, ConversationTuning, RunnerTuning};
pub use llm::families::{
    AccountBinding, ApiKeyArgs, FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily,
    SubscriptionArgs,
};
pub use shard::ShardPlacement;
pub use vault::logins::LoginTiming;
pub use vault::secret::{InstanceSecret, SecretError};
