//! The built-in provider families, one per vendor crate
//! (`crates/backend-providers/src/llm/families.rs`).
//!
//! They are the registry the backend starts with, beside which a test registers
//! scripted families.

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use demi_backend_providers::llm::assembly::MEDIA_FAMILY;
use demi_backend_providers::llm::families::{
    AccountBinding, CompatibleAudioArgs, CompatibleImagesArgs, CompatibleVideosArgs, FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry,
    ProviderFamily,
};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use demi_provider_claude_code::{ClaudeCodeConfig, ClaudeCodeProvider, Placement};
use demi_provider_codex::{CodexConfig, CodexProvider, CodexReleases};
use demi_provider_common::credentials::SubscriptionAccounts;
use demi_provider_common::quota::{MemorySnapshots, QuotaSnapshotStore};
use demi_provider_common::media::MediaVendor;
use demi_provider_common::{Provider, ProviderRuntime, RuntimeEnv};
use demi_provider_bytedance::{Ark, ArkConfig, ArkProvider, Speech, SpeechConfig, SpeechProvider};
use demi_provider_elevenlabs::{ElevenLabsConfig, ElevenLabsProvider};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_grok_build::{GrokConfig, GrokProvider, GrokReleases};
use demi_provider_minimax::{MinimaxConfig, MinimaxProvider, Region};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::{AudioShape, ImageShape, VideoShape, WireApi};
use demi_web_api_protocol::providers::CredentialKind;

/// The families built into the backend.
pub fn builtin() -> FamilyRegistry {
    FamilyRegistry::default()
        .with_compatible_images(Arc::new(|args: CompatibleImagesArgs| {
            demi_provider_openai_api::compatible_images(args.name, args.key, &args.base, args.models, args.clock)
        }))
        .with_compatible_videos(Arc::new(|args: CompatibleVideosArgs| match args.shape {
            VideoShape::ArkTasks => demi_provider_bytedance::compatible_videos(args.name, args.key, &args.base, args.models, args.clock),
            shape => demi_provider_openai_api::compatible_videos(args.name, shape, args.key, &args.base, args.models, args.clock),
        }))
        .with_compatible_audio(Arc::new(|args: CompatibleAudioArgs| {
            demi_provider_openai_api::compatible_audio(args.name, args.key, &args.base, args.models, args.clock)
        }))
        .with("anthropic", AnthropicFamily)
        .with("claude-code", ClaudeCodeFamily)
        .with("codex", CodexFamily::default())
        .with("google", GoogleFamily)
        .with("grok-build", GrokBuildFamily::new())
        .with(MEDIA_FAMILY, MediaFamily)
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
        // A MiniMax entry's text is its Anthropic-shaped API's, and its
        // media are MiniMax's crate's.
        let minimax = settings.vendor_id.as_deref().and_then(Region::of).map(|region| MinimaxConfig {
            region,
            api_key: settings.api_key.clone(),
            base_url: settings.base_url.clone(),
            makes_images: settings.image_shape == Some(ImageShape::Minimax),
            makes_videos: settings.video_shape == Some(VideoShape::Hailuo),
            makes_audio: settings.audio_shape == Some(AudioShape::Minimax),
        });
        let config = AnthropicConfig {
            api_key: settings.api_key,
            base_url: settings.base_url,
            policy: settings.vendor,
        };
        let text: Arc<dyn Provider> = Arc::new(AnthropicProvider::new(config, args.clock.clone()));
        match minimax {
            Some(minimax) => Ok(Arc::new(MinimaxProvider::new(minimax, text, args.clock))),
            None => Ok(text),
        }
    }

    /// A MiniMax key makes images, Hailuo's videos and speech; its coding
    /// plans make none.
    fn image_shape(&self, vendor_id: Option<&str>) -> Option<ImageShape> {
        vendor_id.and_then(Region::of).map(|_| ImageShape::Minimax)
    }

    fn video_shape(&self, vendor_id: Option<&str>) -> Option<VideoShape> {
        vendor_id.and_then(Region::of).map(|_| VideoShape::Hailuo)
    }

    fn audio_shape(&self, vendor_id: Option<&str>) -> Option<AudioShape> {
        vendor_id.and_then(Region::of).map(|_| AudioShape::Minimax)
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
        // Volcengine Ark's text is its OpenAI-compatible API's, and its
        // media are ByteDance's crate's.
        let ark = (settings.vendor_id.as_deref() == Some(MediaVendor::Volcengine.id())).then(|| ArkConfig {
            ark: Ark::Volcengine,
            api_key: settings.api_key.clone(),
            base_url: settings.base_url.clone(),
            makes_images: settings.image_shape == Some(ImageShape::Seedream),
            makes_videos: settings.video_shape == Some(VideoShape::ArkTasks),
            text: None,
        });
        let config = OpenAiConfig {
            api_key: settings.api_key,
            base_url: settings.base_url,
            // An entry that names no wire speaks Responses.
            wire: settings.wire_api.unwrap_or_default(),
            policy: settings.vendor,
            vendor_id: settings.vendor_id,
            makes_images: settings.image_shape == Some(ImageShape::OpenaiImages),
            makes_videos: settings.video_shape == Some(VideoShape::XaiVideos),
            makes_audio: settings.audio_shape == Some(AudioShape::OpenaiSpeech),
        };
        let text: Arc<dyn Provider> = Arc::new(OpenAiProvider::new(config, args.clock.clone()));
        match ark {
            Some(ark) => Ok(Arc::new(ArkProvider::new(ArkConfig { text: Some(text), ..ark }, args.clock))),
            None => Ok(text),
        }
    }

    /// An OpenAI or xAI key makes images through the OpenAI-shaped image
    /// API, a Volcengine Ark key through Seedream's.
    fn image_shape(&self, vendor_id: Option<&str>) -> Option<ImageShape> {
        match vendor_id {
            Some("openai" | "xai") => Some(ImageShape::OpenaiImages),
            Some(id) if id == MediaVendor::Volcengine.id() => Some(ImageShape::Seedream),
            _ => None,
        }
    }

    /// An xAI key makes videos through xAI's video API, a Volcengine Ark
    /// key through Ark Tasks; OpenAI's was shut down.
    fn video_shape(&self, vendor_id: Option<&str>) -> Option<VideoShape> {
        match vendor_id {
            Some("xai") => Some(VideoShape::XaiVideos),
            Some(id) if id == MediaVendor::Volcengine.id() => Some(VideoShape::ArkTasks),
            _ => None,
        }
    }

    /// An OpenAI key makes speech; xAI's speech comes later.
    fn audio_shape(&self, vendor_id: Option<&str>) -> Option<AudioShape> {
        (vendor_id == Some("openai")).then_some(AudioShape::OpenaiSpeech)
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
            makes_audio: settings.audio_shape == Some(AudioShape::Gemini),
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

    /// A Gemini API key makes speech and, with Lyria, music.
    fn audio_shape(&self, vendor_id: Option<&str>) -> Option<AudioShape> {
        (vendor_id == Some("google")).then_some(AudioShape::Gemini)
    }
}

