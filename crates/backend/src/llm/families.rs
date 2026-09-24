//! The provider families entries are assembled with (`providers.md` §
//! Families, vendors and endpoints): each family says how its entries
//! authenticate and builds the provider of one entry and account from the
//! entry's decoded configuration. The registry holds the built-in families;
//! a test registers scripted ones beside them.

use std::collections::BTreeMap;
use std::sync::Arc;

use demi_core::{Clock, WireApi};
use demi_provider::credentials::CredentialPool;
use demi_provider::models_dev::ModelsDevClient;
use demi_provider::quota::QuotaSnapshotStore;
use demi_provider::{Provider, Secret};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, VendorPolicy};
use demi_web_api::providers::CredentialKind;
use url::Url;

/// A provider family: `anthropic`, `codex`, or a test's scripted one.
pub trait ProviderFamily: Send + Sync + 'static {
    /// How the family's entries authenticate.
    fn credential(&self) -> CredentialKind;

    /// The wires an entry of the family may choose; none for a family that
    /// speaks one.
    fn wires(&self) -> &'static [WireApi] {
        &[]
    }

    /// The provider of one entry and account.
    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError>;
}

/// What a family builds a provider from.
pub struct FamilyArgs {
    /// The entry's id, which the provider names itself by.
    pub entry_id: String,
    /// The entry's label.
    pub label: String,
    pub credential: FamilyCredential,
    /// The HTTP client of the backend's shared services, for the provider's
    /// own requests such as its directory and quota probes. A runtime gets
    /// the client of the shard it runs on instead.
    pub http: reqwest::Client,
    pub clock: Arc<dyn Clock>,
    /// The backend's one models.dev copy, for a family whose directory it
    /// is.
    pub models_dev: ModelsDevClient,
}

/// The credential a provider stands for.
pub enum FamilyCredential {
    ApiKey(ApiKeyArgs),
    Subscription(SubscriptionArgs),
}

/// An API-key entry's settings, read only from the entry.
pub struct ApiKeyArgs {
    pub api_key: Secret,
    /// The vendor's API base; the family's default without it.
    pub base_url: Option<Url>,
    pub wire_api: Option<WireApi>,
    /// The request requirements of the vendor the entry was added from,
    /// which the OpenAI wires apply.
    pub vendor: VendorPolicy,
}

/// A subscription entry's accounts: the pool bound to the entry, and the
/// account the provider stands for, when the entry has one.
pub struct SubscriptionArgs {
    pub pool: Arc<dyn CredentialPool>,
    pub account: Option<AccountBinding>,
}

/// The account a subscription provider stands for, with its quota snapshot.
pub struct AccountBinding {
    pub credential_id: String,
    pub quota: Arc<dyn QuotaSnapshotStore>,
}

/// Why a family could not build a provider.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FamilyError {
    /// The entry's credential is not the family's kind.
    #[error("the entry's credential is not one its family takes")]
    WrongCredential,
    #[error("{0}")]
    Invalid(String),
}

/// The families of a backend by name.
#[derive(Clone, Default)]
pub struct FamilyRegistry {
    families: BTreeMap<String, Arc<dyn ProviderFamily>>,
}

impl FamilyRegistry {
    /// The families built into the backend.
    pub fn builtin() -> Self {
        Self::default()
            .with("anthropic", AnthropicFamily)
            .with("google", GoogleFamily)
            .with("openai", OpenAiFamily)
    }

    /// The registry with `family` under `name`, in place of any family of
    /// that name.
    pub fn with(mut self, name: impl Into<String>, family: impl ProviderFamily) -> Self {
        self.families.insert(name.into(), Arc::new(family));
        self
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Arc<dyn ProviderFamily>> {
        self.families.get(name)
    }

    /// The names of the subscription families, in order.
    pub(crate) fn subscriptions(&self) -> impl Iterator<Item = &str> {
        self.families
            .iter()
            .filter(|(_, family)| family.credential() == CredentialKind::Subscription)
            .map(|(name, _)| name.as_str())
    }
}

/// The `anthropic` family: the Anthropic Messages API with an API key.
struct AnthropicFamily;

impl ProviderFamily for AnthropicFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(settings) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let config = AnthropicConfig {
            id: args.entry_id,
            display_name: args.label,
            api_key: settings.api_key,
            base_url: settings.base_url,
        };
        Ok(Arc::new(AnthropicProvider::new(config, args.clock)))
    }
}

/// The `openai` family: the Responses API, or Chat Completions for an
/// OpenAI-compatible endpoint, with an API key.
struct OpenAiFamily;

impl ProviderFamily for OpenAiFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn wires(&self) -> &'static [WireApi] {
        &[WireApi::Responses, WireApi::ChatCompletions]
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(settings) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let config = OpenAiConfig {
            id: args.entry_id,
            display_name: args.label,
            api_key: settings.api_key,
            base_url: settings.base_url,
            // An entry that names no wire speaks Responses.
            wire: settings.wire_api.unwrap_or_default(),
            policy: settings.vendor,
        };
        Ok(Arc::new(OpenAiProvider::new(config, args.clock)))
    }
}

/// The `google` family: Gemini's `generateContent` API with an API key.
struct GoogleFamily;

impl ProviderFamily for GoogleFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(settings) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let config = GoogleConfig {
            id: args.entry_id,
            display_name: args.label,
            api_key: settings.api_key,
            base_url: settings.base_url,
        };
        Ok(Arc::new(GoogleProvider::new(config, args.clock)))
    }
}
