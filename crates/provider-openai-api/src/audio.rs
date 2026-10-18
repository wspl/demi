//! The audio role of an OpenAI key and of a compatible entry whose audio
//! shape is OpenAI Speech (`audio.md` § Vendors): `POST …/audio/speech`
//! under the entry's base URL with the key as a bearer token, answered
//! with the audio's bytes
//! ([Create speech](https://developers.openai.com/api/reference/resources/audio/subresources/speech/methods/create)).
//! An OpenAI key lists the speech models OpenAI lists for it that the
//! catalog knows, and its voices are the catalog's built-ins, as OpenAI's
//! API lists none; a custom voice, `voice_…`, is sent as `{ "id": … }`. A
//! compatible entry offers the speech models its user added, and its voices
//! are what its `GET …/audio/voices` lists, as vLLM-Omni serves it, or none.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret,
    audio::{AudioMaker, AudioRequest, MadeAudio, Voice, searched},
    endpoint_url, json_body,
    media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable},
    media_catalog::{Catalog, MediaKind, Voices},
};
use demi_shared_types::{AudioKind, AudioShape, Clock};
use futures_util::future::BoxFuture;
use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderMap},
};
use serde::{Deserialize, Serialize};

use crate::images::{key_call, openai_models};

/// Whose speech API an entry's requests go to.
enum Flavor {
    OpenAi,
    /// A compatible endpoint, which the user knows by the entry's name and
    /// whose models are the ones the user added.
    Compatible { name: String, models: Vec<MediaModelInfo> },
}

/// The audio role of an OpenAI key or a compatible endpoint.
pub(crate) struct KeyAudio {
    flavor: Flavor,
    /// None for a compatible endpoint that takes none.
    key: Option<Secret>,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl KeyAudio {
    /// The role of an OpenAI `key` under the API `base`.
    pub(crate) fn openai(key: Secret, base: &Url, clock: Arc<dyn Clock>) -> Self {
        Self { flavor: Flavor::OpenAi, key: Some(key), base: base.clone(), clock }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(key) = &self.key {
            headers.insert(AUTHORIZATION, key.bearer());
        }
        headers
    }

    async fn list(&self, http: &demi_provider_common::ProviderHttp, catalog: &Catalog) -> Result<Vec<MediaModelInfo>, MediaFailure> {
        if let Flavor::Compatible { models, .. } = &self.flavor {
            return Ok(models.clone());
        }
        let url = endpoint_url(&self.base, "/models");
        let listed = openai_models(&key_call(false, &*self.clock), http, &url, self.headers()).await?;
        // OpenAI's listing says nothing of what a model makes; the speech
        // models are the ones the catalog knows as OpenAI's audio models.
        Ok(listed
            .into_iter()
            .filter(|model| {
                catalog
                    .model(MediaVendor::OpenAi.id(), &model.id)
                    .is_some_and(|known| known.kind == MediaKind::Audio)
            })
            .map(|model| MediaModelInfo { kinds: vec![AudioKind::Speech], ..model })
            .collect())
    }

    /// What a compatible endpoint's `GET …/audio/voices` lists: none for
    /// a server without the route.
    async fn listed_voices(&self, http: &demi_provider_common::ProviderHttp) -> Result<Option<Vec<Voice>>, MediaFailure> {
        let url = endpoint_url(&self.base, "/audio/voices");
        let Some(text) = key_call(false, &*self.clock).get_if_served(http, &url, self.headers(), true).await? else {
            return Ok(None);
        };
        let listing: VoiceListing = serde_json::from_slice(&text)
            .map_err(|error| unreadable(format!("its voices could not be read: {error}")))?;
        Ok(Some(listing.into_voices()))
    }
}

impl MediaMaker for KeyAudio {
    fn name(&self) -> &str {
        match &self.flavor {
            Flavor::OpenAi => MediaVendor::OpenAi.name(),
            Flavor::Compatible { name, .. } => name,
        }
    }

    fn vendor(&self) -> Option<MediaVendor> {
        match self.flavor {
            Flavor::OpenAi => Some(MediaVendor::OpenAi),
            Flavor::Compatible { .. } => None,
        }
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
        AudioShape::OpenaiSpeech
    }

    /// OpenAI's speech requires `voice` and has no default one; a
    /// compatible server, such as vLLM-Omni's, speaks in its model's own.
    fn needs_voice(&self) -> bool {
        matches!(self.flavor, Flavor::OpenAi)
    }

    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        request: AudioRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(async move {
            let url = endpoint_url(&self.base, "/audio/speech");
            let headers = self.headers();
            key_call(false, &*self.clock)
                .post_json_for_file(http, &url, Bytes::from(speech_body(&request)), |_| {
                    std::future::ready(Ok((headers.clone(), None)))
                })
                .await
                .and_then(|bytes| Ok(MadeAudio { bytes, pcm: None, cost: None, usage: None, vendor_id: None }))
        })
    }

