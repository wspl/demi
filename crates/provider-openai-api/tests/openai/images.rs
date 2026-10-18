//! An OpenAI or xAI key's images (`images.md` § Vendors) against a scripted
//! vendor built from each vendor's documented shapes: OpenAI's Images API
//! reference (developers.openai.com/api/reference/resources/images) and
//! its error codes guide, and xAI's Imagine guide
//! (docs.x.ai/developers/model-capabilities/imagine), its image generation
//! models listing and its debugging guide's status codes. No test calls a
//! vendor.

use demi_provider_common::media_catalog::Catalog;
use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Provider, Secret, VendorPolicy,
    images::{Aspect, ImageRequest, InputImage},
    media::{MediaFailure, MediaModelInfo, MediaVendor},
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::{B64Bytes, TokenUsage, WireApi};
use serde_json::{Value, json};

use crate::NOW;

/// The catalog the tests read: what it states of the models the scripted
/// vendors list.
fn catalog() -> std::sync::Arc<Catalog> {
    let document = json!({ "updated": "2026-10-01T00:00:00Z", "models": [
        { "vendor": "openai", "id": "gpt-image-2", "kind": "image", "name": "GPT Image 2",
          "description": "State-of-the-art image generation model", "released": "2026-04-21" },
        { "vendor": "openai", "id": "gpt-image-1", "kind": "image", "name": "GPT Image 1",
          "image": { "sizes": ["1024x1024", "1536x1024", "1024x1536"] } },
        { "vendor": "xai", "id": "grok-imagine-image-2.0", "kind": "image", "name": "Imagine 2.0",
          "description": "Precise image generation and editing, built for real creative work", "released": "2026-08-07" }
    ] });
    std::sync::Arc::new(Catalog::decode(document.to_string().as_bytes()).unwrap())
}

const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n made image";

