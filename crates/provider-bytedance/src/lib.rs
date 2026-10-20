//! The ByteDance provider: ByteDance's media through its keys.
//!
//! - An Ark key, BytePlus ModelArk's or Volcengine Ark's, makes Seedream's
//!   images and Seedance's videos. A BytePlus ModelArk entry makes no text, and
//!   a Volcengine Ark entry's text is the OpenAI-compatible provider's it is
//!   given.
//! - A speech key, BytePlus Speech's or Doubao Speech's, makes Seed speech and
//!   Seed Audio's sound effects.
//!
//! No ByteDance API lists its models to an API key, so the models are the
//! catalog's, and a key is checked by a request ByteDance refuses for its body.

mod ark;
mod images;
mod speech;
mod videos;

pub use videos::compatible_videos;

use std::sync::Arc;

use demi_provider_common::{
    Capabilities, CatalogError, Provider, ProviderRuntime, RuntimeEnv, RuntimeError, Secret,
    audio::AudioMaker,
    credentials::SubscriptionAccounts,
    images::ImageMaker,
    media::MediaVendor,
    quota::ProviderQuota,
    read_http_failure,
    videos::VideoMaker,
};
use demi_shared_types::{AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState, Timestamp};
use futures_util::future::BoxFuture;
use reqwest::Url;

/// Which Ark an Ark key is of: each has its own accounts, keys, model IDs
/// and prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ark {
    /// BytePlus ModelArk, outside China.
    Byteplus,
    /// Volcengine Ark (火山方舟), in China.
    Volcengine,
}

impl Ark {
    /// Its vendor in the catalog of media models.
    pub fn vendor(self) -> MediaVendor {
        match self {
            Self::Byteplus => MediaVendor::Byteplus,
            Self::Volcengine => MediaVendor::Volcengine,
        }
    }

    /// The API base of an entry that names none.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Byteplus => "https://ark.ap-southeast.bytepluses.com/api/v3",
            Self::Volcengine => "https://ark.cn-beijing.volces.com/api/v3",
        }
    }
}

/// The configuration of an Ark key's entry, as the backend decoded it.
#[derive(Clone)]
pub struct ArkConfig {
    pub ark: Ark,
    pub api_key: Option<Secret>,
    /// The API's base, ending in `/api/v3`; `None` for the Ark's default.
    pub base_url: Option<Url>,
    /// Whether the key makes images, as the entry's image shape says.
    pub makes_images: bool,
    /// Whether the key makes videos, as the entry's video shape says.
    pub makes_videos: bool,
    /// The provider of the entry's text, for an Ark that makes text, as
    /// Volcengine's does through its OpenAI-compatible API; none for an
    /// entry of media alone.
    pub text: Option<Arc<dyn Provider>>,
}

/// An Ark key's entry, shared by every user and request of the entry.
pub struct ArkProvider {
    text: Option<Arc<dyn Provider>>,
    images: Option<images::ArkImages>,
    videos: Option<videos::ArkVideos>,
}

impl ArkProvider {
    pub fn new(config: ArkConfig, clock: Arc<dyn Clock>) -> Self {
        let base = config
            .base_url
            .unwrap_or_else(|| Url::parse(config.ark.default_base_url()).expect("the default base URL parses"));
        let images = config
            .api_key
            .clone()
            .filter(|_| config.makes_images)
            .map(|key| images::ArkImages::new(config.ark.vendor(), key, base.clone(), clock.clone()));
        let videos = config
            .api_key
            .filter(|_| config.makes_videos)
            .map(|key| videos::ArkVideos::vendor(config.ark.vendor(), key, base, clock));
        Self { text: config.text, images, videos }
    }
}

impl Provider for ArkProvider {
    fn capabilities(&self) -> Capabilities {
        self.text.as_ref().map_or_else(Capabilities::default, |text| text.capabilities())
    }

