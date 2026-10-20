//! The audio role of a Gemini API key: speech from the TTS models and music from Lyria, each one
//! `POST …/interactions` with the key in `x-goog-api-key`, whose `model_output` step holds the
//! audio as base64 ([Speech generation](https://ai.google.dev/gemini-api/docs/speech-generation),
//! [Music generation](https://ai.google.dev/gemini-api/docs/music-generation), [Interactions
//! API](https://ai.google.dev/api/interactions-api)). Speech takes its instructions as the turn's
//! `speech_metadata` style and its voice in `generation_config.speech_config`; a block of WAV is
//! written as it came, one of raw PCM (`audio/l16`, as the older TTS models answer) is handed on
//! to be wrapped. The models are those Google lists for the key whose ID names TTS or Lyria; the
//! voices are `GET …/voices`, every page.

use bytes::Bytes;
use demi_provider_common::{
    audio::{AudioFormatAsk, AudioMaker, AudioRequest, MadeAudio, Voice},
    endpoint_url, json_body,
    media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable, within_limit},
    media_catalog::Catalog,
};
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, AudioShape, B64Bytes, TokenUsage};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::images::{GeminiKey, LABEL};

/// The most voices one page of the listing holds, the most Google takes.
const VOICE_PAGE_SIZE: &str = "1000";

/// A Gemini API key's audio role.
pub(crate) struct KeyAudio(pub(crate) GeminiKey);

/// The kinds a listed model makes by the words of its ID: a TTS model's
/// speech, Lyria's music.
fn kinds_of(words: &[String]) -> Vec<AudioKind> {
    if words.iter().any(|word| word == "lyria") {
        vec![AudioKind::Music]
    } else if words.iter().any(|word| word == "tts") {
        vec![AudioKind::Speech]
    } else {
        Vec::new()
    }
}

fn makes_audio(words: &[String], _: &[String]) -> bool {
    !kinds_of(words).is_empty()
}

impl MediaMaker for KeyAudio {
    fn name(&self) -> &str {
        LABEL
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Google)
    }

    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            let models = self.0.list(http, makes_audio).await?;
            Ok(models
                .into_iter()
                .map(|model| MediaModelInfo { kinds: kinds_of(&demi_provider_common::model_words(&model.id)), ..model })
                .collect())
        })
    }
}

impl AudioMaker for KeyAudio {
    fn shape(&self) -> AudioShape {
        AudioShape::Gemini
    }

    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        request: AudioRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(async move {
            let key = &self.0;
            let url = endpoint_url(key.base(), "/interactions");
            let headers = key.headers();
            key.call()
                .post_json(http, &url, Bytes::from(encode(&request)), |_| {
                    std::future::ready(Ok((headers.clone(), None)))
                })
                .await
                .and_then(|body| read_interaction(&body))
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
            let key = &self.0;
            let mut voices = Vec::new();
            let mut page: Option<String> = None;
            loop {
                let mut url = endpoint_url(key.base(), "/voices");
                {
                    let mut query = url.query_pairs_mut();
                    query.append_pair("page_size", VOICE_PAGE_SIZE);
                    if let Some(search) = search {
                        query.append_pair("search", search);
                    }
                    if let Some(token) = &page {
                        query.append_pair("page_token", token);
                    }
                }
                let text = key.call().get(http, &url, key.headers()).await?;
                let listing: VoiceListing = serde_json::from_slice(&text)
                    .map_err(|error| unreadable(format!("its voices could not be read: {error}")))?;
                voices.extend(listing.voices.into_iter().map(ListedVoice::into_voice));
                match listing.next_page_token.filter(|token| !token.is_empty()) {
                    Some(token) => page = Some(token),
                    None => return Ok(Some(voices)),
                }
            }
        })
    }
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    input: Input<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig<'a>>,
}

/// Speech's input is one turn of text; music's the prompt itself.
#[derive(Serialize)]
#[serde(untagged)]
enum Input<'a> {
    Turns([Turn<'a>; 1]),
    Prompt(&'a str),
}

#[derive(Serialize)]
struct Turn<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    content: [Text<'a>; 1],
}

#[derive(Serialize)]
struct Text<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    annotations: Vec<SpeechMetadata<'a>>,
}

#[derive(Serialize)]
struct SpeechMetadata<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    style: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<&'static str>,
}

