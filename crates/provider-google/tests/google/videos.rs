//! A Gemini API key's videos (`videos.md` § Vendors, the Veo row) against a
//! scripted vendor built from Google's Veo guide
//! (ai.google.dev/gemini-api/docs/video): the models that list
//! `predictLongRunning`, the ask with its `instances` and `parameters`, the
//! operation read until `done`, the video downloaded with the key from the
//! key's endpoint and without it through the redirect to storage Google
//! answers, and a failed or filtered operation told in
//! Google's words. No test calls a vendor.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Provider, Secret,
    images::{Aspect, InputImage},
    media::MediaVendor,
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
    videos::{JobState, Resolution, VideoOption, VideoRequest, unsent},
};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_shared_types::VideoShape;
use serde_json::{Value, json};

use crate::NOW;

const OPERATION: &str = "models/veo-3.1-generate-preview/operations/op-1";

fn provider(vendor: &MockVendor) -> GoogleProvider {
    let config = GoogleConfig {
        api_key: Some(Secret::try_from("google-key".to_owned()).unwrap()),
        base_url: Some(vendor.url("/v1beta").parse().unwrap()),
        makes_images: false,
        makes_videos: true,
    };
    GoogleProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

fn image(bytes: &'static [u8]) -> InputImage {
    InputImage { media_type: "image/png", bytes: Bytes::from_static(bytes) }
}

/// Planted defects this catches: a model without `predictLongRunning`
/// listed; `durationSeconds` or the frames sent under the image API's
/// names; an operation not yet `done` read as a failure; the download sent
/// without the key or not following Google's redirect; the key sent on to
/// the storage host the redirect names.
// Two scripted servers, seven requests: milliseconds.
#[tokio::test]
async fn veo_lists_its_models_asks_with_instances_and_parameters_and_downloads_with_the_key() {
    let vendor = MockVendor::start().await;
    let storage = MockVendor::start().await;
    vendor.respond_at("/v1beta/models", json_answer(200, json!({ "models": [
        { "name": "models/veo-3.1-generate-preview", "displayName": "Veo 3.1", "supportedGenerationMethods": ["predictLongRunning"] },
        { "name": "models/gemini-3-flash", "displayName": "Gemini 3 Flash", "supportedGenerationMethods": ["generateContent"] }
    ] })));
    vendor.respond_at(
        "/v1beta/models/veo-3.1-generate-preview:predictLongRunning",
        json_answer(200, json!({ "name": OPERATION })),
    );
    vendor.respond_at(&format!("/v1beta/{OPERATION}"), json_answer(200, json!({ "name": OPERATION })));
    let uri = vendor.url("/v1beta/files/abc:download?alt=media");
    vendor.respond_at(&format!("/v1beta/{OPERATION}"), json_answer(200, json!({ "name": OPERATION, "done": true,
        "response": { "@type": "type.googleapis.com/google.ai.generativelanguage.v1beta.PredictLongRunningResponse",
            "generateVideoResponse": { "generatedSamples": [{ "video": { "uri": uri } }] } } })));
    vendor.respond_at(
        "/v1beta/files/abc:download",
        MockResponse::status(302).header("location", &storage.url("/stored/abc.mp4")),
    );
    storage.respond_at("/stored/abc.mp4", MockResponse::status(200).chunk(Bytes::from_static(b"mp4")));
    let provider = provider(&vendor);
    let maker = provider.videos().expect("a Gemini key makes videos");
    assert_eq!((maker.name(), maker.vendor(), maker.shape()), ("Google", Some(MediaVendor::Google), VideoShape::Veo));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = Catalog::decode(br#"{ "updated": "2026-10-01T00:00:00Z", "models": [] }"#).unwrap();

    let models = maker.models(&http, &catalog).await.unwrap();
    assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["veo-3.1-generate-preview"]);
    let request = VideoRequest {
        model: "veo-3.1-generate-preview".into(),
        prompt: "a lighthouse at dusk".into(),
        image: Some(image(b"first")),
        last_frame: Some(image(b"last")),
        references: vec![image(b"tower")],
        duration: Some(8),
        aspect: Some(Aspect { width: 9, height: 16 }),
        resolution: Some(Resolution::P720),
        audio: Some(false),
        seed: Some(7),
    };
    assert_eq!(unsent(maker.shape(), &request), [VideoOption::Audio]);
    let job = maker.ask(&http, request).await.result.unwrap();
    assert_eq!(job, OPERATION);
    assert_eq!(maker.state(&http, &job).await.result.unwrap(), JobState::Running { progress: None });
    let JobState::Done(done) = maker.state(&http, &job).await.result.unwrap() else {
        panic!("the operation is not done");
    };
    assert_eq!(done.cost, None);
    assert_eq!(maker.download(&http, &done).await.result.unwrap(), Bytes::from_static(b"mp4"));

    let sent = vendor.requests();
    assert_eq!(sent[1].header("x-goog-api-key"), Some("google-key"));
    let inline = |data: &str| json!({ "inlineData": { "mimeType": "image/png", "data": data } });
    assert_eq!(
        sent[1].json(),
        json!({ "instances": [{ "prompt": "a lighthouse at dusk", "image": inline("Zmlyc3Q="), "lastFrame": inline("bGFzdA=="),
                "referenceImages": [{ "image": inline("dG93ZXI="), "referenceType": "asset" }] }],
            "parameters": { "aspectRatio": "9:16", "durationSeconds": 8, "resolution": "720p", "seed": 7 } })
    );
    assert_eq!((sent[2].method.as_str(), sent[2].uri.path()), ("GET", format!("/v1beta/{OPERATION}").as_str()));
    assert_eq!(sent[4].header("x-goog-api-key"), Some("google-key"));
    let stored = storage.requests();
    assert_eq!((stored[0].uri.path(), stored[0].header("x-goog-api-key")), ("/stored/abc.mp4", None));
}

/// An operation that failed, or whose video Google's filters withheld, is
/// told in Google's words.
// One scripted vendor, two requests: milliseconds.
#[tokio::test]
async fn a_failed_or_filtered_operation_is_told_in_googles_words() {
    let vendor = MockVendor::start().await;
    let path = format!("/v1beta/{OPERATION}");
    vendor.respond_at(&path, json_answer(200, json!({ "name": OPERATION, "done": true,
        "error": { "code": 13, "message": "Video generation failed due to an internal error." } })));
    vendor.respond_at(&path, json_answer(200, json!({ "name": OPERATION, "done": true,
        "response": { "generateVideoResponse": { "raiMediaFilteredCount": 1,
            "raiMediaFilteredReasons": ["The prompt could not be submitted."] } } })));
    let provider = provider(&vendor);
    let maker = provider.videos().unwrap();
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    assert_eq!(
        maker.state(&http, OPERATION).await.result.unwrap(),
        JobState::Failed { message: "Video generation failed due to an internal error.".into() }
    );
    assert_eq!(
        maker.state(&http, OPERATION).await.result.unwrap(),
        JobState::Failed { message: "The prompt could not be submitted.".into() }
    );
}
