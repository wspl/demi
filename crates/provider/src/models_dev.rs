//! The models.dev client (`models.md` § The models.dev document): the one
//! copy of the document a backend keeps, revalidated with the copy's `ETag`
//! and `Last-Modified`, shared by concurrent readers, and kept after a failed
//! request as a stale copy; and the mapping of its models onto the catalog's
//! shape. The client knows nothing of vendors: its readers pick what they
//! need, such as one vendor's model list.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_core::{Clock, ModelCost, ProviderModel, ProviderModelList, Timestamp};
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared, WeakShared};
use http::header::{ACCEPT, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use http::{HeaderValue, StatusCode};
use indexmap::IndexMap;
use reqwest::Url;
use serde::Deserialize;
use tokio::time::Instant;

use crate::ProviderFailure;

/// How long a copy serves [`ModelsDevClient::current`] after it was
/// downloaded or confirmed.
const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// The one models.dev copy of a backend. Cloning it is cheap, and every
/// clone shares the copy.
#[derive(Clone)]
pub struct ModelsDevClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    url: Url,
    clock: Arc<dyn Clock>,
    /// Shared by every thread that reads the document, for sections that
    /// never await.
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    copy: Option<Arc<DocumentCopy>>,
    /// The request in flight, which every reader joins; it ends when its
    /// last reader stops waiting.
    flight: Option<WeakShared<Flight>>,
}

type Flight = BoxFuture<'static, Result<Arc<DocumentCopy>, String>>;

/// A downloaded document and what revalidates it.
struct DocumentCopy {
    document: Arc<ModelsDevDocument>,
    /// When the content was downloaded; a confirmation does not move it.
    fetched_at: Timestamp,
    /// When the content was downloaded or last confirmed.
    confirmed_at: Instant,
    etag: Option<HeaderValue>,
    last_modified: Option<HeaderValue>,
}

/// The document as one read returned it.
#[derive(Debug, Clone)]
pub struct ModelsDevSnapshot {
    document: Arc<ModelsDevDocument>,
    /// When the content was downloaded.
    pub fetched_at: Timestamp,
    /// Whether this is the copy kept after a failed request.
    pub stale: bool,
    pub warnings: Vec<String>,
}

/// Why the document could not be read: the request failed, or its answer
/// could not be read, and no copy was kept to fall back on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ModelsDevError(String);

impl ModelsDevClient {
    /// The document's published address.
    pub const DEFAULT_URL: &str = "https://models.dev/api.json";

    /// A client of the document at `url` with no copy yet. `http` is the
    /// client of the runtime that reads it, and `clock` dates each download.
    pub fn new(http: reqwest::Client, url: Url, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                url,
                clock,
                state: Mutex::default(),
            }),
        }
    }

    /// The document, asked for again: a catalog read. With a copy the request
    /// is conditional, and a 304 answer confirms the copy without moving its
    /// fetch time.
    pub async fn refreshed(&self) -> Result<ModelsDevSnapshot, ModelsDevError> {
        self.read(false).await
    }

    /// The document for a reader that may use a recent copy, such as the
    /// vendor list: a copy downloaded or confirmed less than a day ago
    /// serves without a request.
    pub async fn current(&self) -> Result<ModelsDevSnapshot, ModelsDevError> {
        self.read(true).await
    }

    async fn read(&self, reuse_recent: bool) -> Result<ModelsDevSnapshot, ModelsDevError> {
        let flight = {
            let mut state = self.inner.lock();
            if reuse_recent
                && let Some(copy) = state.copy.as_ref().filter(|copy| copy.confirmed_at.elapsed() < FRESH_FOR)
            {
                return Ok(copy.snapshot());
            }
            match state.flight.as_ref().and_then(WeakShared::upgrade) {
                Some(flight) => flight,
                None => {
                    let flight: Shared<Flight> = fetch(self.inner.clone(), state.copy.clone()).boxed().shared();
                    state.flight = flight.downgrade();
                    flight
                }
            }
        };
        match flight.await {
            Ok(copy) => Ok(copy.snapshot()),
            Err(failure) => {
                let kept = self.inner.lock().copy.clone();
                match kept {
                    Some(copy) => Ok(copy.stale(&failure)),
                    None => Err(ModelsDevError(failure)),
                }
            }
        }
    }
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A section under the lock only reads or replaces fields, so a
        // panic elsewhere leaves the state whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One request for the document, revalidating `previous`; its outcome
