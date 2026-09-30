//! The hosted product's server (`backend.md`). The executable is
//! `demi-backend`; `Backend::start` with a `BackendConfig` is the entry point
//! for tests.

mod backend;
mod config;
mod conversation;
mod edge;
mod expose;
#[cfg(feature = "testing")]
mod holds;
mod lifecycle;
mod managed;
mod runner;
mod shard;
mod sync;

pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError};
pub use config::{
    BackendConfig, CloudTuning, Config, ConfigError, ConversationTuning, ExposeTuning, LifecycleTuning, PageTuning,
    RunnerTuning,
};
pub use expose::{ExposeDomain, NotExposeDomain};
#[cfg(feature = "testing")]
pub use managed::client::{MachinesClient, MachinesError};
#[cfg(feature = "testing")]
pub use holds::StepHold;
#[cfg(feature = "testing")]
pub use runner::hold::HelloStep;
pub use runner::native::NativeCatalog;
pub use runner::publication::{PublicationError, publish_native};
pub use shard::ShardPlacement;
#[cfg(feature = "testing")]
pub use sync::SyncStep;
pub use config::secret::{InstanceSecret, SecretError};
