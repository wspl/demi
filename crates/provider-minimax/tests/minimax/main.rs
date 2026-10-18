//! MiniMax's media through its key (`images.md`, `videos.md`, `audio.md`
//! § Vendors), against scripted vendors built from its API reference at
//! platform.minimax.io/docs/api-reference: Text to image and Image to
//! image, Create, Query and Cancel or delete a v2 video task, Create a v1
//! text-to-video task, Query a v1 task and Retrieve file, Synchronous text
//! to speech and Get voice. No test calls a vendor.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Capabilities, Provider, ProviderHttp, ProviderRuntime, RuntimeEnv, RuntimeError, Secret,
    read_http_failure,
    audio::{AudioFormatAsk, AudioOption, AudioRequest},
    images::{Aspect, ImageRequest, InputImage},
    media::{MediaFailure, MediaVendor},
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
    videos::{JobRef, JobState, VideoOption, VideoRequest},
};
use demi_provider_minimax::{MinimaxConfig, MinimaxProvider, Region};
use demi_shared_types::{VendorTrace, 
    AudioKind, AudioShape, AuthState, ProviderErrorDiagnostics, ProviderFailureFacts, RuntimeState, Timestamp, VideoResolution, VideoShape,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

const NOW: &str = "2026-10-18T12:00:00.000Z";

const CATALOG: &str = r#"{ "updated": "2026-10-01T00:00:00Z", "models": [
    { "vendor": "minimax", "id": "image-01", "kind": "image", "name": "image-01" },
    { "vendor": "minimax", "id": "MiniMax-H3", "kind": "video", "name": "MiniMax H3",
      "video": { "durationRange": { "min": 4, "max": 15 }, "resolutions": ["768P", "2K"] }, "api": "v2" },
    { "vendor": "minimax", "id": "MiniMax-Hailuo-02", "kind": "video", "name": "MiniMax Hailuo 02",
      "video": { "durations": [6, 10] }, "api": "v1" },
    { "vendor": "minimax", "id": "MiniMax-Hailuo-Next", "kind": "video", "name": "A model of no request" },
    { "vendor": "minimax-cn", "id": "speech-2.8-hd", "kind": "audio", "name": "speech-2.8-hd",
      "audio": { "makes": ["speech"], "voices": "listed" } }
] }"#;

fn catalog() -> Catalog {
    Catalog::decode(CATALOG.as_bytes()).unwrap()
}

fn http() -> ProviderHttp {
    ProviderHttp::new(reqwest::Client::new())
}

/// The entry's text, which these tests never ask for.
struct Text;

impl Provider for Text {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async { AuthState::Authenticated { account_label: None } })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready { message: None }
    }

    fn read_failure(&self, diagnostics: &ProviderErrorDiagnostics, received_at: Timestamp) -> ProviderFailureFacts {
        read_http_failure(diagnostics, received_at)
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Err(RuntimeError::NoText { provider: "text".into() })
    }
}

/// A MiniMax entry of `region` whose endpoint, its text API's, is under
/// the scripted vendor, so its media are asked on the vendor's origin.
fn minimax(vendor: &MockVendor, region: Region) -> MinimaxProvider {
    let config = MinimaxConfig {
        region,
        api_key: Some(Secret::try_from("mm-key".to_owned()).unwrap()),
        base_url: Some(vendor.url("/anthropic/v1").parse().unwrap()),
        makes_images: true,
        makes_videos: true,
        makes_audio: true,
    };
    MinimaxProvider::new(config, Arc::new(Text), Arc::new(FixedClock(NOW.parse().unwrap())))
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

fn ok(mut body: Value) -> MockResponse {
    body["base_resp"] = json!({ "status_code": 0, "status_msg": "success" });
    json_answer(200, body)
}

/// A refusal MiniMax answers with HTTP 200.
fn base_resp(code: i64, message: &str) -> MockResponse {
    json_answer(200, json!({ "base_resp": { "status_code": code, "status_msg": message } }))
}

fn png(bytes: &'static [u8]) -> InputImage {
    InputImage { media_type: "image/png", bytes: Bytes::from_static(bytes) }
}

fn video(model: &str) -> VideoRequest {
    VideoRequest { model: model.into(), prompt: "Waves on a pier".into(), ..VideoRequest::default() }
}

fn job<'a>(id: &'a str, model: &'a str, catalog: &'a Catalog) -> JobRef<'a> {
    JobRef { id, model, catalog }
}

