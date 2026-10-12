//! The models.dev client (`models.md` § The models.dev document): the one
//! copy of the document a backend keeps, revalidated with the copy's `ETag`
//! and `Last-Modified`, shared by concurrent readers, and kept after a failed
//! request as a stale copy; and the mapping of its models onto the catalog's
//! shape. The client knows nothing of vendors: its readers pick what they
//! need, such as one vendor's model list.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_shared_types::{Clock, ModelCost, ProviderModel, ProviderModelList, Timestamp, modality_extensions};
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
        self.read(Reuse::Never).await
    }

    /// The document for a reader that may use a recent copy, such as the
    /// vendor list: a copy downloaded or confirmed less than a day ago
    /// serves without a request.
    pub async fn current(&self) -> Result<ModelsDevSnapshot, ModelsDevError> {
        self.read(Reuse::Recent).await
    }

    /// The document for a reader that asks as the user types, such as the
    /// model search: any copy serves, and only a backend without one asks
    /// models.dev.
    pub async fn kept(&self) -> Result<ModelsDevSnapshot, ModelsDevError> {
        self.read(Reuse::Any).await
    }

    async fn read(&self, reuse: Reuse) -> Result<ModelsDevSnapshot, ModelsDevError> {
        let flight = {
            let mut state = self.inner.lock();
            let reused = state.copy.as_ref().filter(|copy| match reuse {
                Reuse::Never => false,
                Reuse::Recent => copy.confirmed_at.elapsed() < FRESH_FOR,
                Reuse::Any => true,
            });
            if let Some(copy) = reused {
                return Ok(copy.snapshot());
            }
            match state.flight.as_ref().and_then(WeakShared::upgrade) {
                Some(flight) => flight,
                None => {
                    let flight: Shared<Flight> = fetch(self.inner.clone(), state.copy.clone())
                        .boxed()
                        .shared();
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

/// Which kept copy a read takes without asking models.dev.
#[derive(Clone, Copy)]
enum Reuse {
    Never,
    /// One downloaded or confirmed less than [`FRESH_FOR`] ago.
    Recent,
    Any,
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
async fn fetch(
    inner: Arc<Inner>,
    previous: Option<Arc<DocumentCopy>>,
) -> Result<Arc<DocumentCopy>, String> {
    let outcome = request(&inner, previous.as_deref()).await;
    let mut state = inner.lock();
    state.flight = None;
    if let Ok(copy) = &outcome {
        state.copy = Some(copy.clone());
    }
    outcome
}

async fn request(
    inner: &Inner,
    previous: Option<&DocumentCopy>,
) -> Result<Arc<DocumentCopy>, String> {
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
        return Err(format!(
            "models.dev catalog request failed with HTTP {}",
            status.as_u16()
        ));
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

    /// The models an agent can work with, whose output includes text and
    /// that call tools, and whose name or id holds each word of `query`, in
    /// any case and order, newest release first, at most `limit` (`models.md`
    /// § Adding a model from models.dev). A model several vendors list is
    /// one match: the listing of `vendor` when it lists the model, else
    /// the publisher's ([`Listing::publisher`]). Two listings are the same
    /// model when they name the same model without a vendor's prefix: the
    /// part after the maker in `canonical_model_id`, where the document
    /// gives one, else the last part of the listing's own id, both without
    /// regard to case. A query without a word matches nothing.
    pub fn search(&self, query: &str, vendor: Option<&str>, limit: usize) -> Vec<ModelsDevMatch> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut models: IndexMap<String, Vec<Listing<'_>>> = IndexMap::new();
        for (vendor_id, listed) in &self.document.0 {
            for (id, model) in &listed.models {
                let listing = Listing {
                    vendor: vendor_id,
                    id,
                    model,
                };
                models.entry(listing.identity()).or_default().push(listing);
            }
        }
        let mut found: Vec<&Listing<'_>> = models
            .values()
            .filter_map(|listings| {
                vendor
                    .and_then(|vendor| listings.iter().find(|listing| listing.vendor == vendor))
                    .or_else(|| Listing::publisher(listings))
            })
            .filter(|listing| listing.model.agentic() && listing.matches(&words))
            .collect();
        // Dates are `YYYY-MM-DD`, which order as text. A stable sort keeps
        // the document's order among equal dates, and `None` orders before
        // any date, so a model without one comes last.
        found.sort_by(|left, right| {
            right
                .model
                .release_date
                .as_deref()
                .cmp(&left.model.release_date.as_deref())
        });
        found.into_iter().take(limit).map(Listing::found).collect()
    }

    /// The model `id` as vendor `vendor_id` lists it.
    pub fn listing(&self, vendor_id: &str, id: &str) -> Option<ModelsDevMatch> {
        let (vendor, listed) = self.document.0.get_key_value(vendor_id)?;
        let (id, model) = listed.models.get_key_value(id)?;
        Some(Listing { vendor, id, model }.found())
    }

    /// The whole model list of vendor `vendor_id` as a catalog, with this
    /// read's time, staleness and warnings; `None` for a vendor the document
    /// does not list.
    pub fn vendor_models(&self, vendor_id: &str) -> Option<ProviderModelList> {
        self.listed(vendor_id, |_| true)
    }

    /// The models of vendor `vendor_id` an agent can work with, as the
    /// catalog of an entry added from the vendor list (`models.md`
    /// § Catalog sources): the search's models, so a vendor's speech, image
    /// and video models stay out; `None` for a vendor the document does not
    /// list.
    pub fn agent_models(&self, vendor_id: &str) -> Option<ProviderModelList> {
        self.listed(vendor_id, ModelsDevModel::agentic)
    }

    /// The models of vendor `vendor_id` that `kept` keeps, as a catalog.
    fn listed(
        &self,
        vendor_id: &str,
        kept: impl Fn(&ModelsDevModel) -> bool,
    ) -> Option<ProviderModelList> {
        let vendor = self.vendor(vendor_id)?;
        Some(ProviderModelList {
            models: vendor
                .models
                .iter()
                .filter(|(_, model)| kept(model))
                .map(|(id, model)| model.catalog(id))
                .collect(),
            default_model_id: None,
            warnings: self.warnings.clone(),
            source_fetched_at: self.fetched_at,
            stale: self.stale,
        })
    }
}

/// A model the search found, as the vendor it is filled from lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelsDevMatch {
    /// The vendor's id in the document, such as `google`.
    pub vendor: String,
    /// The model's id as `vendor` names it.
    pub id: String,
    pub model: ProviderModel,
    /// The release date as the document states it, such as `2026-06-17`.
    pub release_date: Option<String>,
}

/// One vendor's listing of a model.
struct Listing<'a> {
    vendor: &'a String,
    id: &'a String,
    model: &'a ModelsDevModel,
}

impl Listing<'_> {
    fn found(&self) -> ModelsDevMatch {
        ModelsDevMatch {
            vendor: self.vendor.clone(),
            id: self.id.clone(),
            model: self.model.catalog(self.id),
            release_date: self.model.release_date.clone(),
        }
    }

    /// The vendor that makes the model, as the listing names it: the first
    /// part of `canonical_model_id`, else of an id such as an aggregator's
    /// `google/gemini-3.8-flash`; none for a bare id without a canonical
    /// one.
    fn maker(&self) -> Option<&str> {
        match &self.model.canonical_model_id {
            Some(canonical) => canonical.split_once('/').map(|(maker, _)| maker),
            None => self.id.split_once('/').map(|(maker, _)| maker),
        }
    }

    /// The model without a vendor's prefix, which every listing of the same
    /// model shares.
    fn identity(&self) -> String {
        let named = match &self.model.canonical_model_id {
            Some(canonical) => canonical.split_once('/').map_or(canonical.as_str(), |(_, model)| model),
            None => self.id.rsplit_once('/').map_or(self.id.as_str(), |(_, model)| model),
        };
        named.to_lowercase()
    }

    /// The listing of the vendor that publishes the model: the one whose
    /// vendor is the maker its listings name, such as Google for Gemini;
    /// when the maker lists it nowhere, the first that names it by a bare
    /// id of its own; else the first.
    fn publisher<'l, 'a>(listings: &'l [Listing<'a>]) -> Option<&'l Listing<'a>> {
        let maker = listings.iter().find_map(Listing::maker);
        maker
            .and_then(|maker| listings.iter().find(|listing| listing.vendor == maker))
            .or_else(|| listings.iter().find(|listing| listing.maker().is_none()))
            .or_else(|| listings.first())
    }

    /// Whether the listing's name or id holds each of `words`, which are
    /// lowercase.
    fn matches(&self, words: &[String]) -> bool {
        let name = self.model.name.as_deref().unwrap_or_default().to_lowercase();
        let id = self.id.to_lowercase();
        words
            .iter()
            .all(|word| name.contains(word.as_str()) || id.contains(word.as_str()))
    }
}

