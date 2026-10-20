//! The ElevenLabs provider (`crates-and-packages.md` § Vendor provider
//! crates): an ElevenLabs entry of the
//! `media` family, which makes no text, authenticated by its key in
//! `xi-api-key`. Its key makes speech, music and sound effects, each one
//! request answered with the audio's bytes; its voices are its account's
//! list, every page; its models are the speech models it lists for the
//! key and the music and effect models of the catalog, and reading that
//! list is what checks the key.

mod audio;

use std::sync::Arc;

use demi_provider_common::{
    Capabilities, Provider, ProviderRuntime, RuntimeEnv, RuntimeError, Secret, audio::AudioMaker, read_http_failure,
};
use demi_shared_types::{AuthState, Clock, ProviderErrorDiagnostics, ProviderFailureFacts, RuntimeState, Timestamp};
use futures_util::future::BoxFuture;
use reqwest::Url;

/// The configuration of an ElevenLabs entry, as the backend decoded it.
#[derive(Debug, Clone)]
pub struct ElevenLabsConfig {
    pub api_key: Option<Secret>,
    /// The API's base, such as `https://api.elevenlabs.io`; `None` for
    /// that default.
    pub base_url: Option<Url>,
}

impl ElevenLabsConfig {
    /// The base URL of an entry that names none, the one the media catalog
    /// names for the vendor.
    pub const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
}

/// An ElevenLabs entry, shared by every user and request of the entry.
pub struct ElevenLabsProvider {
    /// The key's audio role, for an entry with its key.
    audio: Option<audio::KeyAudio>,
}

impl ElevenLabsProvider {
    pub fn new(config: ElevenLabsConfig, clock: Arc<dyn Clock>) -> Self {
        let base = config
            .base_url
            .unwrap_or_else(|| Url::parse(ElevenLabsConfig::DEFAULT_BASE_URL).expect("the default base URL parses"));
        Self { audio: config.api_key.map(|key| audio::KeyAudio::new(key, base, clock)) }
    }
}

impl Provider for ElevenLabsProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    /// Whether ElevenLabs accepts the key shows when its models are read,
    /// as Test Connection does.
    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async { AuthState::Authenticated { account_label: None } })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready { message: Some("Makes speech, music and sound effects".into()) }
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
        Err(RuntimeError::NoText { provider: "ElevenLabs".to_owned() })
    }
}
