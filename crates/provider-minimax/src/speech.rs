//! MiniMax's speech through its key.
//!
//! `POST /v1/t2a_v2` on the entry's origin reads `text` aloud in
//! `voice_setting.voice_id` at `voice_setting.speed`, in the format
//! `audio_setting.format` names. It answers the audio as hexadecimal text in
//! `data.audio`, or the refusal its `base_resp` states ([Text to
//! speech](https://platform.minimax.io/docs/api-reference/speech-t2a-http)).
//!
//! Its voices, the system's, the cloned and the designed ones, are what `POST
//! /v1/get_voice` lists for `voice_type: "all"` ([Get
//! voice](https://platform.minimax.io/docs/api-reference/voice-management-get)).
//!
//! MiniMax has no default voice, so a call without `--voice` fails before it is
//! sent. It makes no music: it closed its music API to new users.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret,
    audio::{AudioMaker, AudioRequest, MadeAudio, Voice, searched},
    endpoint_url, json_body,
    media::{Ask, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable},
    media_catalog::{Catalog, MediaKind},
};
use demi_shared_types::{AudioShape, Clock};
use futures_util::future::BoxFuture;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::minimax;

/// A MiniMax key's speech role.
pub(crate) struct MinimaxSpeech {
    vendor: MediaVendor,
    key: Secret,
    speech: Url,
    voices: Url,
    clock: Arc<dyn Clock>,
}

impl MinimaxSpeech {
    pub(crate) fn new(vendor: MediaVendor, key: Secret, origin: &Url, clock: Arc<dyn Clock>) -> Self {
        Self {
            vendor,
            key,
            speech: endpoint_url(origin, "/v1/t2a_v2"),
            voices: endpoint_url(origin, "/v1/get_voice"),
            clock,
        }
    }

    async fn post(&self, http: &demi_provider_common::ProviderHttp, url: &Url, body: Vec<u8>) -> MediaAnswer<Bytes> {
        minimax::send_checked(&*self.clock, http, &self.key, Ask::post_json(url, Bytes::from(body))).await
    }
}

impl MediaMaker for MinimaxSpeech {
    fn name(&self) -> &str {
        self.vendor.name()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(self.vendor)
    }

    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(std::future::ready(Ok(catalog_models(catalog, self.vendor, MediaKind::Audio))))
    }
}

impl AudioMaker for MinimaxSpeech {
    fn shape(&self) -> AudioShape {
        AudioShape::Minimax
    }

    fn needs_voice(&self) -> bool {
        true
    }

    /// `voice_setting.speed` takes 0.5 to 2.
    fn speeds(&self) -> Option<(f64, f64)> {
        Some((0.5, 2.0))
    }

    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        request: AudioRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(async move {
            let body = json_body(&Body {
                model: &request.model,
                text: &request.text,
                stream: false,
                voice_setting: VoiceSetting { voice_id: request.voice.as_deref().unwrap_or_default(), speed: request.speed },
                audio_setting: AudioSetting { format: request.format.as_str() },
                output_format: "hex",
            });
            self.post(http, &self.speech, body).await.and_then(|body| read_audio(&body))
        })
    }

    fn voices<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        _: &'a str,
        search: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Vec<Voice>>, MediaFailure>> {
        Box::pin(async move {
            let body = json_body(&VoiceType { voice_type: "all" });
            let listed = self.post(http, &self.voices, body).await.result?;
            Ok(Some(searched(read_voices(&listed)?, search)))
        })
    }
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    text: &'a str,
    stream: bool,
    voice_setting: VoiceSetting<'a>,
    audio_setting: AudioSetting,
    output_format: &'static str,
}

#[derive(Serialize)]
struct VoiceSetting<'a> {
    voice_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
}

#[derive(Serialize)]
struct AudioSetting {
    format: &'static str,
}

#[derive(Serialize)]
struct VoiceType {
    voice_type: &'static str,
}

/// The audio of MiniMax's answer, `data.audio` decoded from hexadecimal.
fn read_audio(body: &[u8]) -> Result<MadeAudio, MediaFailure> {
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        data: Option<Data>,
        #[serde(default)]
        trace_id: Option<String>,
    }
    #[derive(Deserialize)]
    struct Data {
        #[serde(default)]
        audio: Option<String>,
    }
    let answer: Answer =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    let audio = answer
        .data
        .and_then(|data| data.audio)
        .filter(|audio| !audio.is_empty())
        .ok_or_else(|| unreadable("its answer held no audio".to_owned()))?;
    let bytes = hex::decode(audio).map_err(|error| unreadable(format!("its audio is not the hexadecimal text it documents: {error}")))?;
    Ok(MadeAudio { bytes: Bytes::from(bytes), pcm: None, cost: None, usage: None, vendor_id: answer.trace_id })
}

#[derive(Deserialize)]
struct Listed {
    #[serde(default)]
    system_voice: Vec<Listing>,
    #[serde(default)]
    voice_cloning: Vec<Listing>,
    #[serde(default)]
    voice_generation: Vec<Listing>,
}

#[derive(Deserialize)]
struct Listing {
    voice_id: String,
    #[serde(default)]
    voice_name: Option<String>,
    #[serde(default)]
    description: Vec<String>,
}

/// The voices `get_voice` listed: the system's, then the account's own,
/// cloned and designed.
fn read_voices(body: &[u8]) -> Result<Vec<Voice>, MediaFailure> {
    let listed: Listed =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its voices could not be read: {error}")))?;
    let voice = |listing: Listing, own: bool| Voice {
        id: listing.voice_id,
        name: listing.voice_name.filter(|name| !name.is_empty()),
        languages: Vec::new(),
        description: Some(listing.description.join(" ")).filter(|description| !description.is_empty()),
        own: Some(own),
    };
    let system = listed.system_voice.into_iter().map(|listing| voice(listing, false));
    let own = listed.voice_cloning.into_iter().chain(listed.voice_generation).map(|listing| voice(listing, true));
    Ok(system.chain(own).collect())
}
