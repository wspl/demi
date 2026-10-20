//! An xAI key's videos and a compatible API's, by the shape its user chose
//! (`videos.md` § Vendors), against a scripted vendor built from each
//! shape's documented requests and answers: xAI's video generation guide
//! and its video models listing; OpenAI's Videos API reference, as
//! vLLM-Omni serves it, with its `DELETE`; OpenRouter's video generation
//! guide, whose ask answers 202 `pending`. No test calls a vendor.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Provider, Secret, VendorPolicy,
    images::{Aspect, InputImage},
    media::{MediaModelInfo, MediaVendor},
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor, RecordedRequest},
    videos::{Done, JobState, VideoOption, VideoRequest, unsent},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, compatible_videos};
use demi_shared_types::{Cost, Currency, VideoShape, WireApi};
use serde_json::{Value, json};

use crate::NOW;

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

fn image(bytes: &'static [u8]) -> InputImage {
    InputImage { media_type: "image/png", bytes: Bytes::from_static(bytes) }
}

/// A request with every option.
fn request(model: &str) -> VideoRequest {
    VideoRequest {
        model: model.into(),
        prompt: "a paper boat on a pond".into(),
        image: Some(image(b"first")),
        last_frame: Some(image(b"last")),
        references: vec![image(b"boat")],
        duration: Some(4),
        aspect: Some(Aspect { width: 16, height: 9 }),
        resolution: Some(demi_shared_types::VideoResolution::Height(480)),
        audio: Some(true),
        seed: Some(7),
    }
}

/// A compatible entry `Local` of `shape` at `/v1` on the scripted vendor,
/// whose key is `sk-test`, offering the model its user added.
fn compatible(vendor: &MockVendor, shape: VideoShape) -> Arc<dyn demi_provider_common::videos::VideoMaker> {
    let models = vec![MediaModelInfo { name: Some("Wan 2.2".into()), ..MediaModelInfo::id("wan-2.2") }];
    compatible_videos(
        "Local".into(),
        shape,
        Some(Secret::try_from("sk-test".to_owned()).unwrap()),
        &vendor.url("/v1").parse().unwrap(),
        models,
        Arc::new(FixedClock(NOW.parse().unwrap())),
    )
}

/// The text of a multipart field `name` of `request`.
fn field(request: &RecordedRequest, name: &str) -> Option<String> {
    let body = String::from_utf8_lossy(&request.body);
    let start = body.find(&format!("name=\"{name}\""))?;
    let rest = &body[start..];
    let value = &rest[rest.find("\r\n\r\n")? + 4..];
    Some(value[..value.find("\r\n")?].to_owned())
}

/// Planted defects this catches: the key's video models read from the
/// image listing; the key sent to the temporary address.
// One scripted vendor, four requests: milliseconds.
#[tokio::test]
async fn an_xai_key_lists_its_video_models_asks_in_the_xai_shape_and_downloads_without_the_key() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/video-generation-models", json_answer(200, json!({ "models": [
        { "id": "grok-imagine-video", "created": 1_769_904_000, "object": "model", "owned_by": "xai",
          "aliases": ["grok-imagine-video-latest"] }
    ] })));
    vendor.respond_at("/v1/videos/generations", json_answer(200, json!({ "request_id": "req-1" })));
    let address = vendor.url("/tmp/req-1.mp4");
    vendor.respond_at("/v1/videos/req-1", json_answer(200, json!({ "status": "done",
        "video": { "url": address, "duration": 4, "respect_moderation": true }, "model": "grok-imagine-video",
        "usage": { "cost_in_usd_ticks": 3_200_000_000_u64 } })));
    vendor.respond_at("/tmp/req-1.mp4", MockResponse::status(200).chunk(Bytes::from_static(b"mp4")));
    let config = OpenAiConfig {
        api_key: Some(Secret::try_from("sk-test".to_owned()).unwrap()),
        base_url: Some(vendor.url("/v1").parse().unwrap()),
        wire: WireApi::Responses,
        policy: VendorPolicy::default(),
        vendor_id: Some("xai".into()),
        makes_images: false,
        makes_videos: true,
        makes_audio: false,
    };
    let provider = OpenAiProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())));
    let maker = provider.videos().expect("an xAI key makes videos");
    assert_eq!((maker.name(), maker.vendor(), maker.shape()), ("xAI", Some(MediaVendor::Xai), VideoShape::XaiVideos));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = Catalog::decode(br#"{ "updated": "2026-10-01T00:00:00Z", "models": [] }"#).unwrap();

    let models = maker.models(&http, &catalog).await.unwrap();
    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["grok-imagine-video", "grok-imagine-video-latest"]);
    assert_eq!(unsent(maker.shape(), &request("grok-imagine-video")), [VideoOption::Seed]);
    let job = maker.ask(&http, empty_catalog(), request("grok-imagine-video")).await.result.unwrap();
    let JobState::Done(done) = maker.state(&http, taken(&job)).await.result.unwrap() else {
        panic!("the job is not done");
    };
    assert_eq!(done.cost, Some(Cost { amount: 0.32, currency: Currency::Usd }));
    maker.download(&http, &done).await.result.unwrap();

    let sent = vendor.requests();
    assert_eq!(sent[1].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(sent[1].json()["seed"], Value::Null);
    assert_eq!(sent[1].json()["reference_images"], json!([{ "url": "data:image/png;base64,Ym9hdA==" }]));
    assert_eq!((sent[3].uri.path(), sent[3].header("authorization")), ("/tmp/req-1.mp4", None));
}

