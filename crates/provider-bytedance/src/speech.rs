//! A ByteDance speech key's audio role, BytePlus Speech's and Doubao Speech's
//! alike, whose request is the model's.
//!
//! # Seed speech
//!
//! Seed speech's models speak through `POST /api/v3/tts/unidirectional`, with
//! the key in `X-Api-Key` and the model as `X-Api-Resource-Id`. The service
//! answers with a chunked series of JSON objects, each with its `code` and the
//! next base64 piece of the audio in `data`, until the object whose `code` is
//! `20000000`
//! ([BytePlus](https://docs.byteplus.com/en/docs/byteplusvoice/unidirectional_tts_http),
//! [Doubao
//! Speech](https://docs.volcengine.com/docs/DoubaoVoice/HTTPChunkedSSEUnidirectionalStreaming-V3?lang=zh)).
//!
//! BytePlus also requires its fixed `X-Api-App-Key`. A WAV is asked as 24 kHz
//! PCM and handed on to be wrapped, as the services repeat a WAV's header in
//! every piece.
//!
//! # Seed Audio
//!
//! Seed Audio's model, which the catalog says makes sound effects, makes them
//! and speaks through `POST /api/v3/tts/create`. The request holds:
//!
//! - `model`;
//! - `text_prompt`, the instructions before the text;
//! - the speaker for speech, as its one `references` item;
//! - `audio_config`.
//!
//! The service answers with the file in base64 `audio` ([Audio
//! 1.0](https://docs.byteplus.com/en/docs/byteplusvoice/seedaudio-01),
//! [音频生成](https://docs.volcengine.com/docs/DoubaoVoice/audio-generation-http?lang=zh)).
//!
//! # Common rules
//!
//! - Neither service has a default speaker, so speech needs `--voice`.
//! - The speakers are the catalog's, as the services list them only to a
//!   request signed with an access key pair. Seed Audio's are those of Seed
//!   speech 2.0. So are the models.
//! - A failure is traced by its code and the request's `X-Tt-Logid`.
//! - A key is checked by a Seed Audio request with no prompt, which the service
//!   refuses for its body. Neither service documents how it refuses a key,
//!   which is read as HTTP 401 or 403.

use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use demi_provider_common::{
    ErrorCode, Secret,
    audio::{AudioFormatAsk, AudioMaker, AudioOption, AudioRequest, MadeAudio, Voice, searched, unsent_of},
    endpoint_url, json_body,
    media::{Answered, Holding, MediaAnswer, MediaCall, MediaCredential, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable, within_limit},
    media_catalog::{AudioOutput, Catalog, MediaKind, Voices},
    read_http_failure,
};
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, AudioShape, B64Bytes, Clock, VendorTrace};
use futures_util::future::BoxFuture;
use reqwest::{
    StatusCode, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};

use crate::Speech;
use crate::images::checked;

/// How failure records name the services.
const LABEL: &str = "ByteDance Speech";

/// The `code` of the object that ends a speech.
const FINISHED: i64 = 20_000_000;

/// The `code` of an object that carries a piece of the speech.
const PIECE: i64 = 0;

/// The `code` of a failure on the service's side.
const SERVER_ERROR: i64 = 55_000_000;

/// The model Seed Audio's requests name, the one it documents.
const SEED_AUDIO: &str = "seed-audio-1.0";

/// The Seed speech model whose speakers Seed Audio takes, as its reference
/// names "豆包语音合成模型2.0"'s.
const SEED_SPEECH_2: &str = "seed-tts-2.0";

/// The fixed `X-Api-App-Key` BytePlus Speech requires.
const BYTEPLUS_APP_KEY: &str = "aGjiRDfUWi";

/// The rate speech is asked at, and its WAV's PCM: 24 kHz mono 16-bit.
const SAMPLE_RATE: u32 = 24_000;

fn speech_pcm() -> Pcm {
    Pcm::new(SAMPLE_RATE, 1, 16).expect("24 kHz 16-bit mono is PCM a WAV file holds")
}

/// A speech key's audio role.
pub(crate) struct SeedAudio {
    speech: Speech,
    key: Secret,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl SeedAudio {
    pub(crate) fn new(speech: Speech, key: Secret, base: Url, clock: Arc<dyn Clock>) -> Self {
        Self { speech, key, base, clock }
    }

    pub(crate) fn label(&self) -> &'static str {
        self.speech.vendor().name()
    }