/// becomes the copy.
async fn fetch(inner: Arc<Inner>, previous: Option<Arc<DocumentCopy>>) -> Result<Arc<DocumentCopy>, String> {
    let outcome = request(&inner, previous.as_deref()).await;
    let mut state = inner.lock();
    state.flight = None;
    if let Ok(copy) = &outcome {
        state.copy = Some(copy.clone());
    }
    outcome
}

async fn request(inner: &Inner, previous: Option<&DocumentCopy>) -> Result<Arc<DocumentCopy>, String> {
    let mut request = inner
        .http
        .get(inner.url.clone())
        .header(ACCEPT, HeaderValue::from_static("application/json"));
    if let Some(previous) = previous {
        if let Some(etag) = &previous.etag {
            request = request.header(IF_NONE_MATCH, etag.clone());
        }
        if let Some(modified) = &previous.last_modified {
            request = request.header(IF_MODIFIED_SINCE, modified.clone());
        }
    }
    let unanswered = |error| ProviderFailure::transport("models.dev", error).message;
    let response = request.send().await.map_err(unanswered)?;
    let status = response.status();
    if let (StatusCode::NOT_MODIFIED, Some(previous)) = (status, previous) {
        return Ok(Arc::new(DocumentCopy {
            document: previous.document.clone(),
            fetched_at: previous.fetched_at,
            confirmed_at: Instant::now(),
            etag: previous.etag.clone(),
            last_modified: previous.last_modified.clone(),
        }));
    }
    if !status.is_success() {
        return Err(format!("models.dev catalog request failed with HTTP {}", status.as_u16()));
    }
    let etag = response.headers().get(ETAG).cloned();
    let last_modified = response.headers().get(LAST_MODIFIED).cloned();
    let body = response.bytes().await.map_err(unanswered)?;
    // The document is megabytes of JSON, more than an async thread may spend.
    let document = tokio::task::spawn_blocking(move || ModelsDevDocument::decode(&body))
        .await
        .map_err(|error| format!("models.dev catalog could not be read: {error}"))??;
    Ok(Arc::new(DocumentCopy {
        document: Arc::new(document),
        fetched_at: inner.clock.now(),
        confirmed_at: Instant::now(),
        etag,
        last_modified,
    }))
}

impl DocumentCopy {
    fn snapshot(&self) -> ModelsDevSnapshot {
        ModelsDevSnapshot {
            document: self.document.clone(),
            fetched_at: self.fetched_at,
            stale: false,
            warnings: Vec::new(),
        }
    }

    /// The copy kept after a request that failed with `failure`.
    fn stale(&self, failure: &str) -> ModelsDevSnapshot {
        ModelsDevSnapshot {
            document: self.document.clone(),
            fetched_at: self.fetched_at,
            stale: true,
            warnings: vec![format!("Using stale models.dev catalog: {failure}")],
        }
    }
}

impl ModelsDevSnapshot {
    /// Every vendor, in the document's order.
    pub fn vendors(&self) -> impl Iterator<Item = &ModelsDevVendor> {
        self.document.0.values()
    }

    pub fn vendor(&self, id: &str) -> Option<&ModelsDevVendor> {
        self.document.0.get(id)
    }

    /// The whole model list of vendor `vendor_id` as a catalog, with this
    /// read's time, staleness and warnings; `None` for a vendor the document
    /// does not list.
    pub fn vendor_models(&self, vendor_id: &str) -> Option<ProviderModelList> {
        let vendor = self.vendor(vendor_id)?;
        Some(ProviderModelList {
            models: vendor.models().collect(),
            default_model_id: None,
            warnings: self.warnings.clone(),
            source_fetched_at: self.fetched_at,
            stale: self.stale,
        })
    }
}

