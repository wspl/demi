//! MiniMax's media through its key, against scripted vendors built from its API
//! reference at platform.minimax.io/docs/api-reference.
//!
//! The reference pages are Text to image and Image to image, Create, Query and
//! Cancel or delete a v2 video task, Create a v1 text-to-video task, Query a v1
//! task and Retrieve file, Synchronous text to speech and Get voice. No test
//! calls a vendor.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Capabilities, Provider, ProviderHttp, ProviderRuntime, RuntimeEnv, RuntimeError, Secret,
    read_http_failure,
    images::{Aspect, InputImage},
    media::MediaFailure,
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
    videos::{JobRef, JobState, VideoOption, VideoRequest},
};
use demi_provider_minimax::{MinimaxConfig, MinimaxProvider, Region};
use demi_shared_types::{VendorTrace, AuthState, ProviderErrorDiagnostics, ProviderFailureFacts, RuntimeState, Timestamp, VideoResolution, VideoShape,
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

/// Planted defects this catches:
///
/// - a v2 ask without the resolution, duration or ratio MiniMax needs, a ratio
///   sent for a video from a first frame, the resolution written other than as
///   MiniMax does, an image in the wrong role;
/// - a status read wrongly, `cancelled` as made;
/// - the video's address not read, or the key sent to it;
/// - a v2 refusal, a status with an `error`, read as other than a refused key
///   or a used-up balance.
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
    assert!(matches!(key, MediaFailure::KeyRefused { .. }), "{key:?}");
    assert_eq!(maker.ask(&http, &catalog, framed).await.result.unwrap_err(), MediaFailure::UsageLimit { retry_at: None, trace: VendorTrace { code: None, id: Some("r".into()) } });

    let mut states = Vec::new();
    for _ in 0..5 {
        states.push(maker.state(&http, job("424010985738629", "MiniMax-H3", &catalog)).await.result.unwrap());
    }
    let JobState::Done(done) = &states[2] else { panic!("{states:?}") };
    assert_eq!(done.url.as_str(), address);
    assert_eq!(states[..2], [JobState::Queued, JobState::Running { progress: None }]);
    assert!(matches!(&states[3], JobState::Failed { trace, .. } if trace.code.as_deref() == Some("2013")), "{states:?}");
    assert!(matches!(&states[4], JobState::Failed { .. }), "{states:?}");
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

/// Planted defects this catches:
///
/// - a v1 ask that sends the frames other than in their fields, or what v1
///   lacks, a reference or a shape, said as sent;
/// - a v1 refusal on HTTP 200 read as a task;
/// - a status read wrongly, `Preparing` as running;
/// - a finished task's file not retrieved for its address;
/// - a model the catalog names no request for asked anyway.
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
    assert!(matches!(&refused, MediaFailure::Refused { code: None, trace, .. } if trace.code.as_deref() == Some("2013")), "{refused:?}");
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
