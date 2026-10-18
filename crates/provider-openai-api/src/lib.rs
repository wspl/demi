//! The OpenAI API provider (`crates-and-packages.md` § Vendor provider
//! crates): an `openai` entry, which sends its API key when it has one,
//! speaking the Responses API or, for OpenAI-compatible endpoints, Chat
//! Completions, with the typed vendor policy its entry names and the
//! standard reading of its failure records; an OpenAI or xAI vendor
//! entry's key also makes images, an xAI vendor entry's key videos, and an
//! OpenAI vendor entry's key speech.

mod audio;
mod images;
mod request;
mod run;
mod videos;

pub use audio::compatible_audio;
pub use images::compatible_images;
pub use videos::compatible_videos;

use std::sync::Arc;

use demi_provider_common::{
    Capabilities, InferenceRequest, Provider, ProviderRun, ProviderRuntime,
    RequestLimits, ToolResultKinds, RuntimeEnv, RuntimeError, Secret, VendorPolicy, endpoint_url, read_http_failure,
    audio::AudioMaker,
    images::ImageMaker,
    media::MediaVendor,
    videos::VideoMaker,
};
use demi_shared_types::{
    AuthState, Clock, Model, ProviderErrorDiagnostics, ProviderFailureFacts,
    RuntimeState, Timestamp, WireApi,
};
use futures_util::{
    StreamExt,
    future::{BoxFuture, LocalBoxFuture},
};
use reqwest::{Url, header::HeaderValue};

/// The vendor of an entry that names none, a compatible API, as the
/// prefix of its reasoning items names it.
const COMPATIBLE: &str = "compatible";

/// The configuration of an `openai` entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// Sent when set: a compatible server of the user's own may take none.
    pub api_key: Option<Secret>,
    /// The API's base, such as `https://api.openai.com/v1`; `None` for that
    /// default. The provider appends the request's path unless the URL
    /// already ends with it.
    pub base_url: Option<Url>,
    pub wire: WireApi,
    pub policy: VendorPolicy,
    /// The models.dev vendor the entry was added from, such as `openai` or
    /// `xai`; none for a compatible API. It prefixes the reasoning items
    /// the provider receives, so that each vendor replays only its own
    /// ciphertext, and an xAI entry's images are xAI's requests.
    pub vendor_id: Option<String>,
    /// Whether the entry's key makes images, as its vendor's image shape
    /// says: an OpenAI or xAI key's does.
    pub makes_images: bool,
    /// Whether the entry's key makes videos, as its vendor's video shape
    /// says: an xAI key's does.
    pub makes_videos: bool,
    /// Whether the entry's key makes speech, as its vendor's audio shape
    /// says: an OpenAI key's does.
    pub makes_audio: bool,
}

impl OpenAiConfig {
    /// The base URL of an entry that names none (`providers.md` § Endpoints).
    pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
}

/// An `openai` entry, shared by every user and request of the entry.
pub struct OpenAiProvider {
    shared: Arc<Shared>,
    /// The key's image role, for an OpenAI or xAI entry with its key.
    images: Option<images::KeyImages>,
    /// The key's video role, for an xAI entry with its key.
    videos: Option<videos::KeyVideos>,
    /// The key's audio role, for an OpenAI entry with its key.
    audio: Option<audio::KeyAudio>,
}

/// What the provider and all its runtimes share.
struct Shared {
    authorization: Option<HeaderValue>,
    /// `<vendor>:`, the prefix of the reasoning items it receives.
    signature_tag: Arc<str>,
    url: Url,
    wire: WireApi,
    policy: VendorPolicy,
    clock: Arc<dyn Clock>,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig, clock: Arc<dyn Clock>) -> Self {
        let base = config.base_url.unwrap_or_else(|| {
            Url::parse(OpenAiConfig::DEFAULT_BASE_URL).expect("the default base URL parses")
        });
        let path = match config.wire {
            WireApi::Responses => "/responses",
            WireApi::ChatCompletions => "/chat/completions",
        };
        let vendor = config.vendor_id.as_deref();
        let image_vendor = match vendor {
            Some("xai") => MediaVendor::Xai,
            _ => MediaVendor::OpenAi,
        };
        let images = config
            .api_key
            .clone()
            .filter(|_| config.makes_images)
            .map(|key| images::KeyImages::new(image_vendor, key, &base, clock.clone()));
        let videos = config
            .api_key
            .clone()
            .filter(|_| config.makes_videos)
            .map(|key| videos::KeyVideos::xai(key, &base, clock.clone()));
        let audio = config
            .api_key
            .clone()
            .filter(|_| config.makes_audio)
            .map(|key| audio::KeyAudio::openai(key, &base, clock.clone()));
        let signature_tag = Arc::from(format!("{}:", vendor.unwrap_or(COMPATIBLE)));
        Self {
            shared: Arc::new(Shared {
                authorization: config.api_key.as_ref().map(Secret::bearer),
                signature_tag,
                url: endpoint_url(&base, path),
                wire: config.wire,
                policy: config.policy,
                clock,
            }),
            images,
            videos,
            audio,
        }
    }
}

impl Provider for OpenAiProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// Whether the vendor accepts the entry's key, or its lack of one,
    /// shows on the first request.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            AuthState::Authenticated {
                account_label: None,
            }
        })
    }

    fn runtime_state(&self) -> RuntimeState {
        let message = match self.shared.wire {
            WireApi::Responses => "Uses the OpenAI Responses API",
            WireApi::ChatCompletions => "Uses the OpenAI Chat Completions API",
        };
        RuntimeState::Ready {
            message: Some(message.into()),
        }
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

    fn read_failure(
        &self,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> ProviderFailureFacts {
        read_http_failure(diagnostics, received_at)
    }

    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(OpenAiRuntime {
            shared: self.shared.clone(),
            http: env.http,
        }))
    }
}

/// A session's runtime: it holds nothing between runs.
struct OpenAiRuntime {
    shared: Arc<Shared>,
    http: demi_provider_common::ProviderHttp,
}

impl ProviderRuntime for OpenAiRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        run::run(self.shared.clone(), self.http.clone(), request).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(OpenAiRuntime {
            shared: self.shared.clone(),
            http: self.http.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// The OpenAI API's, on either wire; a compatible vendor that takes less
    /// refuses a request as too large, which compaction answers.
    fn request_limits(&self, _model: &Model) -> RequestLimits {
        // Images and documents in a user message right after the call's
        // output, in both APIs (`providers.md` § Media in tool results).
        RequestLimits {
            tool_results: ToolResultKinds::IMAGES_AND_DOCUMENTS,
            ..RequestLimits::OPENAI
        }
    }
}

/// Test support (feature `testing`): the bodies this provider sends for a
/// request, for the tests of what sends its requests.
#[cfg(feature = "testing")]
pub mod testing {
    use demi_provider_common::{InferenceRequest, VendorPolicy};

    /// The Responses API body of `request` for OpenAI's own entry, of
    /// `policy`.
    pub fn responses_body(request: &InferenceRequest, policy: VendorPolicy) -> Vec<u8> {
        crate::request::responses(request, policy, "openai:")
    }

    /// The Chat Completions body of `request` for a vendor of `policy`.
    pub fn chat_completions_body(request: &InferenceRequest, policy: VendorPolicy) -> Vec<u8> {
        crate::request::chat_completions(request, policy)
    }
}
