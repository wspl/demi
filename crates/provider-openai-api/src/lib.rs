//! The OpenAI API provider (`crates-and-packages.md` § Vendor provider
//! crates): an `openai` entry authenticated by an API key, speaking the
//! Responses API or, for OpenAI-compatible endpoints, Chat Completions, with
//! the typed vendor policy its entry names, its built-in model directory and
//! the standard reading of its failure records.

mod models;
mod request;
mod run;

use std::sync::Arc;

use demi_core::{
    AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp, WireApi,
};
use demi_provider::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderRun, ProviderRuntime,
    RuntimeEnv, RuntimeError, Secret, endpoint_url, read_http_failure,
};
use futures_util::{
    StreamExt,
    future::{BoxFuture, LocalBoxFuture},
};
use reqwest::{Url, header::HeaderValue};

/// The prefix this provider puts on the reasoning items it receives, so that
/// on replay it sends back only its vendor's.
const SIGNATURE_TAG: &str = "openai:";

/// The configuration of an `openai` entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// The entry's id.
    pub id: String,
    /// The entry's label.
    pub display_name: String,
    pub api_key: Secret,
    /// The API's base, such as `https://api.openai.com/v1`; `None` for that
    /// default. The provider appends the request's path unless the URL
    /// already ends with it.
    pub base_url: Option<Url>,
    pub wire: WireApi,
    pub policy: VendorPolicy,
}

impl OpenAiConfig {
    /// The base URL of an entry that names none (`providers.md` § Endpoints).
    pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
}

/// A vendor's request requirements, which the backend's vendor policy
/// applies to every model of the vendor (`providers.md` § Vendors from
/// models.dev).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VendorPolicy {
    /// Chat Completions: earlier thinking goes back as `reasoning_content`,
    /// as DeepSeek's thinking mode requires on tool-call continuations.
    /// OpenAI refuses the field.
    pub pass_back_reasoning_content: bool,
    /// Responses: replayed assistant messages carry `status: completed`,
    /// which gateways that validate the full item schema require and relay
    /// gateways refuse.
    pub replay_assistant_status: bool,
}

/// An `openai` entry, shared by every user and request of the entry.
pub struct OpenAiProvider {
    shared: Arc<Shared>,
}

/// What the provider and all its runtimes share.
struct Shared {
    id: String,
    display_name: String,
    authorization: HeaderValue,
    url: Url,
    wire: WireApi,
    policy: VendorPolicy,
    clock: Arc<dyn Clock>,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig, clock: Arc<dyn Clock>) -> Self {
        let base = config.base_url.unwrap_or_else(|| {
            Url::parse(OpenAiConfig::DEFAULT_BASE_URL).expect("the default base URL parses")
        });
        let path = match config.wire {
            WireApi::Responses => "/responses",
            WireApi::ChatCompletions => "/chat/completions",
        };
        Self {
            shared: Arc::new(Shared {
                id: config.id,
                display_name: config.display_name,
                authorization: config.api_key.bearer(),
                url: endpoint_url(&base, path),
                wire: config.wire,
                policy: config.policy,
                clock,
            }),
        }
    }
}

impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        &self.shared.id
    }

    fn display_name(&self) -> &str {
        &self.shared.display_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// An entry always holds a key; whether the vendor accepts it shows on
    /// the first request.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            AuthState::Authenticated {
                account_label: None,
            }
        })
    }

    fn runtime_state(&self) -> RuntimeState {
        let message = match self.shared.wire {
            WireApi::Responses => "Uses the OpenAI Responses API",
            WireApi::ChatCompletions => "Uses the OpenAI Chat Completions API",
        };
        RuntimeState::Ready {
            message: Some(message.into()),
        }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(async { Ok(models::directory()) })
    }

    fn read_failure(
        &self,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> ProviderFailureFacts {
        read_http_failure(diagnostics, received_at)
    }

    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(OpenAiRuntime {
            shared: self.shared.clone(),
            http: env.http,
        }))
    }
}

/// A session's runtime: it holds nothing between runs.
struct OpenAiRuntime {
    shared: Arc<Shared>,
    http: reqwest::Client,
}

impl ProviderRuntime for OpenAiRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        run::run(self.shared.clone(), self.http.clone(), request).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(OpenAiRuntime {
            shared: self.shared.clone(),
            http: self.http.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
