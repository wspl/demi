//! An ElevenLabs key's audio role (`audio.md` § Vendors): speech from
//! `POST /v1/text-to-speech/<voice>`, music from `POST /v1/music` and sound
//! effects from `POST /v1/sound-generation`, each answered with the audio's
//! bytes in the `output_format` its query names
//! ([Create speech](https://elevenlabs.io/docs/api-reference/text-to-speech/convert),
//! [Compose music](https://elevenlabs.io/docs/api-reference/music/compose),
//! [Create sound effect](https://elevenlabs.io/docs/api-reference/text-to-sound-effects/convert)).
//! Speech needs a voice, as ElevenLabs has no default one; music never
//! sends a seed, which ElevenLabs takes only with a composition plan; WAV
//! of music or an effect, which ElevenLabs does not make, is asked as
//! 44.1 kHz PCM and handed on to be wrapped. The voices are
//! `GET /v2/voices`, every page
//! ([List voices](https://elevenlabs.io/docs/api-reference/voices/search)),
//! and the speech models `GET /v1/models` with `can_do_text_to_speech`
//! ([List models](https://elevenlabs.io/docs/api-reference/models/list)).

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    ErrorCode, ProviderFailure, Secret,
    audio::{AudioFormatAsk, AudioMaker, AudioRequest, MadeAudio, Voice},
    endpoint_url, json_body,
    media::{MediaAnswer, MediaCall, MediaCredential, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable},
    media_catalog::{AudioOutput, Catalog, MediaKind},
    read_http_failure,
};
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, AudioShape, Clock};
use futures_util::future::BoxFuture;
use reqwest::{
    StatusCode, Url,
    header::{HeaderMap, HeaderName},
};
use serde::{Deserialize, Serialize};

/// How failure messages name the vendor.
const LABEL: &str = "ElevenLabs";

/// The most voices one page of the listing holds, the most ElevenLabs
/// takes.
const VOICE_PAGE_SIZE: &str = "100";

/// The PCM a WAV of music or an effect is asked as, `pcm_44100`:
/// 16-bit mono at 44.1 kHz, as ElevenLabs documents it.
fn wav_pcm() -> Pcm {
    Pcm::new(44_100, 1, 16).expect("44.1 kHz 16-bit mono is PCM a WAV file holds")
}

/// An ElevenLabs key's audio role.
pub(crate) struct KeyAudio {
    key: Secret,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl KeyAudio {
    pub(crate) fn new(key: Secret, base: Url, clock: Arc<dyn Clock>) -> Self {
        Self { key, base, clock }
    }

    fn call(&self) -> MediaCall<'_> {
        MediaCall {
            label: LABEL,
            credential: MediaCredential::Key { refused: refused_key },
            reader: read_http_failure,
            used_up,
            clock: &*self.clock,
            quota: None,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("xi-api-key"), self.key.header_value());
        headers
    }

    /// The speech models ElevenLabs lists for the key, then the catalog's
    /// music and effect models, which no listing names.
    async fn list(&self, http: &demi_provider_common::ProviderHttp, catalog: &Catalog) -> Result<Vec<MediaModelInfo>, MediaFailure> {
        let url = endpoint_url(&self.base, "/v1/models");
        let text = self.call().get(http, &url, self.headers()).await?;
        let listed: Vec<ListedModel> = serde_json::from_slice(&text)
            .map_err(|error| unreadable(format!("its list of models could not be read: {error}")))?;
        let mut models: Vec<MediaModelInfo> = listed
            .into_iter()
            .filter(|model| model.can_do_text_to_speech)
            .map(|model| MediaModelInfo {
                name: model.name,
                description: model.description,
                kinds: vec![AudioKind::Speech],
                ..MediaModelInfo::id(&model.model_id)
            })
            .collect();
        for known in catalog.models.iter().filter(|model| model.vendor == MediaVendor::Elevenlabs.id() && model.kind == MediaKind::Audio) {
            let makes = catalog.audio(known).and_then(|audio| audio.makes.as_ref());
            let kinds: Vec<AudioKind> = makes
                .into_iter()
                .flatten()
                .filter_map(|output| match output {
                    AudioOutput::Music => Some(AudioKind::Music),
                    AudioOutput::Effect => Some(AudioKind::Effect),
                    AudioOutput::Speech | AudioOutput::Other => None,
                })
                .collect();
            if !kinds.is_empty() && !models.iter().any(|model| model.id == known.id) {
                models.push(MediaModelInfo { kinds, ..MediaModelInfo::id(&known.id) });
            }
        }
        Ok(models)
    }
}

/// ElevenLabs refuses a key it does not know with HTTP 401
/// (`invalid_api_key`); a 401 for a spent quota is a usage limit, read
/// first.
fn refused_key(status: StatusCode, _: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
}

/// A failure whose `detail.status` is `quota_exceeded`: the account's
/// characters or credits are spent.
fn used_up(mut failure: ProviderFailure) -> ProviderFailure {
    #[derive(Deserialize)]
    struct Refusal {
        detail: Detail,
    }
    #[derive(Deserialize)]
    struct Detail {
        status: String,
    }
    let refusal = failure
        .diagnostics
        .as_deref()
        .and_then(demi_provider_common::HttpFailureRecord::read)
        .and_then(|record| serde_json::from_str::<Refusal>(&record.body).ok());
    if refusal.is_some_and(|refusal| refusal.detail.status == "quota_exceeded") {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}

impl MediaMaker for KeyAudio {
    fn name(&self) -> &str {
        LABEL
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Elevenlabs)
    }

    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(self.list(http, catalog))
    }
}

impl AudioMaker for KeyAudio {
    fn shape(&self) -> AudioShape {
        AudioShape::Elevenlabs
    }

