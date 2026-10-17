//! Making videos (`videos.md` § Vendors, the Grok row): the chat proxy's
//! video endpoints with the account's identity, every option of the
//! xAI Videos shape sent as its field and the seed, which it has none for,
//! left out; the job read through a 202 `pending` with its progress to
//! `done` with its cost in ticks, or to `failed` and `expired`; and the
//! video's temporary address downloaded without the account's token.

use bytes::Bytes;
use demi_provider_common::{
    Provider,
    images::{Aspect, InputImage},
    media::MediaVendor,
    media_catalog::Catalog,
    testing::{MockResponse, MockVendor},
    videos::{JobState, Resolution, VideoOption, VideoRequest, unsent},
};
use demi_shared_types::{Cost, Currency, VideoShape};
use serde_json::json;

use crate::{ACCOUNT, json_answer, pool_with, provider, secret};

const ASK: &str = "/v1/videos/generations";
const JOB: &str = "/v1/videos/d97415a1";

fn request() -> VideoRequest {
    let image = |bytes: &'static [u8]| InputImage { media_type: "image/png", bytes: Bytes::from_static(bytes) };
    VideoRequest {
        model: "grok-imagine-video".into(),
        prompt: "a blue triangle turning".into(),
        image: Some(image(b"first")),
        last_frame: Some(image(b"last")),
        references: vec![image(b"cat")],
        duration: Some(1),
        aspect: Some(Aspect { width: 16, height: 9 }),
        resolution: Some(Resolution::P480),
        audio: Some(false),
        seed: Some(7),
    }
}

/// Planted defects this catches: the seed sent, which the proxy refuses as
/// an unknown field; a 202 read as a failure; the cost read as dollars
/// rather than ticks; the account's token sent to the video's address.
// One scripted proxy, four requests: milliseconds.
#[tokio::test]
async fn a_video_is_asked_in_the_xai_shape_read_to_done_and_downloaded_without_the_token() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(ASK, json_answer(200, json!({ "request_id": "d97415a1" })));
    vendor.respond_at(JOB, json_answer(202, json!({ "status": "pending", "progress": 40 })));
    let address = vendor.url("/files/clip.mp4");
    vendor.respond_at(JOB, json_answer(200, json!({ "status": "done", "progress": 100,
        "video": { "url": address, "duration": 1, "respect_moderation": true },
        "usage": { "cost_in_usd_ticks": 500_000_000 } })));
    vendor.respond_at("/files/clip.mp4", MockResponse::status(200).chunk(Bytes::from_static(b"mp4 bytes")));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let maker = provider.videos().expect("Grok Build makes videos");
    assert_eq!((maker.name(), maker.vendor(), maker.shape()), ("Grok", Some(MediaVendor::Xai), VideoShape::Grok));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    assert_eq!(unsent(maker.shape(), &request()), [VideoOption::Seed]);
    let job = maker.ask(&http, request()).await.result.unwrap();
    assert_eq!(job, "d97415a1");
    assert_eq!(maker.state(&http, &job).await.result.unwrap(), JobState::Running { progress: Some(40) });
    let JobState::Done(done) = maker.state(&http, &job).await.result.unwrap() else {
        panic!("the job is not done");
    };
    assert_eq!(done.cost, Some(Cost { amount: 0.05, currency: Currency::Usd }));
    assert_eq!(done.url.as_str(), address);
    assert_eq!(maker.download(&http, &done).await.result.unwrap(), Bytes::from_static(b"mp4 bytes"));

    let sent = vendor.requests();
    assert_eq!(sent[0].uri.path(), ASK);
    assert_eq!(sent[0].header("authorization"), Some("Bearer session-token"));
    assert_eq!(
        sent[0].json(),
        json!({ "model": "grok-imagine-video", "prompt": "a blue triangle turning",
            "image": { "url": "data:image/png;base64,Zmlyc3Q=" }, "last_frame": { "url": "data:image/png;base64,bGFzdA==" },
            "reference_images": [{ "url": "data:image/png;base64,Y2F0" }],
            "duration": 1, "aspect_ratio": "16:9", "resolution": "480p", "generate_audio": false })
    );
    assert_eq!((sent[1].method.as_str(), sent[1].uri.path()), ("GET", JOB));
    assert_eq!(sent[1].header("authorization"), Some("Bearer session-token"));
    assert_eq!(sent[3].uri.path(), "/files/clip.mp4");
    assert_eq!(sent[3].header("authorization"), None);
}

/// A job xAI failed is told in its words, one it withheld for moderation
/// says so, and one that expired is no failure of the read.
/// Planted defect this catches: `failed` or `expired` read as an unknown
/// state, which would fail the call as the vendor's unreadable answer.
// One scripted proxy, three requests: milliseconds.
#[tokio::test]
async fn a_failed_a_withheld_and_an_expired_job_are_read_as_such() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(JOB, json_answer(200, json!({ "status": "failed",
        "error": { "code": "internal_error", "message": "Video generation failed" } })));
    vendor.respond_at(JOB, json_answer(200, json!({ "status": "done", "video": { "url": "", "respect_moderation": false } })));
    vendor.respond_at(JOB, json_answer(200, json!({ "status": "expired" })));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let maker = provider.videos().unwrap();
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    let read = || maker.state(&http, "d97415a1");
    assert_eq!(read().await.result.unwrap(), JobState::Failed { message: "Video generation failed".into() });
    assert_eq!(
        read().await.result.unwrap(),
        JobState::Failed { message: "the video was withheld by its moderation".into() }
    );
    assert_eq!(read().await.result.unwrap(), JobState::Expired);
}

/// The proxy lists no video models: a subscription's are those the
/// catalog lists for Grok, and no model of another kind or subscription.
// No request: milliseconds.
#[tokio::test]
async fn the_video_models_are_the_catalogs_for_grok() {
    let vendor = MockVendor::start().await;
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let catalog = Catalog::decode(
        json!({ "updated": "2026-10-01T00:00:00Z", "models": [
            { "vendor": "xai", "id": "grok-imagine-video", "kind": "video", "name": "Imagine Video", "subscriptions": ["grok"] },
            { "vendor": "xai", "id": "grok-imagine-image", "kind": "image", "name": "Imagine", "subscriptions": ["grok"] },
            { "vendor": "openai", "id": "sora-2", "kind": "video", "name": "Sora 2", "subscriptions": ["chatgpt"] }
        ] })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let listed = provider.videos().unwrap().models(&demi_provider_common::ProviderHttp::new(reqwest::Client::new()), &catalog).await.unwrap();
    assert_eq!(listed.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["grok-imagine-video"]);
}
