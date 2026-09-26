//! The hosted product's server (`backend.md`). The executable is
//! `demi-backend`; `Backend::start` with a `BackendConfig` is the entry point
//! for tests.

mod auth;
mod backend;
mod config;
mod conversation;
mod edge;
mod expose;
mod lifecycle;
mod llm;
mod managed;
mod runner;
mod settings;
mod shard;
mod storage;
mod sync;
mod usage;
mod vault;

pub use auth::email_change::{AccountMail, MailError, VerificationMail};
pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError};
pub use config::{
    BackendConfig, CloudTuning, Config, ConfigError, ConversationTuning, ExposeTuning, LifecycleTuning, RunnerTuning,
};
pub use expose::{ExposeDomain, NotExposeDomain};
pub use llm::families::{
    AccountBinding, ApiKeyArgs, FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily,
    SubscriptionArgs,
};
pub use runner::native::NativeCatalog;
pub use runner::publication::{PublicationError, publish_native};
pub use shard::ShardPlacement;
#[cfg(feature = "testing")]
pub use storage::conversations::CommitHold;
pub use storage::objects::S3ConfigError;
pub use vault::logins::LoginTiming;
pub use vault::secret::{InstanceSecret, SecretError};
