//! The Codex catalog (`models.md` § The Codex catalog): the account's model
//! list on the ChatGPT backend, read with the version of the newest Codex
//! CLI release, because the backend picks the models it lists by that
//! version. The newest release is the npm registry's `latest` version of
//! `@openai/codex`, as the CLI's own update check finds it.

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use demi_provider_common::{
    CatalogError,
    kept::KeptRead,
    wire::{NonEmpty, decode_untagged},
};
use demi_shared_types::{Clock, ProviderModel, ProviderModelList, ServiceTier};
use http::{HeaderValue, StatusCode, header::ACCEPT};
use jiff::SignedDuration;
use reqwest::Url;
use semver::Version;
use serde::{Deserialize, Deserializer};
use tokio::runtime::Handle;

use crate::{Shared, request::account_headers};

/// The Codex CLI release built into the provider, which the catalog sends
/// until the newest one is first read.
const CLIENT_VERSION: &str = "0.162.1";

/// How long the newest release is believed before the registry is read
/// again.
const VERSION_LIFETIME: SignedDuration = SignedDuration::from_hours(6);

/// How long one read of the registry may take.
const VERSION_READ_TIMEOUT: Duration = Duration::from_secs(15);

/// The version of the newest Codex CLI release, kept six hours and read
/// again in the background, so that no catalog request waits for it.
pub(crate) struct ClientVersion(Arc<Registry>);

struct Registry {
    /// The registry's `latest` record of `@openai/codex`.
    url: Url,
    http: reqwest::Client,
    newest: KeptRead<Version>,
}

impl ClientVersion {
    pub(crate) fn new(url: Url, http: reqwest::Client, clock: Arc<dyn Clock>) -> Self {
        Self(Arc::new(Registry {
            url,
            http,
            newest: KeptRead::new(VERSION_LIFETIME, clock),
        }))
    }

    /// The version a catalog request sends, at once: the last one read, or
    /// the built-in one before a read succeeds. When that is older than six
    /// hours or missing, the registry is read in the background for a
    /// later request, on the runtime of the caller.
    fn current(&self) -> String {
        let registry = self.0.clone();
        // The read ends within `VERSION_READ_TIMEOUT`. A failed one keeps the
        // version read before, and a later request reads again.
        let read_again = async move {
            match registry.newest.read(false, || registry.latest()).await {
                Ok(version) => tracing::debug!("the newest Codex CLI release is {version}"),
                Err(reason) => {
                    tracing::warn!("the newest Codex CLI release was not read: {reason}");
                }
            }
        };
        match self.0.newest.kept(&Handle::current(), read_again) {
            Some(kept) => kept.value.to_string(),
            None => CLIENT_VERSION.to_owned(),
        }
    }
}

impl Registry {
    /// The `latest` version the registry names: a bare `MAJOR.MINOR.PATCH`,
    /// which is what the CLI sends; a prerelease is no answer.
    async fn latest(&self) -> Result<Version, String> {
        let response = self
            .http
            .get(self.url.clone())
            .header(ACCEPT, "application/json")
            .timeout(VERSION_READ_TIMEOUT)
            .send()
            .await
            .map_err(|error| format!("the registry did not answer ({})", error.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("the registry answered {status}"));
        }
        let text = response
            .text()
            .await
            .map_err(|error| format!("the registry did not answer ({})", error.without_url()))?;
        let latest: LatestRelease =
            decode_untagged(&text).map_err(|error| format!("its answer cannot be read: {error}"))?;
        let version = latest.version;
        if !version.pre.is_empty() || !version.build.is_empty() {
            return Err(format!("it names version {version}, which is not a release"));
        }
        Ok(version)
    }
}

/// The registry's record of a package's `latest` version; only the field
/// Demi reads.
#[derive(Deserialize)]
struct LatestRelease {
    version: Version,
}

/// Codex offers its Fast mode as the `priority` service tier.
const FAST_TIER: &str = "priority";

/// A fresh read of the account's model list. A request refused with HTTP 401
/// refreshes the token that was refused, once, and reads again.
pub(crate) async fn list(shared: &Shared) -> Result<ProviderModelList, CatalogError> {
    let url = models_url(&shared.models_url, &shared.client_version.current());
    tracing::debug!("reading the Codex catalog at {url}");
    let mut refused_token = None;
    loop {
        let credentials = shared
            .auth
            .credentials(&shared.http, refused_token.as_ref())
            .await
            .map_err(|failure| failure.catalog_error())?;
        let mut headers = account_headers(&credentials, &shared.user_agent);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let response = shared
            .http
            .get(url.clone())
            .headers(headers)
            .send()
            .await
            .map_err(|error| {
                CatalogError::Unavailable(format!(
                    "Codex models request failed: {}",
                    error.without_url()
                ))
            })?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED && refused_token.is_none() {
            refused_token = Some(credentials.access_token);
            continue;
        }
        if !status.is_success() {
            return Err(CatalogError::Unavailable(format!(
                "Codex models request failed with HTTP {}",
                status.as_u16()
            )));
        }
        let text = response.text().await.map_err(|error| {
            CatalogError::Unavailable(format!(
                "Codex models request failed: {}",
                error.without_url()
            ))
        })?;
        return catalog(&text, &*shared.clock);
    }
}

