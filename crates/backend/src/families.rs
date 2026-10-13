//! The built-in provider families, one per vendor crate (`providers.md` §
//! Families, vendors and endpoints): the registry the backend starts with,
//! beside which a test registers scripted families.

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use demi_backend_providers::llm::families::{
    AccountBinding, FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily,
};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use demi_provider_claude_code::{AccountMachine, ClaudeCodeConfig, ClaudeCodeProvider, Placement};
use demi_provider_codex::{CodexConfig, CodexProvider, CodexReleases};
use demi_provider_common::credentials::SubscriptionAccounts;
use demi_provider_common::quota::{MemorySnapshots, QuotaSnapshotStore};
use demi_provider_common::{Provider, ProviderRuntime, RuntimeEnv};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_grok_build::{GrokConfig, GrokProvider};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::WireApi;
use demi_web_api_protocol::providers::CredentialKind;

/// The families built into the backend.
pub fn builtin() -> FamilyRegistry {
    FamilyRegistry::default()
        .with("anthropic", AnthropicFamily)
        .with("claude-code", ClaudeCodeFamily::default())
        .with("codex", CodexFamily::default())
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

    fn default_endpoint(&self) -> Option<&'static str> {
        Some(AnthropicConfig::DEFAULT_BASE_URL)
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(settings) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let config = AnthropicConfig {
            api_key: settings.api_key,
            base_url: settings.base_url,
            policy: settings.vendor,
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

    fn default_endpoint(&self) -> Option<&'static str> {
        Some(OpenAiConfig::DEFAULT_BASE_URL)
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

    fn default_endpoint(&self) -> Option<&'static str> {
        Some(GoogleConfig::DEFAULT_BASE_URL)
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
pub struct CodexFamily {
    /// The npm registry's record of the newest Codex CLI release.
    releases_url: url::Url,
    /// That release, one for the whole backend, made with the client and
    /// clock of the first provider's arguments: the backend's shared
    /// services', the same for every provider.
    releases: OnceLock<CodexReleases>,
}

impl CodexFamily {
    /// The family reading the newest release from `releases_url`, the
    /// product's unless a test serves its own.
    pub fn new(releases_url: url::Url) -> Self {
        Self {
            releases_url,
            releases: OnceLock::new(),
        }
    }
}

impl Default for CodexFamily {
    fn default() -> Self {
        Self::new(CodexReleases::URL.parse().expect("the registry URL parses"))
    }
}

impl ProviderFamily for CodexFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::Subscription
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let (account, quota) = bound(subscription.account);
        let releases = self.releases.get_or_init(|| {
            CodexReleases::new(self.releases_url.clone(), args.http.clone(), args.clock.clone())
        });
        let config = CodexConfig::new(account, releases.clone());
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

/// The `claude-code` family: Claude accounts signed in by the Claude Code
/// CLI's own login, whose requests run the CLI on the user's Cloud.
pub struct ClaudeCodeFamily {
    /// The OAuth usage endpoint, the product's unless a test serves its own.
    pub usage_url: url::Url,
    /// The CLI's token endpoint, the product's unless a test serves its own.
    pub token_url: url::Url,
}

impl Default for ClaudeCodeFamily {
    fn default() -> Self {
        Self {
            usage_url: ClaudeCodeConfig::USAGE_URL
                .parse()
                .expect("the usage URL parses"),
            token_url: ClaudeCodeConfig::TOKEN_URL
                .parse()
                .expect("the token URL parses"),
        }
    }
}

impl ClaudeCodeFamily {
    fn build(&self, args: FamilyArgs) -> Result<ClaudeCodeProvider, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let (account, quota) = bound(subscription.account);
        let mut config = ClaudeCodeConfig::new(args.entry_id, args.label, account);
        config.usage_url = self.usage_url.clone();
        config.token_url = self.token_url.clone();
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
        Ok(Arc::new(self.build(args)?))
    }

    fn process_runtime(
        &self,
        args: FamilyArgs,
        env: RuntimeEnv,
        placement: Rc<dyn Placement>,
    ) -> Option<Result<Box<dyn ProviderRuntime>, FamilyError>> {
        Some(
            self.build(args)
                .map(|provider| provider.process_runtime(env, placement)),
        )
    }

    fn login_accounts(
        &self,
        args: FamilyArgs,
        machine: Arc<dyn AccountMachine>,
    ) -> Option<Result<Box<dyn SubscriptionAccounts>, FamilyError>> {
        Some(
            self.build(args)
                .map(|provider| provider.login_accounts(machine)),
        )
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
