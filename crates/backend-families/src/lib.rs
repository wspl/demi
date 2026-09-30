//! The built-in provider families, one per vendor crate (`providers.md` §
//! Families, vendors and endpoints): the registry the executable starts
//! with, beside which a test registers scripted families.

use std::rc::Rc;
use std::sync::Arc;

use demi_backend_providers::llm::families::{
    AccountBinding, FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily,
};
use demi_backend_providers::vault::accounts::SETUP_TOKEN_FAMILY;
use demi_core::WireApi;
use demi_provider::quota::{MemorySnapshots, QuotaSnapshotStore};
use demi_provider::{Provider, ProviderRuntime};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use demi_provider_claude_code::{ClaudeCodeConfig, ClaudeCodeProvider, Placement};
use demi_provider_codex::{CodexConfig, CodexProvider};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_grok_build::{GrokConfig, GrokProvider};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_web_api::providers::CredentialKind;

/// The families built into the backend.
pub fn builtin() -> FamilyRegistry {
    FamilyRegistry::default()
        .with("anthropic", AnthropicFamily)
        .with(SETUP_TOKEN_FAMILY, ClaudeCodeFamily)
        .with("codex", CodexFamily)
        .with("google", GoogleFamily)
        .with("grok-build", GrokBuildFamily)
        .with("openai", OpenAiFamily)
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
        let config = CodexConfig::new(account);
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
        let config = GrokConfig::new(account);
        let provider = GrokProvider::new(config, subscription.pool, quota, args.http, args.clock);
        Ok(Arc::new(provider))
    }
}

/// The `claude-code` family: Claude accounts by setup token, whose requests
/// run the Claude Code CLI on the user's Cloud.
struct ClaudeCodeFamily;

impl ClaudeCodeFamily {
    fn build(args: FamilyArgs) -> Result<ClaudeCodeProvider, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let (account, quota) = bound(subscription.account);
        let config = ClaudeCodeConfig::new(args.entry_id, args.label, account);
        Ok(ClaudeCodeProvider::new(
            config,
            subscription.pool,
            quota,
            args.models_dev,
            args.http,
            args.clock,
        ))
    }
}

impl ProviderFamily for ClaudeCodeFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::Subscription
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        Ok(Arc::new(Self::build(args)?))
    }

    fn process_runtime(
        &self,
        args: FamilyArgs,
        placement: Rc<dyn Placement>,
    ) -> Option<Result<Box<dyn ProviderRuntime>, FamilyError>> {
        Some(Self::build(args).map(|provider| provider.process_runtime(placement)))
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
    use demi_backend_providers::llm::families::SubscriptionArgs;
    use demi_core::{AuthState, Clock, QuotaSnapshot, SnapshotSource};
    use demi_provider::credentials::{AccountMeta, AccountsCapability, CredentialPool, MemoryCredentialPool};
    use demi_provider::models_dev::ModelsDevClient;
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
        builtin().get(family).unwrap().provider(args).unwrap()
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
        let registry = builtin();
        let subscriptions: Vec<&str> = registry.subscriptions().collect();
        assert_eq!(subscriptions, ["claude-code", "codex", "grok-build"]);

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
