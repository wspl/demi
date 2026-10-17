//! Making images (`images.md` § Vendors): the Codex backend's image
//! endpoints with the account's headers, the asked options sent as asked,
//! an edit's images as data URLs, the image and its counts read from the
//! answer, and the failures a call ends with.

use demi_provider_common::media_catalog::Catalog;
use bytes::Bytes;
use demi_provider_codex::TransportMode;
use demi_provider_common::{
    Provider,
    images::{Aspect, ImageFailure, ImageRequest, ImageVendor, InputImage},
    testing::{MockResponse, MockVendor},
};
use demi_shared_types::{B64Bytes, TokenUsage};
use serde_json::{Value, json};

use crate::{NOW, NOW_SECONDS, fresh_token, pool_with, provider, secret};

/// The catalog the tests read: two models of a ChatGPT subscription, one
/// only a key makes, a video model and another subscription's.
fn catalog() -> std::sync::Arc<Catalog> {
    let document = json!({ "updated": "2026-10-01T00:00:00Z", "models": [
        { "vendor": "openai", "id": "gpt-image-2", "kind": "image", "name": "GPT Image 2",
          "subscriptions": ["chatgpt"] },
        { "vendor": "openai", "id": "gpt-image-1", "kind": "image", "name": "GPT Image 1",
          "subscriptions": ["chatgpt"], "image": { "sizes": ["1024x1024", "1536x1024", "1024x1536"] } },
        { "vendor": "openai", "id": "gpt-image-0", "kind": "image", "name": "GPT Image 0" },
        { "vendor": "openai", "id": "sora-3", "kind": "video", "name": "Sora 3", "subscriptions": ["chatgpt"] },
        { "vendor": "xai", "id": "grok-imagine-image", "kind": "image", "name": "Imagine", "subscriptions": ["grok"] }
    ] });
    std::sync::Arc::new(Catalog::decode(document.to_string().as_bytes()).unwrap())
}

const GENERATIONS: &str = "/backend-api/codex/images/generations";
const EDITS: &str = "/backend-api/codex/images/edits";

/// The bytes the scripted backend answers with; only the provider's
/// caller reads them as an image.
const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n made image";

fn answer() -> MockResponse {
    let body = json!({
        "data": [{ "b64_json": B64Bytes::from(IMAGE), "generation_id": "gen_1" }],
        "background": "transparent", "output_format": "png", "size": "1254x1254",
        "usage": { "input_tokens": 25, "output_tokens": 515, "total_tokens": 540,
            "input_tokens_details": { "image_tokens": 0, "text_tokens": 25 } }
    });
    json(200, body)
}

fn json(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

fn request(images: Vec<InputImage>) -> ImageRequest {
    ImageRequest {
        model: "gpt-image-2".into(),
        prompt: "a red fox head".into(),
        aspect: None,
        transparent: false,
        images,
    }
}

// One scripted backend, a few requests: milliseconds.
#[tokio::test]
async fn a_generation_sends_the_asked_options_with_the_accounts_headers_and_reads_the_image_and_its_counts() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(GENERATIONS, answer());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let maker = provider.images().expect("Codex makes images");
    assert_eq!((maker.name(), maker.vendor(), maker.confirms_model()), ("ChatGPT", Some(ImageVendor::OpenAi), false));
    // The catalog's image models of a ChatGPT subscription, in its order.
    let models = maker.models(&reqwest::Client::new(), &catalog()).await.unwrap();
    let offered: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(offered, ["gpt-image-2", "gpt-image-1"]);

    let asked = ImageRequest {
        aspect: Some(Aspect { width: 16, height: 9 }),
        transparent: true,
        ..request(Vec::new())
    };
    let made = maker.make(&reqwest::Client::new(), &catalog(), asked).await.result.unwrap();
    assert_eq!(made.bytes, Bytes::from_static(IMAGE));
    assert_eq!(made.vendor_id.as_deref(), Some("gen_1"));
    assert_eq!(
        made.usage,
        Some(TokenUsage { input_tokens: 25, output_tokens: 515, cache_read_tokens: 0, cache_write_tokens: 0 })
    );
    let sent = &vendor.requests()[0];
    assert_eq!(sent.uri.path(), GENERATIONS);
    assert_eq!(sent.header("authorization"), Some(format!("Bearer {}", fresh_token()).as_str()));
    assert_eq!(sent.header("chatgpt-account-id"), Some("acct-1"));
    assert_eq!(
        sent.json(),
        json!({ "prompt": "a red fox head", "model": "gpt-image-2", "background": "transparent",
            "quality": "auto", "size": "1536x864" })
    );
}

#[tokio::test]
async fn an_edit_sends_each_image_as_a_data_url() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(EDITS, answer());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let images = vec![
        InputImage { media_type: "image/png", bytes: Bytes::from_static(b"png bytes") },
        InputImage { media_type: "image/jpeg", bytes: Bytes::from_static(b"jpeg bytes") },
    ];
    provider.images().unwrap().make(&reqwest::Client::new(), &catalog(), request(images)).await.result.unwrap();
    let sent = &vendor.requests()[0];
    assert_eq!(sent.uri.path(), EDITS);
    assert_eq!(
        sent.json(),
        json!({ "prompt": "a red fox head", "model": "gpt-image-2", "background": "opaque",
            "quality": "auto", "size": "auto", "images": [
                { "image_url": "data:image/png;base64,cG5nIGJ5dGVz" },
                { "image_url": "data:image/jpeg;base64,anBlZyBieXRlcw==" }
            ] })
    );
}

