//! The MiniMax provider (`crates-and-packages.md` § Vendor provider crates): a
//! MiniMax entry's media through its key. The entry is models.dev's `minimax`
//! or `minimax-cn`, whose text is MiniMax's Anthropic-shaped API, the provider
//! it is given; its images, Hailuo videos and speech are asked on its
//! endpoint's origin, `https://api.minimax.io` or `https://api.minimax.cn`.
//! MiniMax lists only its text models to a key, so the media models are the
//! catalog's.

mod images;
mod minimax;
mod speech;
mod videos;

use std::sync::Arc;

use demi_provider_common::{
    Capabilities, CatalogError, Provider, ProviderRuntime, RuntimeEnv, RuntimeError, Secret, audio::AudioMaker,
    credentials::SubscriptionAccounts, images::ImageMaker, media::MediaVendor, quota::ProviderQuota, videos::VideoMaker,
};
use demi_shared_types::{AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList, RuntimeState, Timestamp};
use futures_util::future::BoxFuture;
use reqwest::Url;

/// Which MiniMax an entry is of: each has its own accounts, keys and
/// prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// MiniMax outside China, models.dev's `minimax`.
    Global,
    /// MiniMax in China, models.dev's `minimax-cn`.
    China,
}

impl Region {
    /// The region of models.dev's vendor `id`, for a MiniMax entry; none
    /// for another vendor, a MiniMax coding plan's included, which makes
    /// no media.
    pub fn of(id: &str) -> Option<Self> {
        match MediaVendor::from_id(id)? {
            MediaVendor::Minimax => Some(Self::Global),
            MediaVendor::MinimaxCn => Some(Self::China),
            _ => None,
        }
    }

    /// Its vendor in the catalog of media models.
    pub fn vendor(self) -> MediaVendor {
        match self {
            Self::Global => MediaVendor::Minimax,
            Self::China => MediaVendor::MinimaxCn,
        }
    }

    /// The origin of an entry that names no endpoint.
    fn default_origin(self) -> &'static str {
        match self {
            Self::Global => "https://api.minimax.io/",
            Self::China => "https://api.minimax.cn/",
        }
    }
}

/// The configuration of a MiniMax entry's media, as the backend decoded it.
#[derive(Clone)]
pub struct MinimaxConfig {
    pub region: Region,
    pub api_key: Option<Secret>,
    /// The entry's endpoint, its text API's, whose origin its media are
    /// asked on; `None` for the region's.
    pub base_url: Option<Url>,
    /// Whether the key makes images, videos and speech, as the entry's
    /// shapes say.
    pub makes_images: bool,
    pub makes_videos: bool,
    pub makes_audio: bool,
}

/// A MiniMax entry: its text provider's text, and MiniMax's media.
pub struct MinimaxProvider {
    text: Arc<dyn Provider>,
    images: Option<images::MinimaxImages>,
    videos: Option<videos::Hailuo>,
    audio: Option<speech::MinimaxSpeech>,
}

impl MinimaxProvider {
    pub fn new(config: MinimaxConfig, text: Arc<dyn Provider>, clock: Arc<dyn Clock>) -> Self {
        let origin = config
            .base_url
            .map(|mut endpoint| {
                endpoint.set_path("/");
                endpoint.set_query(None);
                endpoint.set_fragment(None);
                endpoint
            })
            .unwrap_or_else(|| Url::parse(config.region.default_origin()).expect("the default origin parses"));
        let vendor = config.region.vendor();
        let key = config.api_key;
        let images = key
            .clone()
            .filter(|_| config.makes_images)
            .map(|key| images::MinimaxImages::new(vendor, key, &origin, clock.clone()));
        let audio = key
            .clone()
            .filter(|_| config.makes_audio)
            .map(|key| speech::MinimaxSpeech::new(vendor, key, &origin, clock.clone()));
        let videos = key.filter(|_| config.makes_videos).map(|key| videos::Hailuo::new(vendor, key, origin, clock));
        Self { text, images, videos, audio }
    }
}

impl Provider for MinimaxProvider {
    fn capabilities(&self) -> Capabilities {
        self.text.capabilities()
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        self.text.auth_status()
    }

    fn runtime_state(&self) -> RuntimeState {
        self.text.runtime_state()
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        self.text.list_models()
    }

    fn catalog_revision(&self) -> Option<String> {
        self.text.catalog_revision()
    }

    fn read_failure(&self, diagnostics: &ProviderErrorDiagnostics, received_at: Timestamp) -> ProviderFailureFacts {
        self.text.read_failure(diagnostics, received_at)
    }

    fn quota(&self) -> Option<&ProviderQuota> {
        self.text.quota()
    }

    fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        self.text.accounts()
    }

    fn images(&self) -> Option<&dyn ImageMaker> {
        self.images.as_ref().map(|images| images as &dyn ImageMaker)
    }

    fn videos(&self) -> Option<&dyn VideoMaker> {
        self.videos.as_ref().map(|videos| videos as &dyn VideoMaker)
    }

    fn audio(&self) -> Option<&dyn AudioMaker> {
        self.audio.as_ref().map(|audio| audio as &dyn AudioMaker)
    }

    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        self.text.runtime(env)
    }
}