    fn call(&self) -> MediaCall<'_> {
        MediaCall {
            label: LABEL,
            credential: MediaCredential::Key { refused: refused_key },
            reader: read_http_failure,
            used_up: |failure| failure,
            clock: &*self.clock,
            quota: None,
        }
    }

    /// The key's headers, with the model as the resource for speech.
    fn headers(&self, resource: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("x-api-key"), self.key.header_value());
        if let Some(resource) = resource {
            // A model the catalog or the agent names that is no header's
            // text is sent without one, and the service refuses it.
            if let Ok(value) = HeaderValue::from_str(resource) {
                headers.insert(HeaderName::from_static("x-api-resource-id"), value);
            }
            if self.speech == Speech::Byteplus {
                headers.insert(HeaderName::from_static("x-api-app-key"), HeaderValue::from_static(BYTEPLUS_APP_KEY));
            }
        }
        headers
    }

    /// Posts `body` to `path`, answering the body of the service's success
    /// and the `X-Tt-Logid` that names the request, which a failure its
    /// body states is traced by.
    async fn post(&self, http: &demi_provider_common::ProviderHttp, path: &str, body: Vec<u8>, headers: HeaderMap) -> MediaAnswer<Answered> {
        let url = endpoint_url(&self.base, path);
        self.call()
            .send(http, demi_provider_common::media::Ask::post_json(&url, Bytes::from(body)), Holding::Json, |_| {
                std::future::ready(Ok((headers.clone(), None)))
            })
            .await
    }

    /// The catalog's audio models of the service, with the kinds the
    /// catalog says each makes.
    fn listed(&self, catalog: &Catalog) -> Vec<MediaModelInfo> {
        catalog_models(catalog, self.speech.vendor(), MediaKind::Audio)
            .into_iter()
            .map(|listed| {
                let makes = self.makes(catalog, &listed.id);
                let kinds = [(AudioOutput::Speech, AudioKind::Speech), (AudioOutput::Effect, AudioKind::Effect)]
                    .into_iter()
                    .filter(|(output, _)| makes.contains(output))
                    .map(|(_, kind)| kind)
                    .collect();
                MediaModelInfo { kinds, ..listed }
            })
            .collect()
    }

    /// What the catalog says `model` of the service makes.
    fn makes(&self, catalog: &Catalog, model: &str) -> Vec<AudioOutput> {
        catalog
            .model(self.speech.vendor().id(), model)
            .and_then(|known| catalog.audio(known))
            .and_then(|audio| audio.makes.clone())
            .unwrap_or_default()
    }

    /// Whether `request` is Seed Audio's: an effect, or speech of a model
    /// the catalog says makes sound effects, rather than Seed speech's.
    fn seed_audio(&self, catalog: &Catalog, request: &AudioRequest) -> bool {
        request.kind != AudioKind::Speech || self.makes(catalog, &request.model).contains(&AudioOutput::Effect)
    }

    async fn speak(&self, http: &demi_provider_common::ProviderHttp, request: AudioRequest) -> MediaAnswer<MadeAudio> {
        let (format, pcm) = match request.format {
            AudioFormatAsk::Mp3 | AudioFormatAsk::Flac => ("mp3", None),
            AudioFormatAsk::Opus => ("ogg_opus", None),
            AudioFormatAsk::Wav => ("pcm", Some(speech_pcm())),
        };
        let body = json_body(&SpeechBody {
            req_params: SpeechParams {
                text: &request.text,
                speaker: request.voice.as_deref().unwrap_or_default(),
                audio_params: AudioParams { format, sample_rate: SAMPLE_RATE, speech_rate: request.speed.map(speech_rate) },
            },
        });
        let headers = self.headers(Some(&request.model));
        self.post(http, "/api/v3/tts/unidirectional", body, headers).await.and_then(|answered| {
            let bytes = join_pieces(&answered.body, answered.trace_id)?;
            // PCM that ends inside a sample is not the PCM asked for.
            if let Some(pcm) = pcm {
                pcm.check(bytes.len()).map_err(|bad| unreadable(bad.to_string()))?;
            }
            Ok(MadeAudio { bytes, pcm, cost: None, usage: None, vendor_id: None })
        })
    }

    /// Seed Audio's speech or effect: the text to speak, or the effect to
    /// make, after the instructions, with the speaker for speech.
    async fn create(&self, http: &demi_provider_common::ProviderHttp, request: AudioRequest) -> MediaAnswer<MadeAudio> {
        let format = match request.format {
            AudioFormatAsk::Mp3 | AudioFormatAsk::Flac => "mp3",
            AudioFormatAsk::Opus => "ogg_opus",
            AudioFormatAsk::Wav => "wav",
        };
        let speaks = request.kind == AudioKind::Speech;
        let text_prompt = match request.instructions.as_deref().filter(|_| speaks) {
            Some(instructions) => format!("{instructions}\n{}", request.text),
            None => request.text.clone(),
        };
        let body = json_body(&CreateBody {
            model: &request.model,
            text_prompt: &text_prompt,
            references: request.voice.as_deref().filter(|_| speaks).map(|speaker| Reference { speaker }).into_iter().collect(),
            audio_config: AudioConfig { format, speech_rate: request.speed.filter(|_| speaks).map(speech_rate) },
        });
        self.post(http, "/api/v3/tts/create", body, self.headers(None)).await.and_then(|answered| {
            let bytes = read_created(&answered.body, answered.trace_id)?;
            Ok(MadeAudio { bytes, pcm: None, cost: None, usage: None, vendor_id: None })
        })
    }
}

