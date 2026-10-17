//! The built-in provider families, one per vendor crate (`providers.md` §
//! Families, vendors and endpoints): the registry the backend starts with,
//! beside which a test registers scripted families.

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use demi_backend_providers::llm::families::{
    AccountBinding, CompatibleImagesArgs, CompatibleVideosArgs, FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry,
    ProviderFamily,
};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use demi_provider_claude_code::{ClaudeCodeConfig, ClaudeCodeProvider, Placement};
use demi_provider_codex::{CodexConfig, CodexProvider, CodexReleases};
use demi_provider_common::credentials::SubscriptionAccounts;
use demi_provider_common::quota::{MemorySnapshots, QuotaSnapshotStore};
use demi_provider_common::{Provider, ProviderRuntime, RuntimeEnv};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_grok_build::{GrokConfig, GrokProvider};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::{ImageShape, VideoShape, WireApi};
use demi_web_api_protocol::providers::CredentialKind;

/// The families built into the backend.
pub fn builtin() -> FamilyRegistry {
    FamilyRegistry::default()
        .with_compatible_images(Arc::new(|args: CompatibleImagesArgs| {
            demi_provider_openai_api::compatible_images(args.name, args.key, &args.base, args.models, args.clock)
        }))
        .with_compatible_videos(Arc::new(|args: CompatibleVideosArgs| {
            demi_provider_openai_api::compatible_videos(args.name, args.shape, args.key, &args.base, args.models, args.clock)
        }))
        .with("anthropic", AnthropicFamily)
        .with("claude-code", ClaudeCodeFamily)
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
            vendor_id: settings.vendor_id,
            makes_images: settings.image_shape == Some(ImageShape::OpenaiImages),
            makes_videos: settings.video_shape == Some(VideoShape::XaiVideos),
        };
        Ok(Arc::new(OpenAiProvider::new(config, args.clock)))
    }

    /// An OpenAI or xAI key makes images through the OpenAI-shaped image
    /// API (`media.md` § Which entries make which media).
    fn image_shape(&self, vendor_id: Option<&str>) -> Option<ImageShape> {
        matches!(vendor_id, Some("openai" | "xai")).then_some(ImageShape::OpenaiImages)
    }

    /// An xAI key makes videos through xAI's video API; OpenAI's was shut
    /// down (`videos.md` § Vendors).
    fn video_shape(&self, vendor_id: Option<&str>) -> Option<VideoShape> {
        (vendor_id == Some("xai")).then_some(VideoShape::XaiVideos)
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
            makes_images: settings.image_shape == Some(ImageShape::Gemini),
            makes_videos: settings.video_shape == Some(VideoShape::Veo),
        };
        Ok(Arc::new(GoogleProvider::new(config, args.clock)))
    }

    /// A Gemini API key makes images with `generateContent`.
    fn image_shape(&self, vendor_id: Option<&str>) -> Option<ImageShape> {
        (vendor_id == Some("google")).then_some(ImageShape::Gemini)
    }

    /// A Gemini API key makes videos with Veo.
    fn video_shape(&self, vendor_id: Option<&str>) -> Option<VideoShape> {
        (vendor_id == Some("google")).then_some(VideoShape::Veo)
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
            CodexReleases::new(self.releases_url.clone(), args.services_http.clone(), args.clock.clone())
        });
        let config = CodexConfig::new(account, releases.clone());
        let provider = CodexProvider::new(config, subscription.pool, quota, args.http, args.clock);
        Ok(Arc::new(provider))
    }

    fn image_shape(&self, _: Option<&str>) -> Option<ImageShape> {
        Some(ImageShape::Chatgpt)
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

    fn image_shape(&self, _: Option<&str>) -> Option<ImageShape> {
        Some(ImageShape::Grok)
    }

    fn video_shape(&self, _: Option<&str>) -> Option<VideoShape> {
        Some(VideoShape::Grok)
    }
}

/// The `claude-code` family: Claude accounts signed in by the Claude Code
/// CLI's own login, whose requests run the CLI on the Claude Code machine.
#[derive(Default)]
pub struct ClaudeCodeFamily;

impl ClaudeCodeFamily {
    fn build(&self, args: FamilyArgs) -> Result<ClaudeCodeProvider, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let (account, quota) = bound(subscription.account);
        let config = ClaudeCodeConfig::new(args.entry_id, args.label, account, args.machine);
        Ok(ClaudeCodeProvider::new(
            config,
            subscription.pool,
            quota,
            args.models_dev,
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
        _env: RuntimeEnv,
        placement: Rc<dyn Placement>,
    ) -> Option<Result<Box<dyn ProviderRuntime>, FamilyError>> {
        Some(
            self.build(args)
                .map(|provider| provider.process_runtime(placement)),
        )
    }

    fn login_accounts(
        &self,
        args: FamilyArgs,
    ) -> Option<Result<Box<dyn SubscriptionAccounts>, FamilyError>> {
        let machine = args.machine.clone()?;
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
