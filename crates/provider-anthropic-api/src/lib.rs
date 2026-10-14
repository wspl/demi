//! The Anthropic Messages API provider (`crates-and-packages.md` § Vendor
//! provider crates): an `anthropic` entry, which sends its API key when it
//! has one, its request and stream mapping and the standard reading of its
//! failure records.

mod request;
mod stream;

use std::sync::Arc;

use demi_provider_common::{
    Capabilities, InferenceRequest, Provider, ProviderRun, ProviderRuntime,
    RequestLimits, ToolResultKinds, RuntimeEnv, RuntimeError, Secret, VendorPolicy, endpoint_url, read_http_failure,
};
use demi_shared_types::{
    AuthState, Clock, Model, ProviderErrorDiagnostics, ProviderFailureFacts,
    RuntimeState, Timestamp,
};
use futures_util::{
    StreamExt,
    future::{BoxFuture, LocalBoxFuture},
};
use reqwest::{Url, header::HeaderValue};

/// The configuration of an `anthropic` entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// Sent when set: a compatible server of the user's own may take none.
    pub api_key: Option<Secret>,
    /// The Messages API base with its version prefix, such as
    /// `https://api.anthropic.com/v1`; `None` for that default. The provider
    /// appends `/messages` unless the URL already ends with it.
    pub base_url: Option<Url>,
    pub policy: VendorPolicy,
}

impl AnthropicConfig {
    /// The base URL of an entry that names none (`providers.md` § Endpoints).
    pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
}

/// An `anthropic` entry, shared by every user and request of the entry.
pub struct AnthropicProvider {
    shared: Arc<Shared>,
}

/// What the provider and all its runtimes share.
struct Shared {
    api_key: Option<HeaderValue>,
    messages_url: Url,
    policy: VendorPolicy,
    clock: Arc<dyn Clock>,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig, clock: Arc<dyn Clock>) -> Self {
        let base = config.base_url.unwrap_or_else(|| {
            Url::parse(AnthropicConfig::DEFAULT_BASE_URL).expect("the default base URL parses")
        });
        Self {
            shared: Arc::new(Shared {
                api_key: config.api_key.as_ref().map(Secret::header_value),
                messages_url: endpoint_url(&base, "/messages"),
                policy: config.policy,
                clock,
            }),
        }
    }
}

impl Provider for AnthropicProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// Whether the vendor accepts the entry's key, or its lack of one,
    /// shows on the first request.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            AuthState::Authenticated {
                account_label: None,
            }
        })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready {
            message: Some("Uses the Anthropic Messages API".into()),
        }
    }


    fn read_failure(
        &self,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> ProviderFailureFacts {
        read_http_failure(diagnostics, received_at)
    }

    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(AnthropicRuntime {
            shared: self.shared.clone(),
            http: env.http,
        }))
    }
}

/// A session's runtime: it holds nothing between runs.
struct AnthropicRuntime {
    shared: Arc<Shared>,
    http: reqwest::Client,
}

impl ProviderRuntime for AnthropicRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        stream::run(self.shared.clone(), self.http.clone(), request).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(AnthropicRuntime {
            shared: self.shared.clone(),
            http: self.http.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn request_limits(&self, model: &Model) -> RequestLimits {
        // Images and documents inside `tool_result.content` (`providers.md`
        // § Media in tool results).
        RequestLimits {
            tool_results: ToolResultKinds::IMAGES_AND_DOCUMENTS,
            ..RequestLimits::anthropic_messages(model)
        }
    }
}

/// Test support (feature `testing`): the body this provider sends for a
/// request, for the tests of what sends its requests.
#[cfg(feature = "testing")]
pub mod testing {
    use demi_provider_common::{InferenceRequest, VendorPolicy};

    /// The Messages API body of `request` for a vendor of `policy`.
    pub fn body(request: &InferenceRequest, policy: VendorPolicy) -> Vec<u8> {
        crate::request::encode(request, policy)
    }
}