/// Neither service documents how it refuses a key; HTTP 401 and 403 are
/// read as a refused key.
fn refused_key(status: StatusCode, _: &str) -> bool {
    status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN
}

/// The `speech_rate` of a factor of the voice's speed: the services count
/// 0.5× to 2× from −50 to 100, 1× at 0.
fn speech_rate(speed: f64) -> i32 {
    ((speed - 1.0) * 100.0).round().clamp(-50.0, 100.0) as i32
}

/// One object of a speech's answer.
#[derive(Deserialize)]
struct Piece {
    code: i64,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: Option<B64Bytes>,
}

/// The `code` of a request the service refuses for its body, for its size,
/// or for the account's concurrency, which only its message tells apart.
const REQUEST_ERROR: i64 = 45_000_000;

/// The failure a service's `code` and `message` state, traced by the code and
/// the request's `X-Tt-Logid` ([Error
/// codes](https://docs.volcengine.com/docs/DoubaoVoice/error-code-query-1?lang=zh)).
///
/// A request over the account's concurrency is a rate limit, and a server's
/// failure is one, unless it is a speaker the resource does not hold, which is
/// refused as any other code is.
fn speech_failure(code: i64, message: Option<String>, trace_id: Option<String>) -> MediaFailure {
    let message = message.filter(|message| !message.is_empty()).unwrap_or_else(|| "the speech failed".to_owned());
    let trace = VendorTrace { code: Some(code.to_string()), id: trace_id };
    let busy = code == REQUEST_ERROR && message.contains("quota exceeded");
    let server = code == SERVER_ERROR && !message.contains("mismatched with speaker");
    if busy {
        MediaFailure::Unanswered { cause: message, code: Some(ErrorCode::RateLimit), trace }
    } else if server {
        MediaFailure::Unanswered { cause: message, code: Some(ErrorCode::Overloaded), trace }
    } else {
        MediaFailure::Refused { message, code: None, trace }
    }
}

/// The speech in the series of objects `body` holds: each piece's `data`
/// joined in order, up to the object that ends it; an object of another
/// code is the service's failure, traced by `trace_id`.
fn join_pieces(body: &[u8], trace_id: Option<String>) -> Result<Bytes, MediaFailure> {
    let mut audio = BytesMut::new();
    for piece in serde_json::Deserializer::from_slice(body).into_iter::<Piece>() {
        let piece = piece.map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
        match piece.code {
            FINISHED => return within_limit(audio.freeze(), "audio"),
            PIECE => {
                if let Some(data) = piece.data {
                    audio.extend_from_slice(data.as_bytes());
                }
            }
            code => return Err(speech_failure(code, piece.message, trace_id)),
        }
    }
    Err(unreadable("its answer ended before the speech did".to_owned()))
}