/// The catalog a models answer describes: the models the CLI's picker
/// lists, by ascending priority, the first the default. A malformed answer
/// is refused, never read around.
fn catalog(text: &str, clock: &dyn Clock) -> Result<ProviderModelList, CatalogError> {
    let answer: ModelsAnswer = decode_untagged(text).map_err(|error| {
        CatalogError::Invalid(format!("Codex models answer cannot be read: {error}"))
    })?;
    let mut listed: Vec<CodexModel> = answer
        .models
        .into_iter()
        .filter(|model| model.visibility == Visibility::List)
        .collect();
    listed.sort_by_key(|model| model.priority);
    let models: Vec<ProviderModel> = listed.into_iter().map(CodexModel::into_model).collect();
    Ok(ProviderModelList {
        default_model_id: models.first().map(|model| model.id.clone()),
        models,
        warnings: Vec::new(),
        source_fetched_at: clock.now(),
        stale: false,
    })
}

#[derive(Deserialize)]
struct ModelsAnswer {
    models: Vec<CodexModel>,
}

/// One model as the backend describes it; only the fields Demi reads.
#[derive(Deserialize)]
struct CodexModel {
    slug: NonEmpty,
    display_name: NonEmpty,
    #[serde(default)]
    description: Option<String>,
    visibility: Visibility,
    priority: i64,
    #[serde(default)]
    context_window: Option<NonZeroU32>,
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
    supported_reasoning_levels: Vec<ReasoningLevel>,
    #[serde(default)]
    service_tiers: Option<Vec<CodexTier>>,
    #[serde(default)]
    default_service_tier: Option<NonEmpty>,
    #[serde(default)]
    tool_mode: Option<String>,
    #[serde(default)]
    experimental_supported_tools: Option<Vec<String>>,
    /// Present, even as `null`, when the model has an apply-patch tool.
    #[serde(default, deserialize_with = "present")]
    apply_patch_tool_type: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    web_search_tool_type: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Visibility {
    List,
    Hide,
    None,
}

#[derive(Deserialize)]
struct ReasoningLevel {
    effort: NonEmpty,
}

#[derive(Deserialize)]
struct CodexTier {
    id: NonEmpty,
    name: NonEmpty,
    #[serde(default)]
    description: Option<String>,
}

impl CodexModel {
    fn into_model(self) -> ProviderModel {
        let supports_tools = self.supports_tools();
        let images = self
            .input_modalities
            .as_ref()
            .map(|modalities| modalities.iter().any(|modality| modality == "image"));
        let efforts: Vec<String> = self
            .supported_reasoning_levels
            .into_iter()
            .map(|level| level.effort.0)
            .collect();
        ProviderModel {
            id: self.slug.0,
            display_name: self.display_name.0,
            description: self.description,
            context_window: self.context_window.map(NonZeroU32::get),
            output_limit: None,
            supports_tools,
            // The backend documents no file part, so the list names the
            // images alone (`models.md` § Accepted attachment types).
            accepted_extensions: images.map(|images| {
                if images {
                    demi_shared_types::IMAGE_FILE_EXTENSIONS.to_vec()
                } else {
                    Vec::new()
                }
            }),
            supports_reasoning: Some(!efforts.is_empty()),
            supported_thinking_efforts: Some(efforts),
            // Leaving reasoning out gets Codex's default, not no reasoning.
            can_disable_thinking: Some(false),
            service_tiers: self
                .service_tiers
                .into_iter()
                .flatten()
                .map(|tier| ServiceTier {
                    fast: tier.id.0 == FAST_TIER,
                    id: tier.id.0,
                    label: tier.name.0,
                    description: tier
                        .description
                        .filter(|description| !description.is_empty()),
                })
                .collect(),
            default_service_tier_id: self.default_service_tier.map(|tier| tier.0),
            cost: None,
        }
    }

    /// Whether the model calls tools, by the tool fields the backend names;
    /// unknown when it names none.
    fn supports_tools(&self) -> Option<bool> {
        if self
            .tool_mode
            .as_deref()
            .is_some_and(|mode| !mode.is_empty())
        {
            return Some(true);
        }
        if let Some(tools) = &self.experimental_supported_tools {
            return Some(!tools.is_empty());
        }
        if self.apply_patch_tool_type.is_some() || self.web_search_tool_type.is_some() {
            return Some(true);
        }
        None
    }
}

/// A field that is present, `null` included.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

/// The catalog request's URL: the backend's `models`, with the client
/// version the backend picks models by.
fn models_url(models: &Url, version: &str) -> Url {
    let mut url = models.clone();
    url.query_pairs_mut().append_pair("client_version", version);
    url
}
