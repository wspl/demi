//! MiniMax's speech through its key.
//!
//! Speech is an asynchronous task, which outlives a restart of the server
//! ([Asynchronous text to
//! speech](https://platform.minimax.io/docs/api-reference/speech-t2a-async-create)):
//!
//! 1. `POST /v1/t2a_async_v2` on the entry's origin reads `text` aloud in
//!    `voice_setting.voice_id` at `voice_setting.speed`, in the format
//!    `audio_setting.format` names, and answers the task's `task_id`, or the
//!    refusal its `base_resp` states.
//! 2. `GET /v1/query/t2a_async_query_v2?task_id=` reads it: `Processing`,
//!    then `Success` with its `file_id`, `Failed` or `Expired`.
//! 3. `GET /v1/files/retrieve?file_id=` names the file's `download_url`, a
//!    TAR of the audio and its timing, downloaded without a header.
//!
//! Its voices, the system's, the cloned and the designed ones, are what `POST
//! /v1/get_voice` lists for `voice_type: "all"` ([Get
//! voice](https://platform.minimax.io/docs/api-reference/voice-management-get)).
//!
//! MiniMax has no default voice, so a call without `--voice` fails before it is
//! sent. It makes no music: it closed its music API to new users.

use std::sync::Arc;

use std::io::Read as _;

use bytes::Bytes;
use demi_provider_common::{
    Secret,
    audio::{AudioMaker, AudioOption, AudioRequest, MadeAudio, Voice, searched, unsent_of},
    endpoint_url, json_body,
    media::{
        Ask, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable, within_limit,
        jobs::{self, JobRef, JobState, Jobs},
    },
    media_catalog::{Catalog, MediaKind},
    videos,
};
use demi_shared_types::Clock;
use futures_util::future::BoxFuture;
use reqwest::Url;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::minimax;

/// A MiniMax key's speech role.
pub(crate) struct MinimaxSpeech {
    vendor: MediaVendor,
    key: Secret,
    origin: Url,
    voices: Url,
    clock: Arc<dyn Clock>,
}

impl MinimaxSpeech {
    pub(crate) fn new(vendor: MediaVendor, key: Secret, origin: &Url, clock: Arc<dyn Clock>) -> Self {
        Self {
            vendor,
            key,
            origin: origin.clone(),
            voices: endpoint_url(origin, "/v1/get_voice"),
            clock,
        }
    }

