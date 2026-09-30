//! The hosted product's server (`backend.md`). The executable is
//! `demi-backend`; `Backend::start` with a `BackendConfig` is the entry point
//! for tests.

mod backend;
mod config;
mod conversation;
mod edge;
#[cfg(feature = "testing")]
mod holds;
mod lifecycle;
mod shard;
mod sync;

pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError};
pub use config::{
    BackendConfig, Config, ConfigError, ConversationTuning, ExposeTuning, LifecycleTuning, PageTuning,
    RunnerTuning,
};
#[cfg(feature = "testing")]
pub use holds::StepHold;
#[cfg(feature = "testing")]
pub use holds::HelloStep;
pub use shard::ShardPlacement;
#[cfg(feature = "testing")]
pub use sync::SyncStep;
pub use config::secret::{InstanceSecret, SecretError};
