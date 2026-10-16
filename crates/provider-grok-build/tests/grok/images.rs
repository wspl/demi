//! Making images (`images.md` § Vendors): the chat proxy's image endpoints
//! with the account's identity, the shape asked as `aspect_ratio`, the
//! image always as base64, an edit's images as data URLs, and a refused
//! model and a used-up account as the call's failures.

use bytes::Bytes;
use demi_provider_common::{
    Provider,
    images::{Aspect, ImageFailure, ImageRequest, InputImage},
    testing::MockVendor,
};
use demi_shared_types::B64Bytes;
use serde_json::json;

use crate::{ACCOUNT, json_answer, pool_with, provider, secret};

const GENERATIONS: &str = "/v1/images/generations";
const EDITS: &str = "/v1/images/edits";

fn request(model: &str, images: Vec<InputImage>) -> ImageRequest {
    ImageRequest {
        model: model.into(),
        prompt: "a blue triangle".into(),
        aspect: Some(Aspect { width: 16, height: 9 }),
        transparent: false,
        images,
    }
}

// One scripted proxy, a few requests: milliseconds.
#[tokio::test]
async fn a_generation_and_an_edit_send_the_asked_shape_and_take_the_image_as_base64() {
    let vendor = MockVendor::start().await;
    let answer = json!({ "data": [{ "b64_json": B64Bytes::from(&b"jpeg image"[..]), "mime_type": "image/jpeg" }],
        "usage": { "cost_in_usd_ticks": 200_000_000 } });
    vendor.respond_at(GENERATIONS, json_answer(200, answer.clone()));
    vendor.respond_at(EDITS, json_answer(200, answer));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let maker = provider.images().expect("Grok Build makes images");
    assert_eq!(maker.vendor(), "Grok");
    assert_eq!(maker.models()[0], "grok-imagine-image");
    let http = reqwest::Client::new();

    let made = maker.make(&http, request("grok-imagine-image-2.0", Vec::new())).await.result.unwrap();
    assert_eq!(made.bytes, Bytes::from_static(b"jpeg image"));
    assert_eq!(made.usage, None);
    let image = InputImage { media_type: "image/webp", bytes: Bytes::from_static(b"webp bytes") };
    maker.make(&http, request("grok-imagine-image", vec![image])).await.result.unwrap();

    let sent = vendor.requests();
    assert_eq!(sent[0].uri.path(), GENERATIONS);
    assert_eq!(sent[0].header("authorization"), Some("Bearer session-token"));
    assert_eq!(
        sent[0].json(),
        json!({ "prompt": "a blue triangle", "model": "grok-imagine-image-2.0", "aspect_ratio": "16:9",
            "response_format": "b64_json" })
    );
    assert_eq!(sent[1].uri.path(), EDITS);
    assert_eq!(
        sent[1].json()["images"],
        json!([{ "url": "data:image/webp;base64,d2VicCBieXRlcw==", "type": "image_url" }])
    );
}

#[tokio::test]
async fn a_model_the_proxy_does_not_have_is_refused_in_its_words_and_a_402_uses_the_account_up() {
    let vendor = MockVendor::start().await;
    let missing = "The model grok-imagine-nonexistent does not exist or your team t-1 does not have access to it.";
    vendor.respond_at(GENERATIONS, json_answer(404, json!({ "code": "not-found", "error": missing })));
    vendor.respond_at(GENERATIONS, json_answer(402, json!({ "error": "credits" })));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let maker = provider.images().unwrap();
    let http = reqwest::Client::new();

    match maker.make(&http, request("grok-imagine-nonexistent", Vec::new())).await.result.unwrap_err() {
        ImageFailure::Refused { message, .. } => assert_eq!(message, missing),
        other => panic!("not a refusal: {other:?}"),
    }
    let failure = maker.make(&http, request("grok-imagine-image", Vec::new())).await.result.unwrap_err();
    assert_eq!(failure, ImageFailure::UsageLimit { retry_at: None });
}