    /// Whether Ark accepts the key shows on the first request, as Test
    /// Connection's.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        match &self.text {
            Some(text) => text.auth_status(),
            None => Box::pin(async { AuthState::Authenticated { account_label: None } }),
        }
    }

    fn runtime_state(&self) -> RuntimeState {
        match &self.text {
            Some(text) => text.runtime_state(),
            None => RuntimeState::Ready { message: Some("Makes images and videos".into()) },
        }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        match &self.text {
            Some(text) => text.list_models(),
            None => Box::pin(async {
                Ok(ProviderModelList {
                    models: Vec::new(),
                    default_model_id: None,
                    warnings: Vec::new(),
                    source_fetched_at: Timestamp::UNIX_EPOCH,
                    stale: false,
                })
            }),
        }
    }

    fn catalog_revision(&self) -> Option<String> {
        self.text.as_ref().and_then(|text| text.catalog_revision())
    }

    fn read_failure(&self, diagnostics: &ProviderErrorDiagnostics, received_at: Timestamp) -> ProviderFailureFacts {
        match &self.text {
            Some(text) => text.read_failure(diagnostics, received_at),
            None => read_http_failure(diagnostics, received_at),
        }
    }

    fn quota(&self) -> Option<&ProviderQuota> {
        self.text.as_ref().and_then(|text| text.quota())
    }

    fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        self.text.as_ref().and_then(|text| text.accounts())
    }

    fn images(&self) -> Option<&dyn ImageMaker> {
        self.images.as_ref().map(|images| images as &dyn ImageMaker)
    }

    fn videos(&self) -> Option<&dyn VideoMaker> {
        self.videos.as_ref().map(|videos| videos as &dyn VideoMaker)
    }

    fn audio(&self) -> Option<&dyn AudioMaker> {
        self.text.as_ref().and_then(|text| text.audio())
    }

    /// An entry of media alone has no session's runtime; it is never in the
    /// model menu, so no session asks for one.
    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        match &self.text {
            Some(text) => text.runtime(env),
            None => Err(RuntimeError::NoText { provider: self.images.as_ref().map_or("Ark", |images| images.label()).to_owned() }),
        }
    }
}

/// Which ByteDance speech service a speech key is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speech {
    /// BytePlus Speech, outside China.
    Byteplus,
    /// Doubao Speech (豆包语音), in China.
    Doubao,
}

impl Speech {
    /// Its vendor in the catalog of media models.
    pub fn vendor(self) -> MediaVendor {
        match self {
            Self::Byteplus => MediaVendor::ByteplusSpeech,
            Self::Doubao => MediaVendor::DoubaoSpeech,
        }
    }

    /// The API base of an entry that names none.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Byteplus => "https://voice.ap-southeast-1.bytepluses.com",
            Self::Doubao => "https://openspeech.bytedance.com",
        }
    }
}

/// The configuration of a speech key's entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct SpeechConfig {
    pub speech: Speech,
    pub api_key: Option<Secret>,
    /// The API's base; `None` for the service's default.
    pub base_url: Option<Url>,
}

/// A speech key's entry, which makes no text.
pub struct SpeechProvider {
    audio: Option<speech::SeedAudio>,
}

impl SpeechProvider {
    pub fn new(config: SpeechConfig, clock: Arc<dyn Clock>) -> Self {
        let base = config
            .base_url
            .unwrap_or_else(|| Url::parse(config.speech.default_base_url()).expect("the default base URL parses"));
        Self { audio: config.api_key.map(|key| speech::SeedAudio::new(config.speech, key, base, clock)) }
    }
}

impl Provider for SpeechProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// Whether ByteDance accepts the key shows on the first request, as
    /// Test Connection's.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async { AuthState::Authenticated { account_label: None } })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready { message: Some("Makes speech and sound effects".into()) }
    }

    fn read_failure(&self, diagnostics: &ProviderErrorDiagnostics, received_at: Timestamp) -> ProviderFailureFacts {
        read_http_failure(diagnostics, received_at)
    }

    fn audio(&self) -> Option<&dyn AudioMaker> {
        self.audio.as_ref().map(|audio| audio as &dyn AudioMaker)
    }

    /// An entry that makes no text has no session's runtime; it is never in
    /// the model menu, so no session asks for one.
    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Err(RuntimeError::NoText { provider: self.audio.as_ref().map_or("ByteDance", |audio| audio.label()).to_owned() })
    }
}
