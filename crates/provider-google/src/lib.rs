//! The Google Gemini provider (`crates-and-packages.md` § Vendor provider
//! crates): a `google` entry authenticated by an API key, speaking Gemini's
//! native `generateContent` API rather than its OpenAI-compatible one, so a
//! video is a real inline part and thought summaries, thought signatures and
//! thinking token counts arrive as fields of their own. It has a built-in
//! model directory and reads its failure records by the standard reading.

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
    RuntimeEnv, RuntimeError, Secret, read_http_failure,
};
use futures_util::{
    StreamExt,
    future::{BoxFuture, LocalBoxFuture},
};
use reqwest::{Url, header::HeaderValue};

/// The prefix this provider puts on the thought signatures it receives: a
/// transcript outlives a provider choice, and each vendor's signature is its
/// own, so a signature without it is never replayed.
const SIGNATURE_TAG: &str = "google:";

/// The configuration of a `google` entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct GoogleConfig {
    /// The entry's id.
    pub id: String,
    /// The entry's label.
    pub display_name: String,
    pub api_key: Secret,
    /// The API's base with its version, such as
    /// `https://generativelanguage.googleapis.com/v1beta`; `None` for that
    /// default.
    pub base_url: Option<Url>,
}

impl GoogleConfig {
    /// The base URL of an entry that names none (`providers.md` § Endpoints).
    pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
}

/// A `google` entry, shared by every user and request of the entry.
pub struct GoogleProvider {
    shared: Arc<Shared>,
}

/// What the provider and all its runtimes share.
struct Shared {
    id: String,
    display_name: String,
    api_key: HeaderValue,
    base_url: Url,
    clock: Arc<dyn Clock>,
}

impl Shared {
    /// `…/models/<model>:streamGenerateContent?alt=sse` under the base.
    fn stream_url(&self, model_id: &str) -> Url {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            // An http or https URL always has a path to extend.
            .expect("an API base URL has path segments")
            .pop_if_empty()
            .push("models")
            .push(&format!("{model_id}:streamGenerateContent"));
        url.set_query(Some("alt=sse"));
        url
    }
}

impl GoogleProvider {
    pub fn new(config: GoogleConfig, clock: Arc<dyn Clock>) -> Self {
        let base_url = config.base_url.unwrap_or_else(|| {
            Url::parse(GoogleConfig::DEFAULT_BASE_URL).expect("the default base URL parses")
        });
        Self {
            shared: Arc::new(Shared {
                id: config.id,
                display_name: config.display_name,
                api_key: config.api_key.header_value(),
                base_url,
                clock,
            }),
        }
    }
}

impl Provider for GoogleProvider {
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
            message: Some("Uses the Gemini generateContent API".into()),
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
        Ok(Box::new(GoogleRuntime {
            shared: self.shared.clone(),
            http: env.http,
        }))
    }
}

/// A session's runtime: it holds nothing between runs.
struct GoogleRuntime {
    shared: Arc<Shared>,
    http: reqwest::Client,
}

impl ProviderRuntime for GoogleRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        stream::run(self.shared.clone(), self.http.clone(), request).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(GoogleRuntime {
            shared: self.shared.clone(),
            http: self.http.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