/// The document: vendors by id. Only the fields Demi reads are declared, and
/// a vendor or model that does not match them makes the document unreadable.
#[derive(Debug, Deserialize)]
struct ModelsDevDocument(IndexMap<String, ModelsDevVendor>);

impl ModelsDevDocument {
    fn decode(body: &[u8]) -> Result<Self, String> {
        serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_slice(body))
            .map_err(|error| format!("models.dev catalog cannot be read at {}: {}", error.path(), error.inner()))
    }
}

/// One vendor of the document.
#[derive(Debug, Deserialize)]
pub struct ModelsDevVendor {
    pub id: String,
    pub name: String,
    /// The client package the vendor's data is written for, the document's
    /// only protocol tag.
    #[serde(default)]
    pub npm: Option<String>,
    /// The vendor's base URL; absent for vendors whose own clients know it.
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    models: IndexMap<String, ModelsDevModel>,
}

impl ModelsDevVendor {
    /// The vendor's models as catalog models, in the document's order.
    pub fn models(&self) -> impl Iterator<Item = ProviderModel> + '_ {
        self.models.iter().map(|(id, model)| model.catalog(id))
    }
}

/// One model of a vendor.
#[derive(Debug, Deserialize)]
struct ModelsDevModel {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    attachment: Option<bool>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    reasoning_options: Option<Vec<ReasoningOption>>,
    #[serde(default)]
    tool_call: Option<bool>,
    #[serde(default)]
    limit: Option<Limit>,
    #[serde(default)]
    cost: Option<Cost>,
}

#[derive(Debug, Deserialize)]
struct ReasoningOption {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    values: Option<Vec<Option<String>>>,
}

#[derive(Debug, Deserialize)]
struct Limit {
    #[serde(default)]
    context: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Cost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
}

impl ModelsDevModel {
    /// The model as a catalog model (`models.md` § The models.dev
    /// document): `limit.context` and `limit.output` are its token limits,
    /// `attachment` its attachment support, `reasoning` its thinking
    /// support, the values of its `effort` reasoning option its efforts,
    /// `tool_call` its tool support and `cost` its prices. What the document
    /// does not state is unknown.
    fn catalog(&self, id: &str) -> ProviderModel {
        let limit = self.limit.as_ref();
        ProviderModel {
            id: id.to_owned(),
            display_name: self.name.clone().unwrap_or_else(|| id.to_owned()),
            description: self.description.clone(),
            context_window: limit.and_then(|limit| tokens(limit.context)),
            output_limit: limit.and_then(|limit| tokens(limit.output)),
            supports_tools: self.tool_call,
            supports_attachments: self.attachment,
            supports_video: None,
            accepted_extensions: None,
            supports_reasoning: self.reasoning,
            supported_thinking_efforts: self.efforts(),
            default_thinking_effort: None,
            can_disable_thinking: None,
            service_tiers: Vec::new(),
            default_service_tier_id: None,
            cost: self.cost.as_ref().map(|cost| ModelCost {
                input: cost.input,
                output: cost.output,
                cache_read: cost.cache_read,
                cache_write: cost.cache_write,
            }),
        }
    }

    /// The nonempty values of the `effort` reasoning option; unknown without
    /// one.
    fn efforts(&self) -> Option<Vec<String>> {
        let option = self.reasoning_options.as_ref()?.iter().find(|option| option.kind == "effort")?;
        let values = option.values.as_ref()?;
        Some(values.iter().flatten().filter(|value| !value.is_empty()).cloned().collect())
    }
}

/// A token limit the document states as a positive whole number; zero,
/// fractions and other values state none.
fn tokens(value: Option<f64>) -> Option<u32> {
    let value = value?;
    let whole = value.fract() == 0.0 && value >= 1.0 && value <= f64::from(u32::MAX);
    // A whole number in u32's range converts exactly.
    whole.then(|| value as u32)
}
