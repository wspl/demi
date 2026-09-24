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
use demi_provider::quota::{MemorySnapshots, QuotaSnapshotStore};
use demi_provider::{Provider, Secret};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use demi_provider_codex::{CodexConfig, CodexProvider};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_grok_build::{GrokConfig, GrokProvider};
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
            .with("codex", CodexFamily)
            .with("google", GoogleFamily)
            .with("grok-build", GrokBuildFamily)
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

/// The `codex` family: ChatGPT accounts on the Codex backend, signed in by
/// Codex's device login.
struct CodexFamily;

impl ProviderFamily for CodexFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::Subscription
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let (account, quota) = bound(subscription.account);
        let config = CodexConfig::new(args.entry_id, args.label, account);
        let provider = CodexProvider::new(config, subscription.pool, quota, args.http, args.clock);
        Ok(Arc::new(provider))
    }
}

/// The `grok-build` family: Grok accounts on the chat proxy of Grok's own
/// CLI, signed in by device authorization at `auth.x.ai`.
struct GrokBuildFamily;

impl ProviderFamily for GrokBuildFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::Subscription
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let (account, quota) = bound(subscription.account);
        let config = GrokConfig::new(args.entry_id, args.label, account);
        let provider = GrokProvider::new(config, subscription.pool, quota, args.http, args.clock);
        Ok(Arc::new(provider))
    }
}

/// The account a subscription provider stands for and the store of its
/// quota. A provider without an account, such as one built to log in, can
/// neither infer nor probe, so its quota store is one held in memory that
/// nothing writes.
fn bound(account: Option<AccountBinding>) -> (Option<String>, Arc<dyn QuotaSnapshotStore>) {
    match account {
        Some(binding) => (Some(binding.credential_id), binding.quota),
        None => (None, Arc::new(MemorySnapshots::new())),
    }
}

#[cfg(test)]
mod tests {
    use demi_core::{AuthState, QuotaSnapshot, SnapshotSource};
    use demi_provider::credentials::{AccountMeta, AccountsCapability, CredentialPool, MemoryCredentialPool};
    use demi_provider::testing::{FixedClock, jwt};
    use serde_json::json;

    use super::*;

    const NOW: &str = "2026-09-18T14:00:00.000Z";

    /// A provider of the built-in `family` over `pool`, for `account`. No
    /// test here makes a request.
    fn built(family: &str, pool: &MemoryCredentialPool, account: Option<AccountBinding>) -> Arc<dyn Provider> {
        let clock: Arc<dyn Clock> = Arc::new(FixedClock(NOW.parse().unwrap()));
        let url = ModelsDevClient::DEFAULT_URL.parse().unwrap();
        let args = FamilyArgs {
            entry_id: "entry-1".into(),
            label: family.into(),
            credential: FamilyCredential::Subscription(SubscriptionArgs {
                pool: Arc::new(pool.clone()),
                account,
            }),
            http: reqwest::Client::new(),
            clock: clock.clone(),
            models_dev: ModelsDevClient::new(reqwest::Client::new(), url, clock),
        };
        FamilyRegistry::builtin().get(family).unwrap().provider(args).unwrap()
    }

    /// An account whose snapshot says it was probed.
    async fn account(pool: &MemoryCredentialPool, secret: serde_json::Value) -> AccountBinding {
        let meta = AccountMeta {
            id: "cred-1".into(),
            label: "user@example.com".into(),
            detail: None,
            updated_at: NOW.parse().unwrap(),
            source: "login:device".into(),
            identity_key: None,
        };
        pool.write(meta, secret.to_string()).await.unwrap();
        let quota = Arc::new(MemorySnapshots::new());
        quota.update(&mut |_| QuotaSnapshot {
            observed_at: NOW.parse().unwrap(),
            source: SnapshotSource::Probe,
            plan: None,
            account_label: Some("user@example.com".into()),
            windows: Vec::new(),
        });
        AccountBinding {
            credential_id: "cred-1".into(),
            quota,
        }
    }

    #[tokio::test]
    async fn the_subscription_families_log_in_by_device_and_stand_for_their_bound_account() {
        let registry = FamilyRegistry::builtin();
        let subscriptions: Vec<&str> = registry.subscriptions().collect();
        assert_eq!(subscriptions, ["codex", "grok-build"]);

        let codex = json!({
            "accessToken": jwt(&json!({ "exp": 1_900_000_000 })),
            "refreshToken": "refresh-1",
            "idToken": jwt(&json!({ "email": "user@example.com" })),
            "accountId": "acct-1",
            "lastRefresh": NOW,
        });
        let grok = json!({
            "accessToken": "session-token",
            "issuer": "https://auth.x.ai",
            "clientId": "client-1",
            "email": "user@example.com",
        });
        for (family, secret, name) in [("codex", codex, "Codex"), ("grok-build", grok, "Grok")] {
            // Built to log in: the device login, and no account yet.
            let staged = MemoryCredentialPool::new();
            let login = built(family, &staged, None);
            let device_login = AccountsCapability {
                login: true,
                add: false,
            };
            assert_eq!(login.accounts().unwrap().capability(), device_login, "{family}");
            let unauthenticated = AuthState::Unauthenticated {
                message: Some(format!("No {name} account is signed in")),
            };
            assert_eq!(login.auth_status().await, unauthenticated, "{family}");

            // Built for an entry's account: that account's secret and quota.
            let pool = MemoryCredentialPool::new();
            let binding = account(&pool, secret).await;
            let quota = binding.quota.clone();
            let provider = built(family, &pool, Some(binding));
            let signed_in = AuthState::Authenticated {
                account_label: Some("user@example.com".into()),
            };
            assert_eq!(provider.auth_status().await, signed_in, "{family}");
            assert_eq!(provider.quota().unwrap().latest(), quota.latest(), "{family}");
        }
    }
}
