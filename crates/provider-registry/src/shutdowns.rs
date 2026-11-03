//! The days vendors shut their models down, from LiteLLM's model table
//! (`model_prices_and_context_window.json`): the `deprecation_date` it keeps
//! for far more models than models.dev marks deprecated. A vendor entry's
//! list leaves a model out from that day on (`crates/provider-registry/src/vendors.rs`).
//!
//! - The backend reads the table when it starts and once a day after, in
//!   the background: a catalog's refresh never waits for it. A read that
//!   fails keeps the table read before and is tried again in an hour; a
//!   backend that has read none has no days, so it takes nothing out.
//! - A LiteLLM model is a models.dev vendor's by its `litellm_provider`,
//!   through [`PROVIDERS`], and by its key less the provider's prefix, such
//!   as `gemini/gemini-2.5-pro` for Google's `gemini-2.5-pro`. A vendor with
//!   no row has no days.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use demi_shared_types::{Clock, ProviderModelList};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use serde::Deserialize;
use tokio::runtime::Handle;
use url::Url;

/// LiteLLM's published table.
pub const DEFAULT_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// How long a table read is kept before it is read again.
const READ_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long after a failed read the next one is tried.
const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// How long one read of the table may take: it is megabytes.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// LiteLLM's `litellm_provider` of each models.dev vendor it dates models
/// of.
const PROVIDERS: [(&str, &str); 11] = [
    ("google", "gemini"),
    ("anthropic", "anthropic"),
    ("openai", "openai"),
    ("xai", "xai"),
    ("deepseek", "deepseek"),
    ("mistral", "mistral"),
    ("groq", "groq"),
    ("moonshotai", "moonshot"),
    ("cerebras", "cerebras"),
    ("fireworks-ai", "fireworks_ai"),
    ("togetherai", "together_ai"),
];

/// Each dated model's shutdown day, by LiteLLM provider and model id.
type Days = HashMap<(String, String), Date>;

/// One model of the table; every other field is ignored.
#[derive(Deserialize)]
struct TableModel {
    #[serde(default)]
    litellm_provider: Option<String>,
    #[serde(default)]
    deprecation_date: Option<String>,
}

/// The shutdown days of the backend.
#[derive(Clone)]
pub struct ModelShutdowns(Arc<Inner>);

struct Inner {
    http: reqwest::Client,
    url: Url,
    clock: Arc<dyn Clock>,
    /// The table last read; a `std` mutex, as no section awaits.
    days: Mutex<Option<Arc<Days>>>,
}

impl ModelShutdowns {
    /// No table read yet, from `url`.
    pub fn new(http: reqwest::Client, url: Url, clock: Arc<dyn Clock>) -> Self {
        Self(Arc::new(Inner {
            http,
            url,
            clock,
            days: Mutex::new(None),
        }))
    }

    /// Reads the table now and once a day on `edge`. The task holds the
    /// table weakly, so it ends with its owners.
    pub fn start(&self, edge: &Handle) {
        let shutdowns: Weak<Inner> = Arc::downgrade(&self.0);
        edge.spawn(async move {
            loop {
                let Some(inner) = shutdowns.upgrade() else {
                    return;
                };
                let next = match read(&inner.http, &inner.url).await {
                    Ok(days) => {
                        *inner.days.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(days));
                        READ_EVERY
                    }
                    Err(error) => {
                        tracing::warn!(
                            event = "model_shutdowns.read_failed",
                            error,
                            "the models' shutdown days could not be read"
                        );
                        RETRY_AFTER
                    }
                };
                drop(inner);
                tokio::time::sleep(next).await;
            }
        });
    }

    /// Takes out of `list`, vendor `vendor_id`'s, each model whose shutdown
    /// day has come, by the table read last.
    pub fn take_out(&self, vendor_id: &str, list: &mut ProviderModelList) {
        let Some((_, provider)) = PROVIDERS.iter().find(|(vendor, _)| *vendor == vendor_id) else {
            return;
        };
        let Some(days) = self.0.days.lock().unwrap_or_else(PoisonError::into_inner).clone() else {
            return;
        };
        let today = self.0.clock.now().to_jiff().to_zoned(TimeZone::UTC).date();
        list.models.retain(|model| {
            days.get(&((*provider).to_owned(), model.id.clone()))
                .is_none_or(|shutdown| *shutdown > today)
        });
    }
}

async fn read(http: &reqwest::Client, url: &Url) -> Result<Days, String> {
    let answer = http
        .get(url.clone())
        .timeout(READ_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.without_url().to_string())?;
    let body = answer.bytes().await.map_err(|error| error.without_url().to_string())?;
    // The table is megabytes of JSON, more than an async thread may spend.
    tokio::task::spawn_blocking(move || decode(&body))
        .await
        .map_err(|error| error.to_string())?
}

/// The dated models of the table. A model whose date is not a day, as the
/// table's own sample's is not, has none.
fn decode(body: &[u8]) -> Result<Days, String> {
    let table: HashMap<String, TableModel> =
        serde_json::from_slice(body).map_err(|error| format!("LiteLLM's model table cannot be read: {error}"))?;
    Ok(table
        .into_iter()
        .filter_map(|(key, model)| {
            let provider = model.litellm_provider?;
            let day: Date = model.deprecation_date?.parse().ok()?;
            let id = key.strip_prefix(&format!("{provider}/")).unwrap_or(&key).to_owned();
            Some(((provider, id), day))
        })
        .collect())
}