    async fn get(&self, http: &demi_provider_common::ProviderHttp, path: &str, query: (&str, &str)) -> MediaAnswer<Bytes> {
        let mut url = endpoint_url(&self.origin, path);
        url.query_pairs_mut().append_pair(query.0, query.1);
        minimax::send_checked(&*self.clock, http, &self.key, Ask::get(&url)).await
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
    /// MiniMax makes speech alone, which takes neither instructions nor a
    /// seed.
    fn unsent(&self, _: &Catalog, request: &AudioRequest) -> Vec<AudioOption> {
        unsent_of(&[AudioOption::Instructions, AudioOption::Seed], request)
    }

    fn shape_name(&self, _: &Catalog, _: &AudioRequest) -> String {
        "MiniMax speech".to_owned()
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
        catalog: &'a Catalog,
        request: AudioRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(async move {
            let model = request.model.clone();
            jobs::made(self, http, catalog, request, &model).await
        })
    }

    fn jobs(&self, _: &Catalog, _: demi_shared_types::AudioKind, _: &str) -> Option<&dyn Jobs<AudioRequest, MadeAudio>> {
        Some(self)
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

impl Jobs<AudioRequest, MadeAudio> for MinimaxSpeech {
    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, _: &'a Catalog, request: AudioRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let body = json_body(&Body {
                model: &request.model,
                text: &request.text,
                voice_setting: VoiceSetting { voice_id: request.voice.as_deref().unwrap_or_default(), speed: request.speed },
                audio_setting: AudioSetting { format: request.format.as_str() },
            });
            let url = endpoint_url(&self.origin, "/v1/t2a_async_v2");
            self.post(http, &url, body).await.and_then(|body| read_id(&body, "task_id"))
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState<MadeAudio>>> {
        Box::pin(async move {
            let read = self.get(http, "/v1/query/t2a_async_query_v2", ("task_id", job.id)).await;
            let mut observed = read.observed;
            let file = match read.result.and_then(|body| read_state(&body)) {
                Ok(Ok(file)) => file,
                Ok(Err(state)) => return MediaAnswer { result: Ok(state), observed },
                Err(failure) => return MediaAnswer { result: Err(failure), observed },
            };
            // A finished task names its file, whose address another read
            // gives.
            let retrieved = self.get(http, "/v1/files/retrieve", ("file_id", &file)).await;
            observed.extend(retrieved.observed);
            let url = match retrieved.result.and_then(|body| read_file(&body)) {
                Ok(url) => url,
                Err(failure) => return MediaAnswer { result: Err(failure), observed },
            };
            let downloaded = videos::download(minimax::LABEL, &*self.clock, http, &url, &self.origin, HeaderMap::new()).await;
            observed.extend(downloaded.observed);
            let result = downloaded.result.and_then(|archive| {
                let bytes = within_limit(audio_of(&archive)?, "audio")?;
                Ok(JobState::Done(MadeAudio { bytes, pcm: None, cost: None, usage: None, vendor_id: Some(job.id.to_owned()) }))
            });
            MediaAnswer { result, observed }
        })
    }
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    text: &'a str,
    voice_setting: VoiceSetting<'a>,
    audio_setting: AudioSetting,
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

/// The id `field` names in `body`, which MiniMax sends as a number.
fn read_id(body: &[u8], field: &str) -> Result<String, MediaFailure> {
    let answer: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    match &answer[field] {
        serde_json::Value::Number(id) => Ok(id.to_string()),
        serde_json::Value::String(id) if !id.is_empty() => Ok(id.clone()),
        _ => Err(unreadable(format!("its answer named no {field}"))),
    }
}

/// A finished task's file id, or the state of one that has not finished.
fn read_state(body: &[u8]) -> Result<Result<String, JobState<MadeAudio>>, MediaFailure> {
    let answer: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its task's state could not be read: {error}")))?;
    match answer["status"].as_str().unwrap_or_default() {
        "Processing" => Ok(Err(JobState::Running { progress: None })),
        "Failed" => Ok(Err(JobState::Failed { reason: None, trace: Default::default() })),
        "Expired" => Ok(Err(JobState::Expired)),
        "Success" => read_id(body, "file_id").map(Ok),
        other => Err(unreadable(format!("its task's state \"{other}\" is not one it documents"))),
    }
}

/// The address of the file its retrieval names.
fn read_file(body: &[u8]) -> Result<Url, MediaFailure> {
    #[derive(Deserialize)]
    struct Retrieved {
        file: File,
    }
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        download_url: Option<String>,
    }
    let retrieved: Retrieved =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its file could not be read: {error}")))?;
    let url = retrieved.file.download_url.filter(|url| !url.is_empty());
    let url = url.ok_or_else(|| unreadable("its finished task named no file".to_owned()))?;
    Url::parse(&url).map_err(|error| unreadable(format!("its file's address could not be read: {error}")))
}

/// The audio in a finished task's archive: the one entry that is neither
/// its timing (`.titles`) nor its facts (`.extra`).
fn audio_of(archive: &[u8]) -> Result<Bytes, MediaFailure> {
    let unread = |error: std::io::Error| unreadable(format!("its archive could not be read: {error}"));
    let mut entries = tar::Archive::new(archive);
    for entry in entries.entries().map_err(unread)? {
        let mut entry = entry.map_err(unread)?;
        let path = entry.path().map_err(unread)?.into_owned();
        let audio = path.extension().is_some_and(|extension| extension != "titles" && extension != "extra");
        if entry.header().entry_type().is_file() && audio {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(unread)?;
            return Ok(Bytes::from(bytes));
        }
    }
    Err(unreadable("its archive held no audio".to_owned()))
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
