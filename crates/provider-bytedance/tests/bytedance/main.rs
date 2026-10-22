//! ByteDance's media through its keys, against scripted vendors built from its
//! API references:
//!
//! - Ark's Image generation API;
//! - Create, Retrieve and Cancel or delete a video generation task
//!   (docs.byteplus.com/en/docs/modelark/…);
//! - BytePlus Speech's TTS Unidirectional Streaming (HTTP) and Audio 1.0
//!   (…/byteplusvoice/…);
//! - Doubao Speech's HTTP Chunked/SSE V3
//!   (docs.volcengine.com/docs/DoubaoVoice/…).
//!
//! No test calls a vendor.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use demi_provider_bytedance::{Ark, ArkConfig, ArkProvider, Speech, SpeechConfig, SpeechProvider};
use demi_provider_common::{
    ErrorCode, Provider, ProviderHttp, Secret,
    audio::{AudioFormatAsk, AudioOption, AudioRequest},
    images::{Aspect, InputImage},
    media::{MediaFailure, MediaVendor},
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
    videos::{JobState, VideoRequest},
};
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, AudioShape, VendorTrace, VideoShape};
use serde_json::{Value, json};

const NOW: &str = "2026-10-18T12:00:00.000Z";

const CATALOG: &str = r#"{ "updated": "2026-10-01T00:00:00Z", "models": [
    { "vendor": "byteplus", "id": "dola-seedream-5-0-pro-260628", "kind": "image", "name": "Dola Seedream 5.0 Pro",
      "image": { "sizes": ["2048x2048", "2368x1776", "1776x2368", "2816x1584", "1584x2816"] } },
    { "vendor": "byteplus", "id": "dreamina-seedance-2-5-260628", "kind": "video", "name": "Dreamina Seedance 2.5" },
    { "vendor": "volcengine", "id": "doubao-seedream-5-0-pro-260628", "kind": "image", "name": "Doubao Seedream 5.0 Pro" },
    { "vendor": "byteplus-speech", "id": "seed-audio-1.0", "kind": "audio", "name": "Audio 1.0",
      "audio": { "makes": ["speech", "effect"] } },
    { "vendor": "byteplus-speech", "id": "seed-tts-2.0", "kind": "audio", "name": "TTS 2.0",
      "audio": { "makes": ["speech"], "voices": ["en_female_stokie_uranus_bigtts", "zh_female_vv_uranus_bigtts"] } },
    { "vendor": "doubao-speech", "id": "seed-tts-2.0", "kind": "audio", "name": "豆包语音合成模型2.0",
      "audio": { "makes": ["speech"], "voices": ["zh_female_xiaohe_uranus_bigtts"] } }
] }"#;

fn catalog() -> Catalog {
    Catalog::decode(CATALOG.as_bytes()).unwrap()
}

fn http() -> ProviderHttp {
    ProviderHttp::new(reqwest::Client::new())
}

fn clock() -> Arc<FixedClock> {
    Arc::new(FixedClock(NOW.parse().unwrap()))
}

fn key(text: &str) -> Option<Secret> {
    Some(Secret::try_from(text.to_owned()).unwrap())
}

fn ark(vendor: &MockVendor, which: Ark) -> ArkProvider {
    let config = ArkConfig {
        ark: which,
        api_key: key("ark-key"),
        base_url: Some(vendor.url("/api/v3").parse().unwrap()),
        makes_images: true,
        makes_videos: true,
        text: None,
    };
    ArkProvider::new(config, clock())
}