/// The document: vendors by id. Only the fields Demi reads are declared, and
/// a vendor or model that does not match them makes the document unreadable.
#[derive(Debug, Deserialize)]
struct ModelsDevDocument(IndexMap<String, ModelsDevVendor>);

impl ModelsDevDocument {
    fn decode(body: &[u8]) -> Result<Self, String> {
        serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_slice(body)).map_err(
            |error| {
                format!(
                    "models.dev catalog cannot be read at {}: {}",
                    error.path(),
                    error.inner()
                )
            },
        )
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

/// One model of a vendor.
#[derive(Debug, Deserialize)]
struct ModelsDevModel {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    modalities: Option<Modalities>,
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
    /// When the model came out, such as `2026-06-17`.
    #[serde(default)]
    release_date: Option<String>,
    /// The model as its maker names it, such as `google/gemini-2.5-flash`,
    /// the same in every vendor's listing of it.
    #[serde(default)]
    canonical_model_id: Option<String>,
}

/// What a model takes in and gives out, kind by kind.
#[derive(Debug, Deserialize)]
struct Modalities {
    #[serde(default)]
    input: Option<Vec<String>>,
    #[serde(default)]
    output: Option<Vec<String>>,
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
    /// Whether an agent can work with the model: the document states that
    /// its output includes text and that it calls tools. A model that writes
    /// only speech or images, or calls no tool, is not one.
    fn agentic(&self) -> bool {
        let writes_text = self
            .modalities
            .as_ref()
            .and_then(|modalities| modalities.output.as_ref())
            .is_some_and(|output| output.iter().any(|kind| kind == "text"));
        writes_text && self.tool_call == Some(true)
    }