/// Planted defects this catches: `seconds` sent as a number, which the
/// reference refuses; a size other than the aspect's at the resolution;
/// the content read without the key; no `DELETE` for a stopped job.
// One scripted vendor, six requests: milliseconds.
#[tokio::test]
async fn openai_videos_sends_a_form_reads_queued_to_completed_takes_its_content_with_the_key_and_cancels() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/videos", json_answer(200, json!({ "id": "video_1", "object": "video", "status": "queued" })));
    vendor.respond_at("/v1/videos/video_1", json_answer(200, json!({ "id": "video_1", "status": "queued", "progress": 0 })));
    vendor.respond_at("/v1/videos/video_1", json_answer(200, json!({ "id": "video_1", "status": "in_progress", "progress": 40 })));
    vendor.respond_at("/v1/videos/video_1", json_answer(200, json!({ "id": "video_1", "status": "completed", "progress": 100 })));
    vendor.respond_at("/v1/videos/video_1/content", MockResponse::status(200).chunk(Bytes::from_static(b"mp4")));
    vendor.respond_at("/v1/videos/video_1", json_answer(200, json!({ "id": "video_1", "deleted": true })));
    let maker = compatible(&vendor, VideoShape::OpenaiVideos);
    assert_eq!((maker.name(), maker.vendor()), ("Local", None));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    assert_eq!(
        unsent(maker.shape(), &request("wan-2.2")),
        [VideoOption::LastFrame, VideoOption::Reference, VideoOption::Audio, VideoOption::Seed]
    );
    let job = maker.ask(&http, empty_catalog(), request("wan-2.2")).await.result.unwrap();
    assert_eq!(job, "video_1");
    assert_eq!(maker.state(&http, taken(&job)).await.result.unwrap(), JobState::Queued);
    assert_eq!(maker.state(&http, taken(&job)).await.result.unwrap(), JobState::Running { progress: Some(40) });
    let JobState::Done(done) = maker.state(&http, taken(&job)).await.result.unwrap() else {
        panic!("the job is not done");
    };
    assert_eq!(done.url.as_str(), vendor.url("/v1/videos/video_1/content"));
    maker.download(&http, &done).await.result.unwrap();
    maker.cancel(&http, taken(&job)).expect("OpenAI Videos cancels").await.result.unwrap();

    let sent = vendor.requests();
    let ask = &sent[0];
    assert!(ask.header("content-type").unwrap().starts_with("multipart/form-data; boundary="));
    assert_eq!(ask.header("authorization"), Some("Bearer sk-test"));
    let fields: Vec<Option<String>> = ["model", "prompt", "seconds", "size"].iter().map(|name| field(ask, name)).collect();
    assert_eq!(
        fields,
        [Some("wan-2.2".into()), Some("a paper boat on a pond".into()), Some("4".into()), Some("848x480".into())]
    );
    assert!(String::from_utf8_lossy(&ask.body).contains("name=\"input_reference\"; filename=\"reference.png\""));
    assert_eq!((sent[4].uri.path(), sent[4].header("authorization")), ("/v1/videos/video_1/content", Some("Bearer sk-test")));
    assert_eq!((sent[5].method.as_str(), sent[5].uri.path()), ("DELETE", "/v1/videos/video_1"));
}