/// Planted defects this catches: the image asked for as an address rather
/// than base64, several asked for, the shape sent other than as
/// `aspect_ratio`, the portrait sent other than as a `character` subject;
/// the media asked on the text API's path rather than the origin; a
/// refusal MiniMax sends with HTTP 200 read as an image, or a refused key
/// or a used-up balance read as another refusal.
// One scripted vendor, five requests: milliseconds.
#[tokio::test]
async fn images_are_base64_and_a_refusal_on_http_200_is_its_base_resp() {
    let vendor = MockVendor::start().await;
    let at = "/v1/image_generation";
    vendor.respond_at(at, ok(json!({ "id": "img-1", "data": { "image_base64": ["aW1hZ2U="] }, "metadata": { "success_count": 1, "failed_count": 0 } })));
    vendor.respond_at(at, ok(json!({ "id": "img-2", "data": { "image_base64": ["ZWRpdA=="] } })));
    vendor.respond_at(at, base_resp(1026, "input new_sensitive"));
    vendor.respond_at(at, base_resp(2049, "invalid api key"));
    vendor.respond_at(at, base_resp(1008, "insufficient balance"));
    let provider = minimax(&vendor, Region::Global);
    let maker = provider.images().expect("a MiniMax key makes images");
    assert_eq!(maker.vendor(), Some(MediaVendor::Minimax));
    let (http, catalog) = (http(), catalog());
    let request = |images: Vec<InputImage>| ImageRequest {
        model: "image-01".into(),
        prompt: "A lighthouse".into(),
        aspect: Some(Aspect { width: 16, height: 9 }),
        transparent: false,
        images,
    };

    let made = maker.make(&http, &catalog, request(Vec::new())).await.result.unwrap();
    assert_eq!((made.bytes.as_ref(), made.vendor_id.as_deref()), (b"image".as_slice(), Some("img-1")));
    let edited = maker.make(&http, &catalog, request(vec![png(b"face")])).await.result.unwrap();
    assert_eq!(edited.bytes.as_ref(), b"edit");
    let refused = maker.make(&http, &catalog, request(Vec::new())).await.result.unwrap_err();
    assert_eq!(refused, MediaFailure::Refused { message: "input new_sensitive".into(), code: None, trace: VendorTrace { code: Some("1026".into()), id: None } });
    let key = maker.make(&http, &catalog, request(Vec::new())).await.result.unwrap_err();
    assert!(matches!(key, MediaFailure::KeyRefused { ref message, ref trace, .. } if message == "invalid api key" && trace.code.as_deref() == Some("2049")), "{key:?}");
    let used_up = maker.make(&http, &catalog, request(Vec::new())).await.result.unwrap_err();
    assert_eq!(used_up, MediaFailure::UsageLimit { retry_at: None, trace: VendorTrace { code: Some("1008".into()), id: None } });

    let sent = vendor.requests();
    assert_eq!(sent[0].uri.path(), at);
    assert_eq!(sent[0].header("authorization"), Some("Bearer mm-key"));
    assert_eq!(
        sent[0].json(),
        json!({ "model": "image-01", "prompt": "A lighthouse", "aspect_ratio": "16:9", "response_format": "base64", "n": 1 })
    );
    assert_eq!(
        sent[1].json()["subject_reference"],
        json!([{ "type": "character", "image_file": "data:image/png;base64,ZmFjZQ==" }])
    );
}