    fn voices<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        model: &'a str,
        search: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Vec<Voice>>, MediaFailure>> {
        Box::pin(async move {
            let voices = match self.flavor {
                Flavor::OpenAi => {
                    let named = catalog
                        .model(MediaVendor::OpenAi.id(), model)
                        .and_then(|known| catalog.audio(known))
                        .and_then(|audio| audio.voices.as_ref());
                    let builtins = match named {
                        Some(Voices::Named(names)) => names.iter().map(|name| Voice::named(name)).collect(),
                        _ => Vec::new(),
                    };
                    Some(builtins)
                }
                Flavor::Compatible { .. } => self.listed_voices(http).await?,
            };
            Ok(voices.map(|voices| searched(voices, search)))
        })
    }
}

/// The audio role of a compatible entry whose audio shape is OpenAI
/// Speech: OpenAI's speech request under its `base`, with its `key` when it
/// has one, offering the `models` its user added, each making speech;
/// `name` is the entry's, which its failures name.
pub fn compatible_audio(
    name: String,
    key: Option<Secret>,
    base: &Url,
    models: Vec<MediaModelInfo>,
    clock: Arc<dyn Clock>,
) -> Arc<dyn AudioMaker> {
    let models = models
        .into_iter()
        .map(|model| MediaModelInfo { kinds: vec![AudioKind::Speech], ..model })
        .collect();
    Arc::new(KeyAudio { flavor: Flavor::Compatible { name, models }, key, base: base.clone(), clock })
}

#[derive(Serialize)]
struct SpeechBody<'a> {
    model: &'a str,
    input: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<SpeechVoice<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
    response_format: &'static str,
}

/// A built-in voice by its name; a custom one, `voice_…`, as an object.
#[derive(Serialize)]
#[serde(untagged)]
enum SpeechVoice<'a> {
    Named(&'a str),
    Custom { id: &'a str },
}

fn speech_body(request: &AudioRequest) -> Vec<u8> {
    let voice = request.voice.as_deref().map(|voice| {
        if voice.starts_with("voice_") { SpeechVoice::Custom { id: voice } } else { SpeechVoice::Named(voice) }
    });
    json_body(&SpeechBody {
        model: &request.model,
        input: &request.text,
        voice,
        instructions: request.instructions.as_deref(),
        speed: request.speed,
        response_format: request.format.as_str(),
    })
}

/// `GET …/audio/voices` as vLLM-Omni and SGLang-Omni answer it: `voices`,
/// the model's preset voices by name, which vLLM-Omni's list includes its
/// uploaded ones in, and `uploaded_voices`, the server's own, each with its
/// `name` and the `speaker_description` it was uploaded with
/// ([vLLM-Omni](https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/speech_api/),
/// [SGLang-Omni](https://github.com/sgl-project/sglang-omni/blob/main/sglang_omni/serve/speech_voices.py)).
/// A server that lists objects instead, each with its `id`, `voice_id` or
/// `name` and its `display_name` or `name`, `description` and `language`,
/// reads too.
#[derive(Deserialize)]
struct VoiceListing {
    #[serde(default)]
    voices: Vec<ListedVoice>,
    #[serde(default)]
    uploaded_voices: Vec<UploadedVoice>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ListedVoice {
    Named(String),
    Described(DescribedVoice),
}

#[derive(Deserialize)]
struct DescribedVoice {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    voice_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Deserialize)]
struct UploadedVoice {
    name: String,
    #[serde(default)]
    speaker_description: Option<String>,
}

impl VoiceListing {
    /// The voices, each once: a preset by its name, the server's own with
    /// what it was uploaded with, in the list's order, then any uploaded
    /// voice the list leaves out. A described voice with no id is skipped:
    /// `--voice` could not name it.
    fn into_voices(self) -> Vec<Voice> {
        let mut voices: Vec<Voice> = self
            .voices
            .into_iter()
            .filter_map(|voice| match voice {
                ListedVoice::Named(name) => Some(Voice { own: None, ..Voice::named(&name) }),
                ListedVoice::Described(voice) => {
                    let id = voice.id.or(voice.voice_id).or_else(|| voice.name.clone())?;
                    let name = voice.display_name.or(voice.name).filter(|name| *name != id);
                    Some(Voice {
                        id,
                        name,
                        languages: voice.language.into_iter().collect(),
                        description: voice.description,
                        own: None,
                    })
                }
            })
            .collect();
        for uploaded in self.uploaded_voices {
            let own = Voice { own: Some(true), description: uploaded.speaker_description, ..Voice::named(&uploaded.name) };
            match voices.iter_mut().find(|voice| voice.id == uploaded.name) {
                Some(listed) => *listed = Voice { name: listed.name.take(), languages: std::mem::take(&mut listed.languages), ..own },
                None => voices.push(own),
            }
        }
        voices
    }
}