/// Planted defects this catches: the 202 of the ask read as a failure;
/// `pending` read as an unknown state; the unsigned address fetched
/// without the key, which OpenRouter refuses; a cost in another unit.
// One scripted vendor, five requests: milliseconds.
#[tokio::test]
async fn openrouter_videos_asks_with_frames_answers_202_pending_and_downloads_with_the_key() {
    let vendor = MockVendor::start().await;
    let polling = vendor.url("/v1/videos/job-1");
    vendor.respond_at("/v1/videos", json_answer(202, json!({ "id": "job-1", "polling_url": polling, "status": "pending" })));
    vendor.respond_at("/v1/videos/job-1", json_answer(200, json!({ "id": "job-1", "status": "pending" })));
    vendor.respond_at("/v1/videos/job-1", json_answer(200, json!({ "id": "job-1", "status": "in_progress" })));
    let content = vendor.url("/v1/videos/job-1/content?index=0");
    vendor.respond_at("/v1/videos/job-1", json_answer(200, json!({ "id": "job-1", "status": "completed",
        "unsigned_urls": [content], "usage": { "cost": 0.25, "is_byok": false } })));
    vendor.respond_at("/v1/videos/job-1/content", MockResponse::status(200).chunk(Bytes::from_static(b"mp4")));
    let maker = compatible(&vendor, VideoShape::OpenrouterVideos);
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    assert_eq!(unsent(maker.shape(), &request("wan-2.2")), []);
    let job = maker.ask(&http, empty_catalog(), request("wan-2.2")).await.result.unwrap();
    assert_eq!(job, "job-1");
    assert_eq!(maker.state(&http, taken(&job)).await.result.unwrap(), JobState::Queued);
    assert_eq!(maker.state(&http, taken(&job)).await.result.unwrap(), JobState::Running { progress: None });
    let JobState::Done(done) = maker.state(&http, taken(&job)).await.result.unwrap() else {
        panic!("the job is not done");
    };
    assert_eq!(done.cost, Some(Cost { amount: 0.25, currency: Currency::Usd }));
    maker.download(&http, &done).await.result.unwrap();
    assert!(maker.cancel(&http, taken(&job)).is_none(), "OpenRouter documents no cancellation");

    let sent = vendor.requests();
    assert_eq!(
        sent[0].json(),
        json!({ "model": "wan-2.2", "prompt": "a paper boat on a pond",
            "frame_images": [
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,Zmlyc3Q=" }, "frame_type": "first_frame" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,bGFzdA==" }, "frame_type": "last_frame" }
            ],
            "input_references": [{ "type": "image_url", "image_url": { "url": "data:image/png;base64,Ym9hdA==" } }],
            "duration": 4, "aspect_ratio": "16:9", "resolution": "480p", "generate_audio": true, "seed": 7 })
    );
    assert_eq!((sent[4].uri.path(), sent[4].header("authorization")), ("/v1/videos/job-1/content", Some("Bearer sk-test")));
}

/// A compatible entry's key goes only to its own endpoint's origin
/// (`videos.md` § The job): a content address on another host, and the
/// host a same-origin address redirects to, get the download without it.
/// Planted defect this catches: the key sent to whatever address the job
/// answers. Google's test catches the key kept across a redirect, which
/// reqwest's own redirects strip from `Authorization` but not from
/// Google's header.
// Two scripted servers, four requests: milliseconds.
#[tokio::test]
async fn an_openrouter_download_sends_the_key_only_to_the_entrys_own_origin() {
    let vendor = MockVendor::start().await;
    let elsewhere = MockVendor::start().await;
    elsewhere.respond_at("/bucket/a.mp4", MockResponse::status(200).chunk(Bytes::from_static(b"a")));
    vendor.respond_at("/v1/videos/job-1/content", MockResponse::status(302).header("location", &elsewhere.url("/bucket/b.mp4")));
    elsewhere.respond_at("/bucket/b.mp4", MockResponse::status(200).chunk(Bytes::from_static(b"b")));
    let maker = compatible(&vendor, VideoShape::OpenrouterVideos);
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let done = |url: String| Done { url: url.parse().unwrap(), cost: None, usage: None };

    let other = maker.download(&http, &done(elsewhere.url("/bucket/a.mp4"))).await.result.unwrap();
    let redirected = maker.download(&http, &done(vendor.url("/v1/videos/job-1/content"))).await.result.unwrap();

    assert_eq!((other, redirected), (Bytes::from_static(b"a"), Bytes::from_static(b"b")));
    assert_eq!(vendor.requests()[0].header("authorization"), Some("Bearer sk-test"));
    let away = elsewhere.requests();
    let keys: Vec<Option<&str>> = away.iter().map(|request| request.header("authorization")).collect();
    assert_eq!(keys, [None, None]);
}

