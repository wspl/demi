//! The provider contract and the building blocks every vendor crate uses
//! (`crates-and-packages.md` § provider): a shared [`Provider`] per entry and
//! account and a shard-local [`ProviderRuntime`] per session; a run's events
//! and its typed failure; the HTTP failure record and its standard reading;
//! request bodies built off the shard and the endpoint rule of API-key
//! entries; the vendor-wire decoding in [`wire`], with the OpenAI-shaped
//! streams and their request side in [`openai_request`]; the OAuth pieces of
//! device logins and refreshes in [`oauth`]; the credential pool, its refresh
//! protocol and the account operations in [`credentials`]; an account's
//! quota in [`quota`]; the models.dev document in [`models_dev`]; and, behind
//! the `testing` feature, scripted runtimes and a scripted vendor for tests.

mod body;
mod contract;
pub mod credentials;
mod endpoint;
mod failure;
mod http_record;
pub mod models_dev;
pub mod oauth;
pub mod openai_request;
pub mod quota;
mod secret;
#[cfg(feature = "testing")]
pub mod testing;
pub mod wire;

pub use body::{UnloadedMedia, encode_body, json_body};
pub use contract::{
    Capabilities, CatalogError, InferenceItem, InferenceRequest, Provider, ProviderEvent,
    ProviderRun, ProviderRuntime, RuntimeEnv, RuntimeError, ToolCall, ToolDefinition,
};
pub use endpoint::endpoint_url;
pub use failure::{ErrorCode, FailureReader, ProviderFailure};
pub use http_record::{HttpFailureRecord, http_failure, read_http_failure, retry_at};
pub use secret::{Secret, SecretError};