/// The file of Seed Audio's answer: its base64 `audio`, or the failure its
/// `code` and `message` say, traced by `trace_id`.
fn read_created(body: &[u8], trace_id: Option<String>) -> Result<Bytes, MediaFailure> {
    #[derive(Deserialize)]
    struct Created {
        #[serde(default)]
        code: Option<i64>,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        audio: Option<B64Bytes>,
    }
    let created: Created =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    match (created.audio.filter(|audio| !audio.as_bytes().is_empty()), created.code) {
        (Some(audio), _) => within_limit(audio.into_bytes(), "audio"),
        (None, Some(code)) => Err(speech_failure(code, created.message, trace_id)),
        (None, None) => Err(unreadable("its answer held no audio".to_owned())),
    }
}

impl MediaMaker for SeedAudio {
    fn name(&self) -> &str {
        self.label()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(self.speech.vendor())
    }

    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(std::future::ready(Ok(self.listed(catalog))))
    }

    fn check_key<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            let body = json_body(&CreateBody {
                model: SEED_AUDIO,
                text_prompt: "",
                references: Vec::new(),
                audio_config: AudioConfig { format: "mp3", speech_rate: None },
            });
            let probe = self.post(http, "/api/v3/tts/create", body, self.headers(None)).await.result.map(|answered| answered.body);
            checked(probe, self.listed(catalog))
        })
    }
}

impl AudioMaker for SeedAudio {
    fn shape(&self) -> AudioShape {
        AudioShape::Seed
    }

    fn needs_voice(&self) -> bool {
        true
    }

    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        request: AudioRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(async move {
            if self.seed_audio(catalog, &request) {
                self.create(http, request).await
            } else {
                self.speak(http, request).await
            }
        })
    }

    /// Seed Audio's speech takes the instructions Seed speech's does not.
    fn unsent(&self, catalog: &Catalog, request: &AudioRequest) -> Vec<AudioOption> {
        if request.kind == AudioKind::Speech && self.seed_audio(catalog, request) {
            return unsent_of(&[AudioOption::Seed], request);
        }
        demi_provider_common::audio::unsent(self.shape(), request)
    }

    /// Both services count 0.5× to 2×, from −50 to 100.
    fn speeds(&self) -> Option<(f64, f64)> {
        Some((0.5, 2.0))
    }

    fn shape_name(&self, catalog: &Catalog, request: &AudioRequest) -> &'static str {
        if self.seed_audio(catalog, request) { "Seed Audio" } else { "Seed speech" }
    }

    /// The speakers the catalog names for `model`; Seed Audio's are Seed
    /// speech 2.0's.
    fn voices<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        model: &'a str,
        search: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Vec<Voice>>, MediaFailure>> {
        let speaks_as = if self.makes(catalog, model).contains(&AudioOutput::Effect) { SEED_SPEECH_2 } else { model };
        let named = catalog
            .model(self.speech.vendor().id(), speaks_as)
            .and_then(|known| catalog.audio(known))
            .and_then(|audio| audio.voices.as_ref());
        let speakers = match named {
            Some(Voices::Named(names)) => names.iter().map(|name| Voice::named(name)).collect(),
            _ => Vec::new(),
        };
        Box::pin(std::future::ready(Ok(Some(searched(speakers, search)))))
    }
}

#[derive(Serialize)]
struct SpeechBody<'a> {
    req_params: SpeechParams<'a>,
}

#[derive(Serialize)]
struct SpeechParams<'a> {
    text: &'a str,
    speaker: &'a str,
    audio_params: AudioParams,
}

#[derive(Serialize)]
struct AudioParams {
    format: &'static str,
    sample_rate: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    speech_rate: Option<i32>,
}

#[derive(Serialize)]
struct CreateBody<'a> {
    model: &'a str,
    text_prompt: &'a str,
    /// The speaker of speech, the one reference it sends.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    references: Vec<Reference<'a>>,
    audio_config: AudioConfig,
}

#[derive(Serialize)]
struct Reference<'a> {
    speaker: &'a str,
}

#[derive(Serialize)]
struct AudioConfig {
    format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    speech_rate: Option<i32>,
}
