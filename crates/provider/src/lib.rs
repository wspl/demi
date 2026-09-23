//! The provider contract and the building blocks every vendor crate uses
//! (`crates-and-packages.md` § provider): a shared [`Provider`] per entry and
//! account and a shard-local [`ProviderRuntime`] per session; a run's events
//! and its typed failure; the HTTP failure record and its standard reading;
//! the vendor-wire decoding in [`wire`]; the shapes of quota and account
//! operations; and, behind the `testing` feature, scripted runtimes and a
//! scripted vendor for tests.

mod contract;
pub mod credentials;
mod failure;
mod http_record;
pub mod models_dev;
pub mod quota;
mod secret;
#[cfg(feature = "testing")]
pub mod testing;
pub mod wire;

pub use contract::{
    Capabilities, CatalogError, InferenceItem, InferenceRequest, Provider, ProviderEvent,
    ProviderRun, ProviderRuntime, RuntimeEnv, RuntimeError, ToolCall, ToolDefinition,
};
pub use failure::{ErrorCode, FailureReader, ProviderFailure};
pub use http_record::{HttpFailureRecord, http_failure, read_http_failure, retry_at};
pub use secret::{Secret, SecretError};