/// Planted defects this catches: a v2 ask without the resolution,
/// duration or ratio MiniMax needs, a ratio sent for a video from a first
/// frame, the resolution written other than as MiniMax does, an image in
/// the wrong role; a status read wrongly, `cancelled` as made; the video's
/// address not read, or the key sent to it; a v2 refusal, a status with an
/// `error`, read as other than a refused key or a used-up balance.
// One scripted vendor, ten requests: milliseconds.
#[tokio::test]
async fn a_v2_task_is_asked_with_what_it_needs_read_through_its_states_and_downloaded() {
    let vendor = MockVendor::start().await;
    let create = "/v2/video_generation";
    vendor.respond_at(create, json_answer(200, json!({ "task_id": "424010985738629" })));
    vendor.respond_at(create, json_answer(200, json!({ "task_id": "424010985738630" })));
    let refusal = |status: u16, kind: &str, message: &str| {
        json_answer(status, json!({ "type": "error", "error": { "type": kind, "message": message, "http_code": status.to_string() }, "request_id": "r" }))
    };
    vendor.respond_at(create, refusal(401, "authorized_error", "login fail (1004)"));
    vendor.respond_at(create, refusal(402, "insufficient_balance_error", "insufficient balance (1008)"));
    let query = "/v2/query/video_generation/424010985738629";
    let address = vendor.url("/inference_output/output.mp4");
    for task in [
        json!({ "status": "queued" }),
        json!({ "status": "running" }),
        json!({ "status": "succeeded", "content": { "url": address }, "resolution": "768P", "duration": 4 }),
        json!({ "status": "failed", "error": { "code": "2013", "message": "invalid params" } }),
        json!({ "status": "cancelled" }),
    ] {
        vendor.respond_at(query, json_answer(200, json!({ "task": task })));
    }
    vendor.respond_at("/inference_output/output.mp4", MockResponse::status(200).chunk("ftyp-video"));
    let provider = minimax(&vendor, Region::Global);
    let maker = provider.videos().expect("a MiniMax key makes videos");
    assert_eq!(maker.shape(), VideoShape::Hailuo);
    let (http, catalog) = (http(), catalog());

    let from_text = video("MiniMax-H3");
    assert_eq!(maker.ask(&http, &catalog, from_text).await.result.unwrap(), "424010985738629");
    let mut framed = VideoRequest {
        image: Some(png(b"first")),
        duration: Some(6),
        resolution: Some(VideoResolution::K(2)),
        seed: Some(7),
        ..video("MiniMax-H3")
    };
    assert_eq!(maker.unsent(&catalog, &framed), [VideoOption::Seed]);
    assert_eq!(maker.ask(&http, &catalog, framed.clone()).await.result.unwrap(), "424010985738630");
    framed.seed = None;
    let key = maker.ask(&http, &catalog, framed.clone()).await.result.unwrap_err();
    assert!(matches!(key, MediaFailure::KeyRefused { ref message, .. } if message == "login fail (1004)"), "{key:?}");
    assert_eq!(maker.ask(&http, &catalog, framed).await.result.unwrap_err(), MediaFailure::UsageLimit { retry_at: None, trace: VendorTrace { code: None, id: Some("r".into()) } });

    let mut states = Vec::new();
    for _ in 0..5 {
        states.push(maker.state(&http, job("424010985738629", "MiniMax-H3", &catalog)).await.result.unwrap());
    }
    let JobState::Done(done) = &states[2] else { panic!("{states:?}") };
    assert_eq!(done.url.as_str(), address);
    assert_eq!(states[..2], [JobState::Queued, JobState::Running { progress: None }]);
    assert_eq!(
        states[3..],
        [JobState::Failed { message: "invalid params".into(), trace: VendorTrace { code: Some("2013".into()), id: None } }, JobState::Failed { message: "the task was cancelled".into(), trace: Default::default() }]
    );
    assert_eq!(maker.download(&http, done).await.result.unwrap().as_ref(), b"ftyp-video");

    let sent = vendor.requests();
    assert_eq!(
        sent[0].json(),
        json!({ "model": "MiniMax-H3", "content": [{ "type": "text", "text": "Waves on a pier" }],
                "resolution": "768P", "duration": 4, "ratio": "16:9" })
    );
    assert_eq!(
        sent[1].json(),
        json!({ "model": "MiniMax-H3",
                "content": [
                    { "type": "text", "text": "Waves on a pier" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,Zmlyc3Q=" }, "role": "first_frame" }
                ],
                "resolution": "2K", "duration": 6 })
    );
    assert_eq!(sent[0].header("authorization"), Some("Bearer mm-key"));
    let download = sent.last().unwrap();
    assert_eq!((download.uri.path(), download.header("authorization")), ("/inference_output/output.mp4", None));
}

/// Planted defects this catches: a stopped v2 call that deletes a running
/// task, which MiniMax refuses, or a finished one, whose record it would
/// delete, or one that leaves a queued task to run and be paid for; a v1
/// task offered a cancellation MiniMax does not have.
// One scripted vendor, three requests: milliseconds.
#[tokio::test]
async fn a_stopped_call_cancels_a_v2_task_only_while_it_is_queued() {
    let vendor = MockVendor::start().await;
    let query = "/v2/query/video_generation/t-2";
    vendor.respond_at(query, json_answer(200, json!({ "task": { "status": "running" } })));
    vendor.respond_at(query, json_answer(200, json!({ "task": { "status": "queued" } })));
    vendor.respond_at("/v2/video_generation/t-2", json_answer(200, json!({ "task_id": "t-2", "action": "cancelled", "status": "cancelled" })));
    let provider = minimax(&vendor, Region::Global);
    let maker = provider.videos().unwrap();
    let (http, catalog) = (http(), catalog());

    assert!(maker.cancel(&http, job("t-1", "MiniMax-Hailuo-02", &catalog)).is_none(), "MiniMax cancels no v1 task");
    maker.cancel(&http, job("t-2", "MiniMax-H3", &catalog)).expect("MiniMax cancels a v2 task").await.result.unwrap();
    maker.cancel(&http, job("t-2", "MiniMax-H3", &catalog)).expect("MiniMax cancels a v2 task").await.result.unwrap();

    let methods: Vec<_> = vendor.requests().iter().map(|request| (request.method.as_str().to_owned(), request.uri.path().to_owned())).collect();
    assert_eq!(
        methods,
        [
            ("GET".to_owned(), query.to_owned()),
            ("GET".to_owned(), query.to_owned()),
            ("DELETE".to_owned(), "/v2/video_generation/t-2".to_owned())
        ]
    );
}

/// Planted defects this catches: a v1 ask that sends the frames other
/// than in their fields, or what v1 lacks, a reference or a shape, said
/// as sent; a v1 refusal on HTTP 200 read as a task; a status read
/// wrongly, `Preparing` as running; a finished task's file not retrieved
/// for its address; a model the catalog names no request for asked anyway.
// One scripted vendor, seven requests: milliseconds.
#[tokio::test]
async fn a_v1_task_is_read_and_its_file_retrieved_for_its_address() {
    let vendor = MockVendor::start().await;
    let create = "/v1/video_generation";
    vendor.respond_at(create, ok(json!({ "task_id": "106916112212032" })));
    vendor.respond_at(create, base_resp(2013, "invalid params, duration"));
    let query = "/v1/query/video_generation";
    for status in ["Preparing", "Queueing", "Processing"] {
        vendor.respond_at(query, ok(json!({ "task_id": "106916112212032", "status": status })));
    }
    vendor.respond_at(query, ok(json!({ "task_id": "106916112212032", "status": "Success", "file_id": "205258526306433" })));
    let address = vendor.url("/files/output_aigc.mp4");
    vendor.respond_at("/v1/files/retrieve", ok(json!({ "file": { "file_id": 205258526306433_i64, "download_url": address } })));
    let provider = minimax(&vendor, Region::China);
    let maker = provider.videos().unwrap();
    let (http, catalog) = (http(), catalog());
    let request = VideoRequest {
        image: Some(png(b"first")),
        last_frame: Some(png(b"last")),
        references: vec![png(b"product")],
        aspect: Some(Aspect { width: 16, height: 9 }),
        resolution: Some(VideoResolution::Height(1080)),
        ..video("MiniMax-Hailuo-02")
    };
    // The China entry's catalog is its own; these tests' lists Hailuo 02
    // under `minimax` alone, so the global entry asks.
    let provider_global = minimax(&vendor, Region::Global);
    let global = provider_global.videos().unwrap();
    assert_eq!(global.unsent(&catalog, &request), [VideoOption::Reference, VideoOption::Aspect]);
    assert_eq!(global.ask(&http, &catalog, request.clone()).await.result.unwrap(), "106916112212032");
    let refused = global.ask(&http, &catalog, request).await.result.unwrap_err();
    assert_eq!(refused, MediaFailure::Refused { message: "invalid params, duration".into(), code: None, trace: VendorTrace { code: Some("2013".into()), id: None } });
    let unnamed = maker.ask(&http, &catalog, video("MiniMax-Hailuo-02")).await.result.unwrap_err();
    assert!(matches!(unnamed, MediaFailure::Unanswered { .. }), "{unnamed:?}");

    let mut states = Vec::new();
    for _ in 0..4 {
        states.push(global.state(&http, job("106916112212032", "MiniMax-Hailuo-02", &catalog)).await.result.unwrap());
    }
    assert_eq!(states[..3], [JobState::Queued, JobState::Queued, JobState::Running { progress: None }]);
    let JobState::Done(done) = &states[3] else { panic!("{states:?}") };
    assert_eq!(done.url.as_str(), address);

    let sent = vendor.requests();
    assert_eq!(
        sent[0].json(),
        json!({ "model": "MiniMax-Hailuo-02", "prompt": "Waves on a pier",
                "first_frame_image": "data:image/png;base64,Zmlyc3Q=", "last_frame_image": "data:image/png;base64,bGFzdA==",
                "resolution": "1080P" })
    );
    let reads: Vec<_> = sent[2..].iter().map(|request| request.uri.to_string()).collect();
    assert!(reads[..4].iter().all(|read| read.ends_with("/v1/query/video_generation?task_id=106916112212032")), "{reads:?}");
    assert!(reads[4].ends_with("/v1/files/retrieve?file_id=205258526306433"), "{reads:?}");
}

/// Planted defects this catches: the audio read as other than the
/// hexadecimal text MiniMax answers, the voice or speed sent other than in
/// `voice_setting`, the format other than in `audio_setting`; a refusal on
/// HTTP 200 read as audio; instructions said as sent; voices of one kind
/// only listed, or the account's own taken for the system's.
// One scripted vendor, three requests: milliseconds.
#[tokio::test]
async fn speech_is_hex_audio_in_a_voice_get_voice_lists() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/t2a_v2", ok(json!({ "data": { "audio": "52494646", "status": 2 }, "trace_id": "tr-1" })));
    vendor.respond_at("/v1/t2a_v2", base_resp(1042, "invisible characters exceed 10%"));
    vendor.respond_at(
        "/v1/get_voice",
        ok(json!({
            "system_voice": [{ "voice_id": "English_Graceful_Lady", "voice_name": "Graceful Lady", "description": ["A graceful voice"], "created_time": "1970-01-01" }],
            "voice_cloning": [{ "voice_id": "test12345", "description": [], "created_time": "2025-08-20" }],
            "voice_generation": [{ "voice_id": "ttv-voice-1", "description": [], "created_time": "2025-08-20" }]
        })),
    );
    let provider = minimax(&vendor, Region::China);
    let maker = provider.audio().expect("a MiniMax key makes speech");
    assert_eq!((maker.vendor(), maker.shape(), maker.needs_voice()), (Some(MediaVendor::MinimaxCn), AudioShape::Minimax, true));
    let (http, catalog) = (http(), catalog());
    let request = AudioRequest {
        kind: AudioKind::Speech,
        model: "speech-2.8-hd".into(),
        text: "Hello".into(),
        voice: Some("English_Graceful_Lady".into()),
        instructions: Some("Whisper".into()),
        speed: Some(1.5),
        duration: None,
        instrumental: false,
        seed: None,
        looped: false,
        format: AudioFormatAsk::Wav,
    };
    assert_eq!(maker.unsent(&catalog, &request), [AudioOption::Instructions]);

    let made = maker.make(&http, &catalog, request.clone()).await.result.unwrap();
    assert_eq!((made.bytes.as_ref(), made.pcm), (b"RIFF".as_slice(), None));
    let refused = maker.make(&http, &catalog, request).await.result.unwrap_err();
    assert_eq!(refused, MediaFailure::Refused { message: "invisible characters exceed 10%".into(), code: None, trace: VendorTrace { code: Some("1042".into()), id: None } });
    let voices = maker.voices(&http, &catalog, "speech-2.8-hd", None).await.unwrap().expect("MiniMax lists its voices");
    let listed: Vec<_> = voices.iter().map(|voice| (voice.id.as_str(), voice.name.as_deref(), voice.own)).collect();
    assert_eq!(
        listed,
        [("English_Graceful_Lady", Some("Graceful Lady"), Some(false)), ("test12345", None, Some(true)), ("ttv-voice-1", None, Some(true))]
    );

    let sent = vendor.requests();
    assert_eq!(
        sent[0].json(),
        json!({ "model": "speech-2.8-hd", "text": "Hello", "stream": false,
                "voice_setting": { "voice_id": "English_Graceful_Lady", "speed": 1.5 },
                "audio_setting": { "format": "wav" }, "output_format": "hex" })
    );
    assert_eq!(sent[2].json(), json!({ "voice_type": "all" }));
}
