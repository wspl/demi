//! The Anthropic Messages API provider (`crates-and-packages.md` § Vendor
//! provider crates): an `anthropic` entry authenticated by an API key, its
//! request and stream mapping, its built-in model directory and the standard
//! reading of its failure records.

mod models;
mod request;
mod stream;

use std::sync::Arc;

use demi_core::{
    AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
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

/// The configuration of an `anthropic` entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// The entry's id.
    pub id: String,
    /// The entry's label.
    pub display_name: String,
    pub api_key: Secret,
    /// The Messages API base with its version prefix, such as
    /// `https://api.anthropic.com/v1`; `None` for that default. The provider
    /// appends `/messages` unless the URL already ends with it.
    pub base_url: Option<Url>,
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
    id: String,
    display_name: String,
    api_key: HeaderValue,
    messages_url: Url,
    clock: Arc<dyn Clock>,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig, clock: Arc<dyn Clock>) -> Self {
        let base = config.base_url.unwrap_or_else(|| {
            Url::parse(AnthropicConfig::DEFAULT_BASE_URL).expect("the default base URL parses")
        });
        Self {
            shared: Arc::new(Shared {
                id: config.id,
                display_name: config.display_name,
                api_key: config.api_key.header_value(),
                messages_url: endpoint_url(&base, "/messages"),
                clock,
            }),
        }
    }
}

impl Provider for AnthropicProvider {
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
        RuntimeState::Ready {
            message: Some("Uses the Anthropic Messages API".into()),
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
}