/// The `media` family: an API-key entry of a vendor that makes media and no
/// text, such as ElevenLabs or BytePlus ModelArk, built by its vendor's crate.
///
/// A vendor whose shapes are not built yet, Stability AI, has none here, so it
/// is not offered.
///
/// These vendors sell media without a text model and models.dev lists none of
/// them, so the backend offers them beside models.dev's vendors, from the
/// media catalog. The family makes no text request, so its entry is never in
/// the model menu.
///
/// Its key is checked by its vendor's media model list, not a chat message. A
/// vendor whose API lists no model to its key (ByteDance's) has its key checked
/// by a request the vendor refuses for its body before making anything, such
/// as an image request with an empty prompt: a refusal of the body passes the
/// key, a refusal of the key fails it.
struct MediaFamily;

impl ProviderFamily for MediaFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(settings) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        match settings.vendor_id.as_deref().and_then(MediaVendor::from_id) {
            Some(MediaVendor::Elevenlabs) => {
                let config = ElevenLabsConfig { api_key: settings.api_key, base_url: settings.base_url };
                Ok(Arc::new(ElevenLabsProvider::new(config, args.clock)))
            }
            Some(MediaVendor::Byteplus) => {
                let config = ArkConfig {
                    ark: Ark::Byteplus,
                    api_key: settings.api_key,
                    base_url: settings.base_url,
                    makes_images: settings.image_shape == Some(ImageShape::Seedream),
                    makes_videos: settings.video_shape == Some(VideoShape::ArkTasks),
                    text: None,
                };
                Ok(Arc::new(ArkProvider::new(config, args.clock)))
            }
            Some(vendor @ (MediaVendor::ByteplusSpeech | MediaVendor::DoubaoSpeech)) => {
                let speech = if vendor == MediaVendor::ByteplusSpeech { Speech::Byteplus } else { Speech::Doubao };
                let config = SpeechConfig { speech, api_key: settings.api_key, base_url: settings.base_url };
                Ok(Arc::new(SpeechProvider::new(config, args.clock)))
            }
            _ => Err(FamilyError::Invalid(format!(
                "the media family makes nothing for vendor {}",
                settings.vendor_id.as_deref().unwrap_or("none")
            ))),
        }
    }

    fn makes_text(&self) -> bool {
        false
    }

    /// A BytePlus ModelArk key makes Seedream's images.
    fn image_shape(&self, vendor_id: Option<&str>) -> Option<ImageShape> {
        (vendor_id == Some(MediaVendor::Byteplus.id())).then_some(ImageShape::Seedream)
    }

    /// A BytePlus ModelArk key makes Seedance's videos through Ark Tasks.
    fn video_shape(&self, vendor_id: Option<&str>) -> Option<VideoShape> {
        (vendor_id == Some(MediaVendor::Byteplus.id())).then_some(VideoShape::ArkTasks)
    }

    /// An ElevenLabs key makes speech, music and sound effects; a BytePlus
    /// Speech or Doubao Speech key Seed speech and Seed Audio's effects.
    fn audio_shape(&self, vendor_id: Option<&str>) -> Option<AudioShape> {
        match vendor_id.and_then(MediaVendor::from_id) {
            Some(MediaVendor::Elevenlabs) => Some(AudioShape::Elevenlabs),
            Some(MediaVendor::ByteplusSpeech | MediaVendor::DoubaoSpeech) => Some(AudioShape::Seed),
            _ => None,
        }
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
struct GrokBuildFamily {
    /// The newest stable Grok CLI release, one for the whole backend, made
    /// with the client and clock of the first provider's arguments: the
    /// backend's shared services', the same for every provider.
    releases: OnceLock<GrokReleases>,
}

impl GrokBuildFamily {
    fn new() -> Self {
        Self {
            releases: OnceLock::new(),
        }
    }
}

impl ProviderFamily for GrokBuildFamily {
    fn credential(&self) -> CredentialKind {
        CredentialKind::Subscription
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let (account, quota) = bound(subscription.account);
        let releases = self.releases.get_or_init(|| {
            let url = GrokReleases::URL.parse().expect("the pointer URL parses");
            GrokReleases::new(url, args.services_http.clone(), args.clock.clone())
        });
        let config = GrokConfig::new(account, releases.clone());
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
