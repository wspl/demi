//! An xAI key's videos and a compatible API's, by the shape its user chose,
//! against a scripted vendor built from each shape's documented requests and
//! answers:
//!
//! - xAI's video generation guide and its video models listing;
//! - OpenAI's Videos API reference, as vLLM-Omni serves it, with its `DELETE`;
//! - OpenRouter's video generation guide, whose ask answers 202 `pending`.
//!
//! No test calls a vendor.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret,
    images::{Aspect, InputImage},
    media::MediaModelInfo,
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor, RecordedRequest},
    videos::{Done, JobState, VideoOption, VideoRequest, unsent},
};
use demi_provider_openai_api::compatible_videos;
use demi_shared_types::VideoShape;
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

/// A compatible entry's key goes only to its own endpoint's origin: a content
/// address on another host, and the host a same-origin address redirects to,
/// get the download without it.
///
/// Planted defect this catches: the key sent to whatever address the job
/// answers. Google's test catches the key kept across a redirect, which
/// reqwest's own redirects strip from `Authorization` but not from Google's
/// header.
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
