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
    images::{Aspect, ImageRequest, InputImage},
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
/// - the image asked for as an address rather than base64, or with Ark's
///   watermark, its JPEG or its group of images;
/// - a shape sent as other than the catalog's size of it, or a size sent with
///   no shape asked;
/// - one edit image sent as an array, or several as one;
/// - the key other than a bearer token;
/// - an image Ark refused read as made.
// One scripted vendor, four requests: milliseconds.
#[tokio::test]
async fn seedream_generates_and_edits_as_base64_in_the_catalogs_size() {
    let vendor = MockVendor::start().await;
    let path = "/api/v3/images/generations";
    for image in [b"made-1".as_slice(), b"made-2", b"made-3"] {
        vendor.respond_at(path, json_answer(200, json!({ "model": "dola-seedream-5-0-pro-260628", "data": [{ "b64_json": b64(image), "size": "2816x1584" }] })));
    }
    vendor.respond_at(path, json_answer(200, json!({ "data": [{ "error": { "code": "OutputImageSensitiveContentDetected", "message": "The image may contain sensitive content" } }] })));
    let provider = ark(&vendor, Ark::Byteplus);
    let maker = provider.images().expect("an Ark key makes images");
    assert_eq!(maker.vendor(), Some(MediaVendor::Byteplus));
    let (http, catalog) = (http(), catalog());
    let request = |aspect: Option<Aspect>, images: Vec<InputImage>| ImageRequest {
        model: "dola-seedream-5-0-pro-260628".into(),
        prompt: "A lighthouse at dusk".into(),
        aspect,
        transparent: false,
        images,
    };

    let wide = maker.make(&http, &catalog, request(Some(Aspect { width: 16, height: 9 }), Vec::new())).await.result.unwrap();
    assert_eq!(wide.bytes.as_ref(), b"made-1");
    maker.make(&http, &catalog, request(None, vec![png(b"one")])).await.result.unwrap();
    maker.make(&http, &catalog, request(None, vec![png(b"one"), png(b"two")])).await.result.unwrap();
    let refused = maker.make(&http, &catalog, request(None, Vec::new())).await.result;
    assert!(
        matches!(&refused, Err(MediaFailure::Refused { message, trace, .. })
            if message == "The image may contain sensitive content" && trace.code.as_deref() == Some("OutputImageSensitiveContentDetected")),
        "{refused:?}"
    );

    let sent = vendor.requests();
    assert_eq!(sent[0].header("authorization"), Some("Bearer ark-key"));
    assert_eq!(
        sent[0].json(),
        json!({ "model": "dola-seedream-5-0-pro-260628", "prompt": "A lighthouse at dusk", "size": "2816x1584",
                "response_format": "b64_json", "watermark": false, "sequential_image_generation": "disabled", "output_format": "png" })
    );
    assert_eq!(sent[1].json()["image"], json!(format!("data:image/png;base64,{}", b64(b"one"))));
    assert_eq!(sent[1].json().get("size"), None, "no shape asked sends no size");
    assert_eq!(
        sent[2].json()["image"],
        json!([format!("data:image/png;base64,{}", b64(b"one")), format!("data:image/png;base64,{}", b64(b"two"))])
    );
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

/// Planted defects this catches: a stopped call that deletes a running or
/// finished task, which Ark cannot stop and whose record it would delete,
/// or one that leaves a queued task to run and be paid for.
// One scripted vendor, three requests: milliseconds.
#[tokio::test]
async fn a_stopped_call_cancels_its_task_only_while_it_is_queued() {
    let vendor = MockVendor::start().await;
    let task = "/api/v3/contents/generations/tasks/cgt-2";
    vendor.respond_at(task, json_answer(200, json!({ "id": "cgt-2", "status": "running" })));
    vendor.respond_at(task, json_answer(200, json!({ "id": "cgt-2", "status": "queued" })));
    vendor.respond_at(task, MockResponse::status(200).chunk("{}"));
    let provider = ark(&vendor, Ark::Byteplus);
    let maker = provider.videos().unwrap();
    let http = http();

    maker.cancel(&http, taken("cgt-2")).expect("Ark cancels a task").await.result.unwrap();
    maker.cancel(&http, taken("cgt-2")).expect("Ark cancels a task").await.result.unwrap();

    let methods: Vec<_> = vendor.requests().iter().map(|request| (request.method.as_str().to_owned(), request.uri.path().to_owned())).collect();
    assert_eq!(
        methods,
        [("GET".to_owned(), task.to_owned()), ("GET".to_owned(), task.to_owned()), ("DELETE".to_owned(), task.to_owned())]
    );
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

/// Planted defects this catches:
///
/// - an effect, or Seed Audio's speech, sent to Seed speech's endpoint, or with
///   a resource header;
/// - the speaker or the instructions left out of Seed Audio's speech, or the
///   speaker sent outside its one `references` item, or the instructions sent
///   with an effect;
/// - the format or the speed's rate not asked;
/// - the base64 not decoded;
/// - an answer without audio read as made;
/// - the options a Seed speech model does not take said of Seed Audio's, or its
///   name.
// One scripted vendor, three requests: milliseconds.
#[tokio::test]
async fn seed_audio_makes_an_effect_and_speaks_as_base64() {
    let vendor = MockVendor::start().await;
    let path = "/api/v3/tts/create";
    vendor.respond_at(path, json_answer(200, json!({ "audio": b64(b"OggS-effect"), "duration": 3.2, "original_duration": 3.2, "url": "https://example.test/a" })));
    vendor.respond_at(path, json_answer(200, json!({ "audio": b64(b"RIFF-speech"), "duration": 1.0 })));
    vendor.respond_at(path, json_answer(200, json!({ "code": 40000001, "message": "text_prompt is required" })));
    let provider = speech(&vendor, Speech::Byteplus);
    let maker = provider.audio().unwrap();
    let (http, catalog) = (http(), catalog());
    let door = AudioRequest { duration: Some(3.0), looped: true, format: AudioFormatAsk::Opus, ..audio(AudioKind::Effect, "seed-audio-1.0", "A door creaks open") };
    assert_eq!(maker.unsent(&catalog, &door), [AudioOption::Duration, AudioOption::Loop]);
    let spoken = AudioRequest {
        voice: Some("en_female_stokie_uranus_bigtts".into()),
        instructions: Some("Whisper, as in a library".into()),
        speed: Some(2.0),
        seed: Some(3),
        format: AudioFormatAsk::Wav,
        ..audio(AudioKind::Speech, "seed-audio-1.0", "Stay close.")
    };
    assert_eq!(maker.unsent(&catalog, &spoken), [AudioOption::Seed]);
    let tts = AudioRequest { model: "seed-tts-2.0".into(), ..spoken.clone() };
    assert_eq!(maker.unsent(&catalog, &tts), [AudioOption::Instructions, AudioOption::Seed]);
    let names = [&door, &spoken, &tts].map(|request| maker.shape_name(&catalog, request));
    assert_eq!(names, ["Seed Audio", "Seed Audio", "Seed speech"]);

    let made = maker.make(&http, &catalog, door.clone()).await.result.unwrap();
    assert_eq!((made.bytes.as_ref(), made.pcm), (b"OggS-effect".as_slice(), None));
    let said = maker.make(&http, &catalog, spoken).await.result.unwrap();
    assert_eq!((said.bytes.as_ref(), said.pcm), (b"RIFF-speech".as_slice(), None), "Seed Audio's WAV is a file");
    let refused = maker.make(&http, &catalog, door).await.result;
    assert!(
        matches!(&refused, Err(MediaFailure::Refused { message, trace, .. }) if message == "text_prompt is required" && trace.code.as_deref() == Some("40000001")),
        "{refused:?}"
    );

    let sent = vendor.requests();
    assert_eq!((sent[0].header("x-api-key"), sent[0].header("x-api-resource-id")), (Some("speech-key"), None));
    assert_eq!(sent[0].json(), json!({ "model": "seed-audio-1.0", "text_prompt": "A door creaks open", "audio_config": { "format": "ogg_opus" } }));
    assert_eq!(
        sent[1].json(),
        json!({ "model": "seed-audio-1.0", "text_prompt": "Whisper, as in a library\nStay close.",
                "references": [{ "speaker": "en_female_stokie_uranus_bigtts" }],
                "audio_config": { "format": "wav", "speech_rate": 100 } })
    );
}

/// Planted defects this catches:
///
/// - a key checked by a listing ByteDance does not answer to a key;
/// - a refusal of the probe's body read as a refused key, or a refused key read
///   as a pass;
/// - the probe sent with a prompt that would make something;
/// - the models not the catalog's, or not named.
// Two scripted vendors, four requests: milliseconds.
#[tokio::test]
async fn a_key_passes_on_a_body_refusal_and_fails_on_a_key_refusal() {
    let ark_vendor = MockVendor::start().await;
    let generations = "/api/v3/images/generations";
    ark_vendor.respond_at(generations, json_answer(400, json!({ "error": { "code": "MissingParameter", "message": "The request failed because it is missing the `prompt` parameter.", "type": "BadRequest" } })));
    ark_vendor.respond_at(generations, json_answer(401, json!({ "error": { "code": "AuthenticationError", "message": "The API key or AK/SK in the request is missing or invalid.", "type": "Unauthorized" } })));
    let speech_vendor = MockVendor::start().await;
    let create = "/api/v3/tts/create";
    speech_vendor.respond_at(create, json_answer(400, json!({ "code": 40000001, "message": "text_prompt is empty" })));
    speech_vendor.respond_at(create, json_answer(401, json!({ "code": 45000010, "message": "invalid api key" })));
    let (http, catalog) = (http(), catalog());

    let images = ark(&ark_vendor, Ark::Byteplus);
    let passed = images.images().unwrap().check_key(&http, &catalog).await.unwrap();
    let named: Vec<_> = passed.iter().map(|model| (model.id.as_str(), model.name.as_deref())).collect();
    assert_eq!(named, [("dola-seedream-5-0-pro-260628", Some("Dola Seedream 5.0 Pro"))]);
    let refused = images.images().unwrap().check_key(&http, &catalog).await;
    assert!(
        matches!(&refused, Err(MediaFailure::KeyRefused { message, .. }) if message == "The API key or AK/SK in the request is missing or invalid."),
        "{refused:?}"
    );
    let probe = &ark_vendor.requests()[0];
    assert_eq!(probe.json(), json!({ "model": "dola-seedream-5-0-pro-260628", "prompt": "" }));

    let voices = speech(&speech_vendor, Speech::Byteplus);
    let passed = voices.audio().unwrap().check_key(&http, &catalog).await.unwrap();
    let kinds: Vec<_> = passed.iter().map(|model| (model.id.as_str(), model.kinds.clone())).collect();
    assert_eq!(
        kinds,
        [("seed-audio-1.0", vec![AudioKind::Speech, AudioKind::Effect]), ("seed-tts-2.0", vec![AudioKind::Speech])]
    );
    let refused = voices.audio().unwrap().check_key(&http, &catalog).await;
    assert!(matches!(&refused, Err(MediaFailure::KeyRefused { message, .. }) if message == "invalid api key"), "{refused:?}");
    assert_eq!(speech_vendor.requests()[0].json()["text_prompt"], json!(""));
}

/// Planted defects this catches: speakers read from another model's or
/// another service's list, or not searched.
// No request: microseconds.
#[tokio::test]
async fn the_speakers_are_the_catalogs_for_the_model_and_service() {
    let vendor = MockVendor::start().await;
    let (http, catalog) = (http(), catalog());
    let byteplus = speech(&vendor, Speech::Byteplus);
    let listed = byteplus.audio().unwrap().voices(&http, &catalog, "seed-tts-2.0", None).await.unwrap().unwrap();
    let ids: Vec<_> = listed.iter().map(|voice| voice.id.as_str()).collect();
    assert_eq!(ids, ["en_female_stokie_uranus_bigtts", "zh_female_vv_uranus_bigtts"]);
    let searched = byteplus.audio().unwrap().voices(&http, &catalog, "seed-tts-2.0", Some("zh_")).await.unwrap().unwrap();
    assert_eq!(searched.len(), 1);
    let doubao = speech(&vendor, Speech::Doubao);
    let listed = doubao.audio().unwrap().voices(&http, &catalog, "seed-tts-2.0", None).await.unwrap().unwrap();
    assert_eq!(listed.iter().map(|voice| voice.id.as_str()).collect::<Vec<_>>(), ["zh_female_xiaohe_uranus_bigtts"]);
    assert!(vendor.requests().is_empty());
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
