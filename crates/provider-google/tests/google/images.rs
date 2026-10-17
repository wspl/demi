//! A Gemini API key's images (`images.md` § Vendors) against a scripted
//! vendor built from Google's documented shapes: the image generation
//! guide (ai.google.dev/gemini-api/docs/image-generation), the
//! `models.list` and `generateContent` references
//! (ai.google.dev/api/models, ai.google.dev/api/generate-content) and the
//! troubleshooting guide's errors (ai.google.dev/gemini-api/docs/troubleshooting).
//! No test calls a vendor.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Provider, Secret,
    images::{Aspect, ImageFailure, ImageRequest, ImageVendor, InputImage},
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_shared_types::{B64Bytes, TokenUsage};
use serde_json::{Value, json};

use crate::NOW;

fn provider(vendor: &MockVendor, makes_images: bool) -> GoogleProvider {
    let config = GoogleConfig {
        api_key: Some(Secret::try_from("google-key".to_owned()).unwrap()),
        base_url: Some(vendor.url("/v1beta").parse().unwrap()),
        makes_images,
    };
    GoogleProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

fn model(name: &str, methods: &[&str]) -> Value {
    json!({ "name": format!("models/{name}"), "version": "001", "displayName": name,
        "description": format!("What {name} is"), "inputTokenLimit": 65_536, "outputTokenLimit": 32_768,
        "supportedGenerationMethods": methods })
}

fn request(aspect: Option<Aspect>, images: Vec<InputImage>) -> ImageRequest {
    ImageRequest {
        model: "gemini-nano-banana-2.1".into(),
        prompt: "a red fox head".into(),
        aspect,
        transparent: false,
        images,
    }
}

const GENERATE: &str = "/v1beta/models/gemini-nano-banana-2.1:generateContent";

// Milliseconds.
//
// Planted defects: a second page left unread; a model whose ID has the
// word `banana` but not `image` left out, or one that cannot generate
// content listed; the ratio sent when none was asked; an edit's images
// not sent as inline data; the image looked for in the wrong part.
#[tokio::test]
async fn a_gemini_key_lists_every_page_and_sends_generations_and_edits_as_documented() {
    let vendor = MockVendor::start().await;
    assert!(provider(&vendor, false).images().is_none());
    vendor.respond_at(
        "/v1beta/models",
        json_answer(200, json!({ "models": [
            model("gemini-3-pro", &["generateContent", "countTokens"]),
            model("gemini-nano-banana-2.1", &["generateContent"]),
            model("imagen-4.0-generate-001", &["predict"]),
        ], "nextPageToken": "page-2" })),
    );
    vendor.respond_at(
        "/v1beta/models",
        json_answer(200, json!({ "models": [model("gemini-4-flash-image", &["generateContent", "countTokens"])] })),
    );
    let answer = json!({
        "candidates": [{ "content": { "role": "model", "parts": [
            { "text": "Here is your fox." },
            { "inlineData": { "mimeType": "image/png", "data": B64Bytes::from(&b"png image"[..]) } }
        ] }, "finishReason": "STOP", "index": 0 }],
        "usageMetadata": { "promptTokenCount": 12, "candidatesTokenCount": 1_290, "totalTokenCount": 1_302 },
        "modelVersion": "gemini-nano-banana-2.1", "responseId": "resp-1"
    });
    vendor.respond_at(GENERATE, json_answer(200, answer.clone()));
    vendor.respond_at(GENERATE, json_answer(200, answer));
    let provider = provider(&vendor, true);
    let maker = provider.images().expect("a Gemini key makes images");
    assert_eq!((maker.name(), maker.vendor(), maker.confirms_model()), ("Google", ImageVendor::Google, true));
    let http = reqwest::Client::new();

    let models = maker.models(&http).await.unwrap();
    let listed: Vec<(&str, Option<&str>, &str)> = models
        .iter()
        .map(|model| (model.id.as_str(), model.released.as_deref(), model.description.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("gemini-nano-banana-2.1", None, "Nano Banana 2.1: the high-efficiency workhorse for image generation and conversational editing"),
            ("gemini-4-flash-image", None, "What gemini-4-flash-image is"),
        ]
    );

    let made = maker
        .make(&http, request(Some(Aspect { width: 16, height: 9 }), Vec::new()))
        .await
        .result
        .unwrap();
    assert_eq!(made.bytes, Bytes::from_static(b"png image"));
    assert_eq!(made.vendor_id.as_deref(), Some("resp-1"));
    assert_eq!(
        made.usage,
        Some(TokenUsage { input_tokens: 12, output_tokens: 1_290, cache_read_tokens: 0, cache_write_tokens: 0 })
    );
    let edit = vec![InputImage { media_type: "image/jpeg", bytes: Bytes::from_static(b"jpeg bytes") }];
    maker.make(&http, request(None, edit)).await.result.unwrap();

    let sent = vendor.requests();
    assert_eq!(sent[0].uri.query(), Some("pageSize=1000"));
    assert_eq!(sent[1].uri.query(), Some("pageSize=1000&pageToken=page-2"));
    assert_eq!(sent[0].header("x-goog-api-key"), Some("google-key"));
    assert_eq!(sent[2].uri.path(), GENERATE);
    assert_eq!(sent[2].header("x-goog-api-key"), Some("google-key"));
    assert_eq!(
        sent[2].json(),
        json!({ "contents": [{ "parts": [{ "text": "a red fox head" }] }],
            "generationConfig": { "responseModalities": ["IMAGE"], "imageConfig": { "aspectRatio": "16:9" } } })
    );
    assert_eq!(
        sent[3].json(),
        json!({ "contents": [{ "parts": [
                { "text": "a red fox head" },
                { "inline_data": { "mime_type": "image/jpeg", "data": "anBlZyBieXRlcw==" } }
            ] }],
            "generationConfig": { "responseModalities": ["IMAGE"] } })
    );
}

// Milliseconds.
//
// Planted defects: a blocked prompt or answer read as an unreadable one,
// without Google's reason; a day's quota read as a rate limit, so the call
// does not go on with another entry; a minute's read as a usage limit; a
// key Google does not accept told as a refusal of the request.
#[tokio::test]
async fn a_gemini_keys_failures_read_as_a_block_a_spent_quota_a_refused_key_or_googles_words() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(GENERATE, json_answer(200, json!({ "promptFeedback": { "blockReason": "PROHIBITED_CONTENT" } })));
    vendor.respond_at(
        GENERATE,
        json_answer(200, json!({ "candidates": [{ "finishReason": "IMAGE_SAFETY", "finishMessage": "Unable to show the generated image.", "index": 0 }] })),
    );
    let quota = |id: &str, value: &str| {
        json!({ "error": { "code": 429, "message": "You exceeded your current quota, please check your plan and billing details.",
            "status": "RESOURCE_EXHAUSTED", "details": [
                { "@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": [
                    { "quotaMetric": "generativelanguage.googleapis.com/generate_content_requests", "quotaId": id,
                      "quotaDimensions": { "location": "global", "model": "gemini-nano-banana-2.1" }, "quotaValue": value } ] },
                { "@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "41s" } ] } })
    };
    vendor.respond_at(GENERATE, json_answer(429, quota("GenerateRequestsPerDayPerProjectPerModel", "250")));
    vendor.respond_at(GENERATE, json_answer(429, quota("GenerateRequestsPerMinutePerProjectPerModel", "10")));
    let invalid = json!({ "error": { "code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT",
        "details": [{ "@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "API_KEY_INVALID", "domain": "googleapis.com" }] } });
    vendor.respond_at(GENERATE, json_answer(400, invalid));
    vendor.respond_at(
        GENERATE,
        json_answer(400, json!({ "error": { "code": 400, "message": "Unsupported aspect ratio: 4:5", "status": "INVALID_ARGUMENT" } })),
    );
    let provider = provider(&vendor, true);
    let maker = provider.images().unwrap();
    let http = reqwest::Client::new();
    let fail = async || maker.make(&http, request(None, Vec::new())).await.result.unwrap_err();

    match fail().await {
        ImageFailure::Refused { message, .. } => assert_eq!(message, "the prompt was blocked (PROHIBITED_CONTENT)"),
        other => panic!("not a refusal: {other:?}"),
    }
    match fail().await {
        ImageFailure::Refused { message, .. } => assert_eq!(message, "Unable to show the generated image. (IMAGE_SAFETY)"),
        other => panic!("not a refusal: {other:?}"),
    }
    assert!(matches!(fail().await, ImageFailure::UsageLimit { .. }));
    assert!(matches!(fail().await, ImageFailure::Unanswered { .. }));
    assert!(matches!(fail().await, ImageFailure::KeyRefused { message } if message == "API key not valid. Please pass a valid API key."));
    match fail().await {
        ImageFailure::Refused { message, .. } => assert_eq!(message, "Unsupported aspect ratio: 4:5"),
        other => panic!("not a refusal: {other:?}"),
    }
}
