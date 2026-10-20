//! The Google Gemini provider: a `google` entry authenticated by an API key.
//!
//! It speaks Gemini's native `generateContent` API rather than its
//! OpenAI-compatible one, so a video is a real inline part and thought
//! summaries, thought signatures and thinking token counts arrive as fields of
//! their own. It reads its failure records by the standard reading. The key of
//! a Gemini API vendor entry also makes images, videos with Veo, and speech and
//! music.

mod audio;
mod images;
mod request;
mod stream;
mod videos;

use std::sync::Arc;

use demi_provider_common::{
    Capabilities, InferenceRequest, Provider, ProviderRun, ProviderRuntime,
    RequestLimits, ToolResultKinds, RuntimeEnv, RuntimeError, Secret, read_http_failure,
    audio::AudioMaker,
    images::ImageMaker,
    videos::VideoMaker,
};
use demi_shared_types::{
    AuthState, Clock, Model, ProviderErrorDiagnostics, ProviderFailureFacts,
    RuntimeState, Timestamp,
};
use futures_util::{
    StreamExt,
    future::{BoxFuture, LocalBoxFuture},
};
use reqwest::{Url, header::HeaderValue};

/// The prefix this provider puts on the thought signatures it receives: a
/// transcript outlives a provider choice, and each vendor's signature is its
/// own, so a signature without it is never replayed.
const SIGNATURE_TAG: &str = "google:";

/// The configuration of a `google` entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct GoogleConfig {
    /// Sent when set, as every API-key family sends its key.
    pub api_key: Option<Secret>,
    /// The API's base with its version, such as
    /// `https://generativelanguage.googleapis.com/v1beta`; `None` for that
    /// default.
    pub base_url: Option<Url>,
    /// Whether the key makes images: an entry of the Gemini API vendor,
    /// not another vendor that speaks its protocol.
    pub makes_images: bool,
    /// Whether the key makes videos with Veo: an entry of the Gemini API
    /// vendor.
    pub makes_videos: bool,
    /// Whether the key makes speech and music: an entry of the Gemini API
    /// vendor.
    pub makes_audio: bool,
}

impl GoogleConfig {
    /// The base URL of an entry that names none (`crates/provider-common/src/endpoint.rs`).
    pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
}

/// A `google` entry, shared by every user and request of the entry.
pub struct GoogleProvider {
    shared: Arc<Shared>,
    /// The key's image role, for a Gemini API entry with its key.
    images: Option<images::KeyImages>,
    /// The key's video role, for a Gemini API entry with its key.
    videos: Option<videos::KeyVideos>,
    /// The key's audio role, for a Gemini API entry with its key.
    audio: Option<audio::KeyAudio>,
}

/// What the provider and all its runtimes share.
struct Shared {
    api_key: Option<HeaderValue>,
    base_url: Url,
    clock: Arc<dyn Clock>,
}

impl Shared {
    /// `…/models/<model>:streamGenerateContent?alt=sse` under the base.
    fn stream_url(&self, model_id: &str) -> Url {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            // An http or https URL always has a path to extend.
            .expect("an API base URL has path segments")
            .pop_if_empty()
            .push("models")
            .push(&format!("{model_id}:streamGenerateContent"));
        url.set_query(Some("alt=sse"));
        url
    }
}

impl GoogleProvider {
    pub fn new(config: GoogleConfig, clock: Arc<dyn Clock>) -> Self {
        let base_url = config.base_url.unwrap_or_else(|| {
            Url::parse(GoogleConfig::DEFAULT_BASE_URL).expect("the default base URL parses")
        });
        let key = config.api_key.clone().map(|key| images::GeminiKey::new(key, base_url.clone(), clock.clone()));
        let images = key.clone().filter(|_| config.makes_images).map(images::KeyImages);
        let videos = key.clone().filter(|_| config.makes_videos).map(videos::KeyVideos);
        let audio = key.filter(|_| config.makes_audio).map(audio::KeyAudio);
        Self {
            shared: Arc::new(Shared {
                api_key: config.api_key.as_ref().map(Secret::header_value),
                base_url,
                clock,
            }),
            images,
            videos,
            audio,
        }
    }
}

impl Provider for GoogleProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// Whether the vendor accepts the entry's key shows on the first
    /// request.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            AuthState::Authenticated {
                account_label: None,
            }
        })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready {
            message: Some("Uses the Gemini generateContent API".into()),
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
        Ok(Box::new(GoogleRuntime {
            shared: self.shared.clone(),
            http: env.http,
        }))
    }
}

/// A session's runtime: it holds nothing between runs.
struct GoogleRuntime {
    shared: Arc<Shared>,
    http: demi_provider_common::ProviderHttp,
}

impl ProviderRuntime for GoogleRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        stream::run(self.shared.clone(), self.http.clone(), request).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(GoogleRuntime {
            shared: self.shared.clone(),
            http: self.http.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// 20 MB, since media travel inline, and 3,600 images.
    fn request_limits(&self, _model: &Model) -> RequestLimits {
        RequestLimits {
            body_bytes: Some(20_000_000),
            images: Some(3_600),
            // `inlineData` parts beside the `functionResponse`
            // (`crates/provider-common/src/contract.rs`).
            tool_results: ToolResultKinds::ALL,
        }
    }
}

/// Test support (feature `testing`): the body this provider sends for a
/// request, for the tests of what sends its requests.
#[cfg(feature = "testing")]
pub mod testing {
    use demi_provider_common::InferenceRequest;

    /// The `streamGenerateContent` body of `request`.
    pub fn body(request: &InferenceRequest) -> Vec<u8> {
        crate::request::encode(request)
    }
}