/// An entry of `vendor` whose key is `sk-test`, at `/v1` on the scripted
/// vendor.
fn provider(vendor: &MockVendor, images: Option<MediaVendor>) -> OpenAiProvider {
    // An entry of neither vendor, a compatible API, makes no images.
    let config = OpenAiConfig {
        api_key: Some(Secret::try_from("sk-test".to_owned()).unwrap()),
        base_url: Some(vendor.url("/v1").parse().unwrap()),
        wire: WireApi::Responses,
        policy: VendorPolicy::default(),
        vendor_id: images.map(|vendor| vendor.id().to_owned()),
        makes_images: images.is_some(),
        makes_videos: false,
        makes_audio: false,
    };
    OpenAiProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

fn request(model: &str, aspect: Option<Aspect>, images: Vec<InputImage>) -> ImageRequest {
    ImageRequest {
        model: model.into(),
        prompt: "a red fox head".into(),
        aspect,
        transparent: true,
        images,
    }
}

fn image(media_type: &'static str, bytes: &'static [u8]) -> InputImage {
    InputImage { media_type, bytes: Bytes::from_static(bytes) }
}

fn ids(models: &[MediaModelInfo]) -> Vec<(&str, Option<&str>, &str)> {
    models
        .iter()
        .map(|model| (model.id.as_str(), model.released.as_deref(), model.description.as_deref().unwrap_or_default()))
        .collect()
}

// One scripted vendor, three requests: milliseconds.
//
// Planted defect: the sizes a request is sent at read from anywhere but the
// catalog the call is given, such as the release's copy.
#[tokio::test]
async fn the_size_sent_for_a_shape_follows_the_sizes_the_catalog_gives_the_model() {
    let vendor = MockVendor::start().await;
    let answer = json!({ "created": 1_790_000_000, "data": [{ "b64_json": B64Bytes::from(IMAGE) }] });
    for _ in 0..3 {
        vendor.respond_at("/v1/images/generations", json_answer(200, answer.clone()));
    }
    let provider = provider(&vendor, Some(MediaVendor::OpenAi));
    let maker = provider.images().unwrap();
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let with_sizes = |image: Value| {
        let document = json!({ "updated": "2026-10-01T00:00:00Z", "models": [
            { "vendor": "openai", "id": "picture", "kind": "image", "name": "Picture", "image": image }
        ] });
        Catalog::decode(document.to_string().as_bytes()).unwrap()
    };
    let wide = || request("picture", Some(Aspect { width: 16, height: 9 }), Vec::new());
    for (image, sent) in [
        (json!({ "sizes": ["1024x1024", "1536x1024", "1024x1536"] }), "1536x1024"),
        (json!({ "sizes": ["1024x1024", "1792x1024"] }), "1792x1024"),
        (json!({ "edits": true }), "1536x864"),
    ] {
        maker.make(&http, &with_sizes(image), wide()).await.result.unwrap();
        assert_eq!(vendor.requests().last().unwrap().json()["size"], sent);
    }
}

// One scripted vendor, a few requests: milliseconds.
//
// Planted defects: a compatible entry, or one of another vendor, that
// makes images; the listing's filter dropped, so a text model is listed;
// a size, background or format left out of the body.
#[tokio::test]
async fn an_openai_key_lists_its_image_models_and_sends_generations_and_edits_as_documented() {
    let vendor = MockVendor::start().await;
    assert!(provider(&vendor, None).images().is_none());
    let listing = json!({ "object": "list", "data": [
        { "id": "gpt-5.5", "object": "model", "created": 1_770_000_000, "owned_by": "system" },
        { "id": "gpt-image-2", "object": "model", "created": 1_776_729_600, "owned_by": "system" },
        { "id": "gpt-image-3-preview", "object": "model", "created": 1_790_000_000, "owned_by": "system" },
        { "id": "dall-e-3", "object": "model", "created": 1_698_785_189, "owned_by": "system" }
    ] });
    vendor.respond_at("/v1/models", json_answer(200, listing));
    let answer = json!({ "created": 1_790_000_000, "background": "transparent", "output_format": "png",
        "data": [{ "b64_json": B64Bytes::from(IMAGE) }],
        "usage": { "input_tokens": 50, "output_tokens": 1_056, "total_tokens": 1_106,
            "input_tokens_details": { "image_tokens": 40, "text_tokens": 10 } } });
    for _ in 0..4 {
        vendor.respond_at("/v1/images/generations", json_answer(200, answer.clone()));
    }
    vendor.respond_at("/v1/images/edits", json_answer(200, answer));
    let provider = provider(&vendor, Some(MediaVendor::OpenAi));
    let maker = provider.images().expect("an OpenAI key makes images");
    assert_eq!((maker.name(), maker.vendor(), maker.confirms_model()), ("OpenAI", Some(MediaVendor::OpenAi), true));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    let models = maker.models(&http, &catalog()).await.unwrap();
    assert_eq!(ids(&models), [("gpt-image-2", Some("2026-04-21"), ""), ("gpt-image-3-preview", Some("2026-09-21"), "")]);

    let made = maker
        .make(&http, &catalog(), request("gpt-image-2", Some(Aspect { width: 16, height: 9 }), Vec::new()))
        .await
        .result
        .unwrap();
    assert_eq!(made.bytes, Bytes::from_static(IMAGE));
    assert_eq!(
        made.usage,
        Some(TokenUsage { input_tokens: 50, output_tokens: 1_056, cache_read_tokens: 0, cache_write_tokens: 0 })
    );
    let edit = vec![image("image/png", b"png bytes"), image("image/webp", b"webp bytes")];
    maker.make(&http, &catalog(), request("gpt-image-2", None, edit)).await.result.unwrap();
    // A model that takes only three sizes is sent the one nearest the
    // shape, never a size of the shape it would refuse.
    for (aspect, sent) in [((1, 1), "1024x1024"), ((16, 9), "1536x1024"), ((2, 3), "1024x1536")] {
        let shape = Some(Aspect { width: aspect.0, height: aspect.1 });
        maker.make(&http, &catalog(), request("gpt-image-1", shape, Vec::new())).await.result.unwrap();
        assert_eq!(vendor.requests().last().unwrap().json()["size"], sent);
    }

    let sent = vendor.requests();
    assert_eq!(sent[0].uri.path(), "/v1/models");
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(sent[1].uri.path(), "/v1/images/generations");
    assert_eq!(sent[1].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(
        sent[1].json(),
        json!({ "prompt": "a red fox head", "model": "gpt-image-2", "size": "1536x864", "background": "transparent",
            "output_format": "png", "quality": "auto" })
    );
    assert_eq!(sent[2].uri.path(), "/v1/images/edits");
    assert_eq!(
        sent[2].json(),
        json!({ "prompt": "a red fox head", "model": "gpt-image-2", "size": "auto", "background": "transparent",
            "output_format": "png", "quality": "auto", "images": [
                { "image_url": "data:image/png;base64,cG5nIGJ5dGVz" },
                { "image_url": "data:image/webp;base64,d2VicCBieXRlcw==" }
            ] })
    );
}

// Milliseconds.
//
// Planted defects: a spent credit read as a rate limit, so the call does
// not go on with another entry; a refused key told as an expired sign-in;
// a policy block told without OpenAI's words.
#[tokio::test]
async fn an_openai_keys_failures_read_as_a_spent_limit_a_refused_key_a_policy_block_or_the_vendors_words() {
    let vendor = MockVendor::start().await;
    let error = |message: &str, kind: &str, code: Value| json!({ "error": { "message": message, "type": kind, "param": null, "code": code } });
    let generations = "/v1/images/generations";
    vendor.respond_at(
        generations,
        json_answer(429, error("You exceeded your current quota, please check your plan and billing details.", "insufficient_quota", json!("insufficient_quota"))),
    );
    vendor.respond_at(
        generations,
        json_answer(400, error("Billing hard limit has been reached", "invalid_request_error", json!("billing_hard_limit_reached"))),
    );
    vendor.respond_at(
        generations,
        json_answer(401, error("Incorrect API key provided: sk-test. You can find your API key at https://platform.openai.com/account/api-keys.", "invalid_request_error", json!("invalid_api_key"))),
    );
    vendor.respond_at(
        generations,
        json_answer(400, error("Your request was rejected as a result of our safety system.", "image_generation_user_error", json!("moderation_blocked"))),
    );
    vendor.respond_at(generations, json_answer(400, error("Invalid size '9x9'.", "invalid_request_error", Value::Null)));
    vendor.respond_at(generations, json_answer(429, error("Rate limit reached for gpt-image-2.", "requests", Value::Null)));
    vendor.respond_at("/v1/models", json_answer(401, error("Incorrect API key provided.", "invalid_request_error", json!("invalid_api_key"))));
    let provider = provider(&vendor, Some(MediaVendor::OpenAi));
    let maker = provider.images().unwrap();
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let fail = async || maker.make(&http, &catalog(), request("gpt-image-2", None, Vec::new())).await.result.unwrap_err();

    let quota = demi_shared_types::VendorTrace { code: Some("insufficient_quota".into()), id: None };
    assert_eq!(fail().await, MediaFailure::UsageLimit { retry_at: None, trace: quota });
    let billing = demi_shared_types::VendorTrace { code: Some("billing_hard_limit_reached".into()), id: None };
    assert_eq!(fail().await, MediaFailure::UsageLimit { retry_at: None, trace: billing });
    let refused = fail().await;
    assert!(matches!(&refused, MediaFailure::KeyRefused { message, .. } if message.starts_with("Incorrect API key provided")), "{refused:?}");
    assert_eq!(refused.outcome(), "auth_expired");
    match fail().await {
        MediaFailure::Refused { message, .. } => assert_eq!(message, "Your request was rejected as a result of our safety system."),
        other => panic!("not a refusal: {other:?}"),
    }
    match fail().await {
        MediaFailure::Refused { message, .. } => assert_eq!(message, "Invalid size '9x9'."),
        other => panic!("not a refusal: {other:?}"),
    }
    match fail().await {
        MediaFailure::Unanswered { cause, .. } => assert_eq!(cause, "HTTP 429: Rate limit reached for gpt-image-2."),
        other => panic!("not unanswered: {other:?}"),
    }
    // A key refused while listing is the same failure.
    assert!(matches!(maker.models(&http, &catalog()).await, Err(MediaFailure::KeyRefused { .. })));
    // A key is never retried: each refusal was one request.
    assert_eq!(vendor.requests().len(), 7);
}

// Milliseconds.
//
// Planted defects: an alias left out of the listing; one image sent as
// `images` rather than `image`; xAI's refusal of a shape told without its
// shapes; a spent credit or a refused key read as a plain refusal.
#[tokio::test]
async fn an_xai_key_lists_models_with_their_aliases_sends_its_shapes_and_reads_its_failures() {
    let vendor = MockVendor::start().await;
    let listing = json!({ "models": [
        { "id": "grok-imagine-image-2.0", "fingerprint": "fp_1", "max_prompt_length": 1024, "created": 1_786_406_400,
          "object": "model", "owned_by": "xai", "version": "2.0", "input_modalities": ["text", "image"],
          "output_modalities": ["image"], "image_price": 200_000_000, "aliases": ["grok-imagine-image-2.0-latest"] }
    ] });
    vendor.respond_at("/v1/image-generation-models", json_answer(200, listing));
    let answer = json!({ "data": [{ "b64_json": B64Bytes::from(&b"jpeg image"[..]), "mime_type": "image/jpeg", "revised_prompt": "" }] });
    vendor.respond_at("/v1/images/generations", json_answer(200, answer.clone()));
    vendor.respond_at("/v1/images/edits", json_answer(200, answer.clone()));
    vendor.respond_at("/v1/images/edits", json_answer(200, answer));
    let shapes = "Failed to deserialize the JSON body into the target type: aspect_ratio: unknown variant `4:5`, expected one of `1:1`, `3:4`, `16:9`, `auto` at line 1 column 80";
    vendor.respond_at("/v1/images/generations", json_answer(422, json!({ "code": "Client specified an invalid argument", "error": shapes })));
    vendor.respond_at(
        "/v1/images/generations",
        json_answer(429, json!({ "code": "Some resource has been exhausted", "error": "Your team has either used all available credits or reached its monthly spending limit." })),
    );
    vendor.respond_at(
        "/v1/images/generations",
        json_answer(400, json!({ "code": "Client specified an invalid argument", "error": "Incorrect API key provided: xa***st. You can obtain an API key from https://console.x.ai." })),
    );
    vendor.respond_at(
        "/v1/images/generations",
        json_answer(400, json!({ "code": "Client specified an invalid argument", "error": "Generated image rejected by content moderation." })),
    );
    let provider = provider(&vendor, Some(MediaVendor::Xai));
    let maker = provider.images().expect("an xAI key makes images");
    assert_eq!((maker.name(), maker.vendor()), ("xAI", Some(MediaVendor::Xai)));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    let models = maker.models(&http, &catalog()).await.unwrap();
    assert_eq!(
        ids(&models),
        [
            ("grok-imagine-image-2.0", Some("2026-08-11"), ""),
            ("grok-imagine-image-2.0-latest", Some("2026-08-11"), "Alias of grok-imagine-image-2.0"),
        ]
    );
    let wide = Some(Aspect { width: 16, height: 9 });
    let made = maker.make(&http, &catalog(), request("grok-imagine-image-2.0", wide, Vec::new())).await.result.unwrap();
    assert_eq!(made.bytes, Bytes::from_static(b"jpeg image"));
    maker.make(&http, &catalog(), request("grok-imagine-image-2.0", None, vec![image("image/png", b"png bytes")])).await.result.unwrap();
    let two = vec![image("image/png", b"png bytes"), image("image/jpeg", b"jpeg bytes")];
    maker.make(&http, &catalog(), request("grok-imagine-image-2.0", None, two)).await.result.unwrap();

    let sent = vendor.requests();
    assert_eq!((sent[0].uri.path(), sent[0].header("authorization")), ("/v1/image-generation-models", Some("Bearer sk-test")));
    assert_eq!(
        sent[1].json(),
        json!({ "prompt": "a red fox head", "model": "grok-imagine-image-2.0", "aspect_ratio": "16:9", "response_format": "b64_json" })
    );
    assert_eq!(
        sent[2].json(),
        json!({ "prompt": "a red fox head", "model": "grok-imagine-image-2.0", "response_format": "b64_json",
            "image": { "url": "data:image/png;base64,cG5nIGJ5dGVz", "type": "image_url" } })
    );
    assert_eq!(
        sent[3].json()["images"],
        json!([
            { "url": "data:image/png;base64,cG5nIGJ5dGVz", "type": "image_url" },
            { "url": "data:image/jpeg;base64,anBlZyBieXRlcw==", "type": "image_url" }
        ])
    );

    let fail = async || maker.make(&http, &catalog(), request("grok-imagine-image-2.0", None, Vec::new())).await.result.unwrap_err();
    assert!(matches!(fail().await, MediaFailure::Shape { takes, .. } if takes == ["1:1", "3:4", "16:9"]));
    assert_eq!(fail().await, MediaFailure::UsageLimit { retry_at: None, trace: Default::default() });
    assert!(matches!(fail().await, MediaFailure::KeyRefused { .. }));
    match fail().await {
        MediaFailure::Refused { message, .. } => assert_eq!(message, "Generated image rejected by content moderation."),
        other => panic!("not a refusal: {other:?}"),
    }
}