    fn needs_voice(&self) -> bool {
        true
    }

    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        request: AudioRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(async move {
            let (path, body) = match request.kind {
                AudioKind::Speech => {
                    let voice = request.voice.as_deref().unwrap_or_default();
                    let body = json_body(&SpeechBody {
                        text: &request.text,
                        model_id: &request.model,
                        voice_settings: request.speed.map(|speed| VoiceSettings { speed }),
                    });
                    (format!("/v1/text-to-speech/{voice}"), body)
                }
                AudioKind::Music => {
                    let body = json_body(&MusicBody {
                        prompt: &request.text,
                        music_length_ms: request.duration.map(|seconds| (seconds * 1000.0).round() as u64),
                        model_id: &request.model,
                        force_instrumental: request.instrumental.then_some(true),
                    });
                    ("/v1/music".to_owned(), body)
                }
                AudioKind::Effect => {
                    let body = json_body(&EffectBody {
                        text: &request.text,
                        duration_seconds: request.duration,
                        r#loop: request.looped.then_some(true),
                        model_id: &request.model,
                    });
                    ("/v1/sound-generation".to_owned(), body)
                }
            };
            let (output_format, pcm) = output_format(request.kind, request.format);
            let mut url = endpoint_url(&self.base, &path);
            if let Some(output_format) = output_format {
                url.query_pairs_mut().append_pair("output_format", output_format);
            }
            let headers = self.headers();
            self.call()
                .post_json_for_file(http, &url, Bytes::from(body), |_| std::future::ready(Ok((headers.clone(), None))))
                .await
                .and_then(|bytes| {
                    // PCM that ends inside a sample is not the PCM asked for.
                    if let Some(pcm) = pcm {
                        pcm.check(bytes.len()).map_err(|bad| unreadable(bad.to_string()))?;
                    }
                    Ok(MadeAudio { bytes, pcm, cost: None, usage: None, vendor_id: None })
                })
        })
    }

    fn voices<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        _model: &'a str,
        search: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Vec<Voice>>, MediaFailure>> {
        Box::pin(async move {
            let mut voices = Vec::new();
            let mut page: Option<String> = None;
            loop {
                let mut url = endpoint_url(&self.base, "/v2/voices");
                {
                    let mut query = url.query_pairs_mut();
                    query.append_pair("page_size", VOICE_PAGE_SIZE);
                    query.append_pair("include_total_count", "false");
                    if let Some(search) = search {
                        query.append_pair("search", search);
                    }
                    if let Some(token) = &page {
                        query.append_pair("next_page_token", token);
                    }
                }
                let text = self.call().get(http, &url, self.headers()).await?;
                let listing: VoiceListing = serde_json::from_slice(&text)
                    .map_err(|error| unreadable(format!("its voices could not be read: {error}")))?;
                voices.extend(listing.voices.into_iter().map(ListedVoice::into_voice));
                match listing.next_page_token.filter(|token| listing.has_more && !token.is_empty()) {
                    Some(token) => page = Some(token),
                    None => return Ok(Some(voices)),
                }
            }
        })
    }
}

/// The `output_format` a call of `kind` asks for `format`, and the PCM it
/// answers for one ElevenLabs makes no file of: speech has MP3, WAV and
/// Opus; music and effects MP3 and Opus, and PCM for a WAV. None asks
/// ElevenLabs' default, MP3; FLAC, which it does not make, comes as MP3.
fn output_format(kind: AudioKind, format: AudioFormatAsk) -> (Option<&'static str>, Option<Pcm>) {
    match (kind, format) {
        (_, AudioFormatAsk::Mp3 | AudioFormatAsk::Flac) => (None, None),
        (_, AudioFormatAsk::Opus) => (Some("opus_48000_128"), None),
        (AudioKind::Speech, AudioFormatAsk::Wav) => (Some("wav_44100"), None),
        (AudioKind::Music | AudioKind::Effect, AudioFormatAsk::Wav) => (Some("pcm_44100"), Some(wav_pcm())),
    }
}

#[derive(Serialize)]
struct SpeechBody<'a> {
    text: &'a str,
    model_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_settings: Option<VoiceSettings>,
}

#[derive(Serialize)]
struct VoiceSettings {
    speed: f64,
}

#[derive(Serialize)]
struct MusicBody<'a> {
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    music_length_ms: Option<u64>,
    model_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    force_instrumental: Option<bool>,
}

#[derive(Serialize)]
struct EffectBody<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r#loop: Option<bool>,
    model_id: &'a str,
}

#[derive(Deserialize)]
struct ListedModel {
    model_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    can_do_text_to_speech: bool,
}

#[derive(Deserialize)]
struct VoiceListing {
    voices: Vec<ListedVoice>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct ListedVoice {
    voice_id: String,
    #[serde(default)]
    name: Option<String>,
    /// `premade` for ElevenLabs' own; `cloned`, `generated` or
    /// `professional` for the account's.
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    verified_languages: Option<Vec<VerifiedLanguage>>,
}

#[derive(Deserialize)]
struct VerifiedLanguage {
    language: String,
}

impl ListedVoice {
    fn into_voice(self) -> Voice {
        let mut languages: Vec<String> = Vec::new();
        for language in self.verified_languages.into_iter().flatten().map(|verified| verified.language) {
            if !languages.contains(&language) {
                languages.push(language);
            }
        }
        if languages.is_empty() {
            languages.extend(self.labels.get("language").cloned());
        }
        Voice {
            id: self.voice_id,
            name: self.name,
            languages,
            description: self.description.filter(|description| !description.trim().is_empty()),
            own: self.category.map(|category| category != "premade"),
        }
    }
}