fn speech(vendor: &MockVendor, which: Speech) -> SpeechProvider {
    let config = SpeechConfig { speech: which, api_key: key("speech-key"), base_url: Some(vendor.url("/").parse().unwrap()) };
    SpeechProvider::new(config, clock())
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn png(bytes: &'static [u8]) -> InputImage {
    InputImage { media_type: "image/png", bytes: Bytes::from_static(bytes) }
}

fn audio(kind: AudioKind, model: &str, text: &str) -> AudioRequest {
    AudioRequest {
        kind,
        model: model.into(),
        text: text.into(),
        voice: None,
        instructions: None,
        speed: None,
        duration: None,
        instrumental: false,
        seed: None,
        looped: false,
        format: AudioFormatAsk::Mp3,
    }
}

/// Planted defects this catches:
///
/// - the prompt or an image outside `content`, or an image in the wrong role;
/// - an option sent other than in its field, or the watermark left on;
/// - a status read wrongly, `running` as queued, `cancelled` or `expired` as
///   made;
/// - the video's address or its tokens not read;
/// - the key sent to the video's address.
// One scripted vendor, nine requests: milliseconds.
#[tokio::test]
async fn an_ark_task_is_asked_read_through_its_states_and_downloaded() {
    let vendor = MockVendor::start().await;
    let tasks = "/api/v3/contents/generations/tasks";
    vendor.respond_at(tasks, json_answer(200, json!({ "id": "cgt-1" })));
    let task = "/api/v3/contents/generations/tasks/cgt-1";
    let video = vendor.url("/files/cgt-1.mp4");
    for state in [
        json!({ "id": "cgt-1", "status": "queued" }),
        json!({ "id": "cgt-1", "status": "running" }),
        json!({ "id": "cgt-1", "status": "succeeded", "content": { "video_url": video }, "usage": { "completion_tokens": 108900, "total_tokens": 108900 } }),
        json!({ "id": "cgt-1", "status": "failed", "error": { "code": "InputImageSensitiveContentDetected", "message": "The input image may contain sensitive content" } }),
        json!({ "id": "cgt-1", "status": "cancelled" }),
        json!({ "id": "cgt-1", "status": "expired" }),
    ] {
        vendor.respond_at(task, json_answer(200, state));
    }
    vendor.respond_at("/files/cgt-1.mp4", MockResponse::status(200).chunk("ftyp-video"));
    let provider = ark(&vendor, Ark::Volcengine);
    let maker = provider.videos().expect("an Ark key makes videos");
    assert_eq!((maker.vendor(), maker.shape()), (Some(MediaVendor::Volcengine), VideoShape::ArkTasks));
    let http = http();

    let request = VideoRequest {
        model: "doubao-seedance-2-5-260628".into(),
        prompt: "Waves on a pier".into(),
        image: Some(png(b"first")),
        last_frame: Some(png(b"last")),
        references: vec![png(b"product")],
        duration: Some(8),
        aspect: Some(Aspect { width: 9, height: 16 }),
        resolution: Some(demi_shared_types::VideoResolution::Height(1080)),
        audio: Some(false),
        seed: Some(7),
    };
    assert_eq!(demi_provider_common::videos::unsent(VideoShape::ArkTasks, &request), []);
    assert_eq!(maker.ask(&http, empty_catalog(), request).await.result.unwrap(), "cgt-1");
    let mut states = Vec::new();
    for _ in 0..6 {
        states.push(maker.state(&http, taken("cgt-1")).await.result.unwrap());
    }
    let JobState::Done(done) = &states[2] else { panic!("{states:?}") };
    assert_eq!((done.url.as_str(), done.usage.map(|usage| usage.output_tokens)), (video.as_str(), Some(108900)));
    assert_eq!(states[0], JobState::Queued);
    assert_eq!(states[1], JobState::Running { progress: None });
    assert_eq!(
        states[3..],
        [
            JobState::Failed {
                message: "The input image may contain sensitive content".into(),
                trace: VendorTrace { code: Some("InputImageSensitiveContentDetected".into()), id: None },
            },
            JobState::Failed { message: "the task was cancelled".into(), trace: Default::default() },
            JobState::Expired,
        ]
    );
    assert_eq!(maker.download(&http, done).await.result.unwrap().as_ref(), b"ftyp-video");

    let sent = vendor.requests();
    let data = |bytes: &[u8]| format!("data:image/png;base64,{}", b64(bytes));
    assert_eq!(
        sent[0].json(),
        json!({ "model": "doubao-seedance-2-5-260628",
                "content": [
                    { "type": "text", "text": "Waves on a pier" },
                    { "type": "image_url", "image_url": { "url": data(b"first") }, "role": "first_frame" },
                    { "type": "image_url", "image_url": { "url": data(b"last") }, "role": "last_frame" },
                    { "type": "image_url", "image_url": { "url": data(b"product") }, "role": "reference_image" }
                ],
                "duration": 8, "ratio": "9:16", "resolution": "1080p", "generate_audio": false, "seed": 7, "watermark": false })
    );
    assert_eq!(sent[1].header("authorization"), Some("Bearer ark-key"));
    let download = sent.last().unwrap();
    assert_eq!((download.uri.path(), download.header("authorization")), ("/files/cgt-1.mp4", None));
}

/// The answer of a speech: chunked JSON objects, each with a piece of the
/// audio, then the one that ends it, as both services document.
fn speech_answer(pieces: &[&[u8]]) -> MockResponse {
    let mut response = MockResponse::status(200).header("content-type", "application/json");
    for piece in pieces {
        response = response.chunk(json!({ "code": 0, "message": "", "data": b64(piece) }).to_string());
    }
    response
        .chunk(json!({ "code": 0, "message": "", "data": null, "sentence": { "text": "Hello there." } }).to_string())
        .chunk(json!({ "code": 20000000, "message": "ok", "data": null, "usage": { "text_words": 12 } }).to_string())
}

/// Planted defects this catches:
///
/// - the pieces not joined in order, or read past the end;
/// - a text object read as audio;
/// - WAV asked as `wav`, whose header the services repeat in every piece, or
///   not marked as 24 kHz PCM;
/// - the speed's rate counted wrongly;
/// - the model outside `X-Api-Resource-Id`;
/// - BytePlus' fixed app key left out or sent to Doubao Speech;
/// - a failure code read as speech, or its code or log ID lost;
/// - the account's concurrency read as a refusal;
/// - an answer that ends early read as whole.
// Two scripted vendors, six requests: milliseconds.
#[tokio::test]
async fn seed_speech_joins_its_pieces_until_the_end_and_wraps_pcm_for_wav() {
    let byteplus = MockVendor::start().await;
    let path = "/api/v3/tts/unidirectional";
    byteplus.respond_at(path, speech_answer(&[b"ID3-", b"one", b"-two"]));
    byteplus.respond_at(path, speech_answer(&[&[0u8; 4], &[0u8; 6]]));
    byteplus.respond_at(path, json_answer(200, json!({ "code": 45000000, "message": "speaker permission denied: get resource id: access denied", "data": null })));
    byteplus.respond_at(
        path,
        json_answer(200, json!({ "code": 45000000, "message": "quota exceeded for types: concurrency", "data": null }))
            .header("x-tt-logid", "20261018120000AB"),
    );
    byteplus.respond_at(path, MockResponse::status(200).chunk(json!({ "code": 0, "message": "", "data": b64(b"half") }).to_string()));
    let doubao = MockVendor::start().await;
    doubao.respond_at(path, speech_answer(&[b"ID3-zh"]));
    let provider = speech(&byteplus, Speech::Byteplus);
    let maker = provider.audio().expect("a speech key makes audio");
    assert_eq!((maker.vendor(), maker.shape(), maker.needs_voice()), (Some(MediaVendor::ByteplusSpeech), AudioShape::Seed, true));
    let (http, catalog) = (http(), catalog());
    let hello = AudioRequest {
        voice: Some("en_female_stokie_uranus_bigtts".into()),
        speed: Some(1.5),
        instructions: Some("Whisper".into()),
        ..audio(AudioKind::Speech, "seed-tts-2.0", "Hello there.")
    };
    assert_eq!(maker.unsent(&catalog, &hello), [AudioOption::Instructions]);

    let mp3 = maker.make(&http, &catalog, hello.clone()).await.result.unwrap();
    assert_eq!((mp3.bytes.as_ref(), mp3.pcm), (b"ID3-one-two".as_slice(), None));
    let wav = maker.make(&http, &catalog, AudioRequest { format: AudioFormatAsk::Wav, speed: Some(0.5), ..hello.clone() }).await.result.unwrap();
    assert_eq!((wav.bytes.len(), wav.pcm), (10, Some(Pcm::new(24_000, 1, 16).unwrap())));
    let denied = maker.make(&http, &catalog, hello.clone()).await.result;
    assert!(
        matches!(&denied, Err(MediaFailure::Refused { message, trace, .. })
            if message == "speaker permission denied: get resource id: access denied" && trace.code.as_deref() == Some("45000000")),
        "{denied:?}"
    );
    let busy = maker.make(&http, &catalog, hello.clone()).await.result;
    assert!(
        matches!(&busy, Err(MediaFailure::Unanswered { code: Some(ErrorCode::RateLimit), trace, .. })
            if *trace == VendorTrace { code: Some("45000000".into()), id: Some("20261018120000AB".into()) }),
        "the account's concurrency is a rate limit, traced by its log ID: {busy:?}"
    );
    let cut = maker.make(&http, &catalog, hello.clone()).await.result;
    assert!(matches!(cut, Err(MediaFailure::Unanswered { .. })), "{cut:?}");
    let chinese = speech(&doubao, Speech::Doubao);
    let zh = AudioRequest { voice: Some("zh_female_xiaohe_uranus_bigtts".into()), speed: None, instructions: None, ..hello };
    chinese.audio().unwrap().make(&http, &catalog, zh).await.result.unwrap();

    let sent = byteplus.requests();
    let headers = |request: &demi_provider_common::testing::RecordedRequest| {
        ["x-api-key", "x-api-resource-id", "x-api-app-key"].map(|name| request.header(name).map(str::to_owned))
    };
    assert_eq!(headers(&sent[0]), [Some("speech-key".into()), Some("seed-tts-2.0".into()), Some("aGjiRDfUWi".into())]);
    assert_eq!(
        sent[0].json(),
        json!({ "req_params": { "text": "Hello there.", "speaker": "en_female_stokie_uranus_bigtts",
                                "audio_params": { "format": "mp3", "sample_rate": 24000, "speech_rate": 50 } } })
    );
    assert_eq!(sent[1].json()["req_params"]["audio_params"], json!({ "format": "pcm", "sample_rate": 24000, "speech_rate": -50 }));
    let zh_sent = &doubao.requests()[0];
    assert_eq!(headers(zh_sent), [Some("speech-key".into()), Some("seed-tts-2.0".into()), None]);
    assert_eq!(zh_sent.json()["req_params"]["audio_params"], json!({ "format": "mp3", "sample_rate": 24000 }));
}

/// A catalog of no models, for a maker whose requests are one.
fn empty_catalog() -> &'static Catalog {
    static EMPTY: std::sync::LazyLock<Catalog> =
        std::sync::LazyLock::new(|| Catalog::decode(br#"{ "updated": "2026-10-01T00:00:00Z", "models": [] }"#).unwrap());
    &EMPTY
}

/// The job `id`, as the backend names a taken job to its maker.
fn taken(id: &str) -> demi_provider_common::videos::JobRef<'_> {
    demi_provider_common::videos::JobRef { id, model: "", catalog: empty_catalog() }
}