#[derive(Serialize)]
struct GenerationConfig<'a> {
    speech_config: [SpeechVoice<'a>; 1],
}

#[derive(Serialize)]
struct SpeechVoice<'a> {
    voice: &'a str,
}

/// The interaction `request` asks for: speech with its voice and its
/// instructions as the turn's style, and the vendor's default format, WAV
/// for the current models; music from its prompt, asking WAV only when
/// `--format wav` does, MP3 being Lyria's default.
fn encode(request: &AudioRequest) -> Vec<u8> {
    let body = match request.kind {
        AudioKind::Speech => Body {
            model: &request.model,
            input: Input::Turns([Turn {
                kind: "user_input",
                content: [Text {
                    kind: "text",
                    text: &request.text,
                    annotations: request
                        .instructions
                        .as_deref()
                        .map(|style| SpeechMetadata { kind: "speech_metadata", style })
                        .into_iter()
                        .collect(),
                }],
            }]),
            response_format: Some(ResponseFormat { kind: "audio", mime_type: None }),
            generation_config: request
                .voice
                .as_deref()
                .map(|voice| GenerationConfig { speech_config: [SpeechVoice { voice }] }),
        },
        AudioKind::Music | AudioKind::Effect => Body {
            model: &request.model,
            input: Input::Prompt(&request.text),
            response_format: (request.format == AudioFormatAsk::Wav)
                .then_some(ResponseFormat { kind: "audio", mime_type: Some("audio/wav") }),
            generation_config: None,
        },
    };
    json_body(&body)
}

#[derive(Deserialize)]
struct Interaction {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Step {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    content: Vec<Content>,
}

#[derive(Deserialize)]
struct Content {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: Option<B64Bytes>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    total_input_tokens: u64,
    #[serde(default)]
    total_output_tokens: u64,
}

/// The last audio block of the interaction's `model_output` steps: WAV or
/// MP3 as it came, raw PCM marked for wrapping. One without audio fails
/// with what the model said instead, when it said something.
fn read_interaction(body: &[u8]) -> Result<MadeAudio, MediaFailure> {
    let interaction: Interaction =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    let outputs: Vec<&Content> = interaction
        .steps
        .iter()
        .filter(|step| step.kind == "model_output")
        .flat_map(|step| &step.content)
        .collect();
    let Some(audio) = outputs.iter().rev().find(|content| content.kind == "audio" && content.data.is_some()) else {
        let said: Vec<&str> = outputs.iter().filter_map(|content| content.text.as_deref()).collect();
        return Err(match said.is_empty() {
            true => unreadable("its answer held no audio".to_owned()),
            false => MediaFailure::Refused { message: said.join(" "), code: None, trace: Default::default() },
        });
    };
    let bytes = within_limit(audio.data.clone().expect("an audio block with data").into_bytes(), "audio")?;
    // Raw PCM is wrapped only as its media type states it, checked here.
    let pcm = match audio.mime_type.as_deref().map(Pcm::of_media_type).transpose() {
        Ok(pcm) => pcm.flatten(),
        Err(bad) => return Err(unreadable(bad.to_string())),
    };
    if let Some(pcm) = pcm {
        pcm.check(bytes.len()).map_err(|bad| unreadable(bad.to_string()))?;
    }
    Ok(MadeAudio {
        bytes,
        pcm,
        cost: None,
        usage: interaction.usage.map(|usage| TokenUsage {
            input_tokens: usage.total_input_tokens,
            output_tokens: usage.total_output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }),
        vendor_id: interaction.id,
    })
}

/// `GET …/voices`: the project's own voices, then the prebuilt ones.
#[derive(Deserialize)]
struct VoiceListing {
    #[serde(default)]
    voices: Vec<ListedVoice>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct ListedVoice {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    accent: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// `prebuilt`, `prompted` or `replicated`.
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

impl ListedVoice {
    fn into_voice(self) -> Voice {
        let description = match (self.description, self.accent) {
            (Some(description), Some(accent)) => Some(format!("{description} ({accent})")),
            (description, accent) => description.or(accent),
        };
        Voice {
            id: self.id,
            name: self.display_name,
            languages: self.language_code.into_iter().collect(),
            description,
            own: self.kind.map(|kind| kind != "prebuilt"),
        }
    }
}