    /// The model as a catalog model (`models.md` § The models.dev
    /// document): `limit.context` and `limit.output` are its token limits,
    /// an output limit above the window stating none, as some speech and
    /// video models' do,
    /// the types its input modalities give its accepted types (`models.md`
    /// § Accepted attachment types), `reasoning` its thinking
    /// support, the values of its `effort` reasoning option its efforts,
    /// `tool_call` its tool support and `cost` its prices. What the document
    /// does not state is unknown.
    fn catalog(&self, id: &str) -> ProviderModel {
        let limit = self.limit.as_ref();
        let context_window = limit.and_then(|limit| tokens(limit.context));
        let output_limit = limit
            .and_then(|limit| tokens(limit.output))
            .filter(|output| context_window.is_none_or(|window| *output <= window));
        ProviderModel {
            id: id.to_owned(),
            display_name: self.name.clone().unwrap_or_else(|| id.to_owned()),
            description: self.description.clone(),
            context_window,
            output_limit,
            supports_tools: self.tool_call,
            accepted_extensions: self
                .modalities
                .as_ref()
                .and_then(|modalities| modalities.input.as_ref())
                .map(|input| modality_extensions(input.iter().map(String::as_str))),
            supports_reasoning: self.reasoning,
            supported_thinking_efforts: self.efforts(),
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
        let option = self
            .reasoning_options
            .as_ref()?
            .iter()
            .find(|option| option.kind == "effort")?;
        let values = option.values.as_ref()?;
        Some(
            values
                .iter()
                .flatten()
                .filter(|value| !value.is_empty())
                .cloned()
                .collect(),
        )
    }
}

/// A token limit the document states as a positive whole number; zero,
/// fractions and other values state none.
fn tokens(value: Option<f64>) -> Option<u32> {
    let value = value?;
    let whole = value.fract() == 0.0 && value >= 1.0 && value <= f64::from(u32::MAX);
    // A whole number in u32's range converts exactly.
    whole.then_some(value as u32)
}