/// A failed job is told in the vendor's words, as text or in an object.
// One scripted vendor, two requests: milliseconds.
#[tokio::test]
async fn a_failed_job_of_either_compatible_shape_is_told_in_the_vendors_words() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/videos/job-1", json_answer(200, json!({ "id": "job-1", "status": "failed", "error": "Content policy violation" })));
    vendor.respond_at("/v1/videos/video_1", json_answer(200, json!({ "id": "video_1", "status": "failed",
        "error": { "code": "video_generation_failed", "message": "The worker ran out of memory" } })));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    let openrouter = compatible(&vendor, VideoShape::OpenrouterVideos).state(&http, taken("job-1")).await.result.unwrap();
    assert_eq!(openrouter, JobState::Failed { message: "Content policy violation".into(), trace: Default::default() });
    let openai = compatible(&vendor, VideoShape::OpenaiVideos).state(&http, taken("video_1")).await.result.unwrap();
    let trace = demi_shared_types::VendorTrace { code: Some("video_generation_failed".into()), id: None };
    assert_eq!(openai, JobState::Failed { message: "The worker ran out of memory".into(), trace });
}

/// The device route of a test: it keeps each request and answers an
/// OpenAI Videos job's id.
struct Device(std::sync::Mutex<Vec<demi_provider_common::DeviceRequest>>);

impl demi_provider_common::DeviceRoute for Device {
    fn send(
        &self,
        _: &str,
        request: demi_provider_common::DeviceRequest,
    ) -> futures_util::future::BoxFuture<'static, demi_provider_common::Routed> {
        self.0.lock().unwrap().push(request);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let body = Bytes::from(json!({ "id": "video_1", "status": "queued" }).to_string());
        let answer = demi_provider_common::DeviceAnswer {
            status: reqwest::StatusCode::OK,
            headers,
            body: Box::pin(futures_util::stream::once(async move { Ok(body) })),
        };
        Box::pin(async move { demi_provider_common::Routed::Answered(answer) })
    }
}

/// A compatible server on a device's exposed port, such as vLLM-Omni on a
/// paired workstation, takes the OpenAI Videos form: the device route
/// carries only a whole body (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
/// Planted defect this catches: the form sent as a stream, which the route
/// refuses.
// No network: the route answers in memory.
#[tokio::test]
async fn an_openai_videos_ask_reaches_a_server_on_a_devices_exposed_port() {
    let device = Arc::new(Device(std::sync::Mutex::new(Vec::new())));
    let maker = compatible_videos(
        "Workstation".into(),
        VideoShape::OpenaiVideos,
        None,
        &"http://6f1c2a7e-3b9d-4c55-9a51-2d7f0e8b4c11:8091/v1".parse().unwrap(),
        Vec::new(),
        Arc::new(FixedClock(NOW.parse().unwrap())),
    );
    let http = demi_provider_common::ProviderHttp::with_devices(reqwest::Client::new(), device.clone(), "user-1");

    let job = maker.ask(&http, empty_catalog(), request("wan-2.2")).await.result.unwrap();

    assert_eq!(job, "video_1");
    let sent = device.0.lock().unwrap();
    assert_eq!((sent[0].method.as_str(), sent[0].path.as_str()), ("POST", "/v1/videos"));
    let body = String::from_utf8_lossy(sent[0].body.as_deref().expect("the form is sent whole")).into_owned();
    assert!(body.contains("name=\"model\"\r\n\r\nwan-2.2\r\n"));
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