/// A used-up account says when it lifts; a refusal is the vendor's words;
/// a failing backend did not answer; a refused token is renewed once and
/// the request sent again.
#[tokio::test]
async fn a_failed_call_says_whether_the_account_is_used_up_the_vendor_refused_or_did_not_answer() {
    let vendor = MockVendor::start().await;
    let limit = json!({ "error": { "type": "usage_limit_reached", "message": "limit", "resets_at": NOW_SECONDS + 3_600 } });
    vendor.respond_at(GENERATIONS, json(429, limit));
    let refusal = json!({ "error": { "message": "Your request was rejected by the safety system.", "code": "moderation_blocked" } });
    vendor.respond_at(GENERATIONS, json(400, refusal));
    vendor.respond_at(GENERATIONS, MockResponse::status(503).chunk("upstream connect error"));
    vendor.respond_at(GENERATIONS, json(401, json!({ "error": { "message": "expired" } })));
    let renewed = json!({ "access_token": "new-access", "refresh_token": "refresh-2", "id_token": crate::secret_id_token() });
    vendor.respond_at("/oauth/token", json(200, renewed));
    vendor.respond_at(GENERATIONS, answer());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let maker = provider.images().unwrap();
    let http = reqwest::Client::new();

    let failure = maker.make(&http, &catalog(), request(Vec::new())).await.result.unwrap_err();
    let lifts = demi_shared_types::Timestamp::from_millisecond((NOW_SECONDS + 3_600) * 1000).unwrap();
    assert_eq!(failure, ImageFailure::UsageLimit { retry_at: Some(lifts) });
    assert_eq!(failure.outcome(), "usage_limit");
    match maker.make(&http, &catalog(), request(Vec::new())).await.result.unwrap_err() {
        ImageFailure::Refused { message, .. } => {
            assert_eq!(message, "Your request was rejected by the safety system.");
        }
        other => panic!("not a refusal: {other:?}"),
    }
    match maker.make(&http, &catalog(), request(Vec::new())).await.result.unwrap_err() {
        ImageFailure::Unanswered { cause, .. } => assert_eq!(cause, "HTTP 503: upstream connect error"),
        other => panic!("not unanswered: {other:?}"),
    }
    maker.make(&http, &catalog(), request(Vec::new())).await.result.unwrap();
    let sent: Vec<_> = vendor.requests().into_iter().filter(|sent| sent.uri.path() == GENERATIONS).collect();
    assert_eq!(sent.last().unwrap().header("authorization"), Some("Bearer new-access"));
}

/// The windows an image answer reports, a refusal's too, update the
/// account's snapshot as a turn's do, and the call answers them for its
/// ledger row.
/// Planted defect this catches: a refusal's headers left unread.
#[tokio::test]
async fn the_quota_an_image_answer_or_refusal_reports_updates_the_accounts_snapshot() {
    let vendor = MockVendor::start().await;
    let refusal = json(400, json!({ "error": { "message": "no" } }))
        .header("x-codex-primary-used-percent", "42")
        .header("x-codex-primary-window-minutes", "300");
    vendor.respond_at(GENERATIONS, refusal);
    vendor.respond_at(
        GENERATIONS,
        answer()
            .header("x-codex-primary-used-percent", "43")
            .header("x-codex-primary-window-minutes", "300"),
    );
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let maker = provider.images().unwrap();
    let used = || {
        let snapshot = provider.quota().unwrap().latest().unwrap();
        snapshot.windows.iter().map(|window| window.used_percent).collect::<Vec<_>>()
    };

    let refused = maker.make(&reqwest::Client::new(), &catalog(), request(Vec::new())).await;
    assert!(refused.result.is_err());
    assert_eq!(refused.observed.len(), 1);
    assert_eq!(used(), [Some(42.0)]);
    let answered = maker.make(&reqwest::Client::new(), &catalog(), request(Vec::new())).await;
    assert!(answered.result.is_ok());
    assert_eq!(used(), [Some(43.0)]);
}

/// The deadline bounds the whole call: a refusal whose body never ends
/// fails when it passes. The clock is paused, so the 120 s pass at once.
/// Planted defect this catches: a refusal's body read without the
/// deadline, which hangs the call.
#[tokio::test(start_paused = true)]
async fn a_refusal_whose_body_never_ends_fails_at_the_deadline() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(GENERATIONS, MockResponse::status(503).chunk("upstream").stay_open());
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let started = tokio::time::Instant::now();
    let http = reqwest::Client::new();
    let catalog = catalog();
    let call = provider.images().unwrap().make(&http, &catalog, request(Vec::new()));
    // Well past the deadline, so a call that ignores it fails here.
    let answer = tokio::time::timeout(std::time::Duration::from_secs(600), call)
        .await
        .expect("the call ends at its deadline");
    match answer.result.unwrap_err() {
        ImageFailure::Unanswered { cause, .. } => assert_eq!(cause, "no answer in 120 s"),
        other => panic!("not unanswered: {other:?}"),
    }
    assert!(started.elapsed() >= std::time::Duration::from_secs(120));
}
