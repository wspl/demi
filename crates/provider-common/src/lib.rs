//! The provider contract and the building blocks every vendor crate uses
//! (`crates-and-packages.md` § provider-common): a shared [`Provider`] per entry and
//! account and a shard-local [`ProviderRuntime`] per session; a run's events
//! and its typed failure; the HTTP failure record and its standard reading;
//! request bodies built off the shard and the endpoint rule of API-key
//! entries; the vendor-wire decoding in [`wire`], with the OpenAI-shaped
//! streams and their request side in [`openai_request`]; the OAuth pieces of
//! device logins and refreshes in [`oauth`]; the credential pool, its refresh
//! protocol and the account operations in [`credentials`]; an account's
//! quota in [`quota`]; the image role in [`images`]; the models.dev document in [`models_dev`]; a value
//! read from a vendor's service and kept for a while in [`kept`]; and, behind
//! the `testing` feature, scripted runtimes and a scripted vendor for tests.

mod body;
mod contract;
pub mod credentials;
mod endpoint;
mod failure;
mod http_record;
pub mod images;
pub mod kept;
pub mod models_dev;
pub mod oauth;
pub mod openai_request;
pub mod quota;
mod secret;
#[cfg(feature = "testing")]
pub mod testing;
pub mod thinking;
mod vendor;
pub mod wire;

pub use body::{encode_body, json_body};
pub use contract::{
    AccountSwitch, Ahead, CallEvent, CallOutcome, Capabilities, CatalogError, EntriesOf, InferenceItem, InferenceRequest,
    MediaBytes, Medium, PromptCache, Provider, ProviderEvent, ProviderRun, ProviderRuntime, RequestBlock, RequestLimits,
    RequestPurpose, ResultPart, RuntimeEnv, RuntimeError, ToolCall, ToolDefinition, ToolResultKinds, UserPart, VendorCall,
    offers,
};
pub use endpoint::endpoint_url;
pub use failure::{EarlyEnd, ErrorCode, FailureReader, ProviderFailure};
pub use http_record::{HttpFailureRecord, http_failure, read_http_failure, retry_at};
pub use secret::{Secret, SecretError};
pub use vendor::VendorPolicy;
