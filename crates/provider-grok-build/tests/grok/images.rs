//! Making images: the chat proxy's image endpoints with the account's identity.
//!
//! The tests cover the shape asked as `aspect_ratio`, the image always as
//! base64, an edit's images as data URLs, and a refused model and a used-up
//! account as the call's failures.

use demi_provider_common::media_catalog::Catalog;
use bytes::Bytes;
use demi_provider_common::{
    Provider,
    images::{Aspect, ImageRequest, InputImage},
    media::{MediaFailure, MediaVendor},
    testing::MockVendor,
};
use demi_shared_types::B64Bytes;
use serde_json::json;

use crate::{ACCOUNT, json_answer, pool_with, provider, secret};

/// The catalog the tests read: the models of a Grok subscription.
fn catalog() -> std::sync::Arc<Catalog> {
    let document = json!({ "updated": "2026-10-01T00:00:00Z", "models": [
        { "vendor": "xai", "id": "grok-imagine-image-2.0", "kind": "image", "name": "Imagine 2.0", "subscriptions": ["grok"] },
        { "vendor": "xai", "id": "grok-imagine-image", "kind": "image", "name": "Imagine", "subscriptions": ["grok"] }
    ] });
    std::sync::Arc::new(Catalog::decode(document.to_string().as_bytes()).unwrap())
}

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
    assert_eq!((maker.name(), maker.vendor(), maker.confirms_model()), ("Grok", Some(MediaVendor::Xai), true));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    let made = maker.make(&http, &catalog(), request("grok-imagine-image-2.0", Vec::new())).await.result.unwrap();
    assert_eq!(made.bytes, Bytes::from_static(b"jpeg image"));
    assert_eq!(made.usage, None);
    let image = InputImage { media_type: "image/webp", bytes: Bytes::from_static(b"webp bytes") };
    maker.make(&http, &catalog(), request("grok-imagine-image", vec![image])).await.result.unwrap();

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
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    match maker.make(&http, &catalog(), request("grok-imagine-nonexistent", Vec::new())).await.result.unwrap_err() {
        MediaFailure::Refused { message, .. } => assert_eq!(message, missing),
        other => panic!("not a refusal: {other:?}"),
    }
    let failure = maker.make(&http, &catalog(), request("grok-imagine-image", Vec::new())).await.result.unwrap_err();
    assert_eq!(failure, MediaFailure::UsageLimit { retry_at: None, trace: Default::default() });
}

/// A shape the proxy does not take fails with the shapes its refusal names, in
/// its order, so a shape xAI adds needs no change in Demi. The proxy's `auto`
/// is no shape and is left out.
///
/// Planted defect this catches: the refusal passed on as the proxy's raw
/// deserializer text, or a list of shapes of Demi's own.
// One scripted proxy, one request: milliseconds.
#[tokio::test]
async fn a_shape_the_proxy_does_not_take_fails_with_the_shapes_it_names() {
    let vendor = MockVendor::start().await;
    let refusal = "Failed to deserialize the JSON body into the target type: aspect_ratio: unknown variant `4:5`, expected one of `1:1`, `3:4`, `4:3`, `9:16`, `16:9`, `2:3`, `3:2`, `9:19.5`, `19.5:9`, `9:20`, `20:9`, `1:2`, `2:1`, `21:9`, `5:2`, `auto` at line 1 column 304";
    vendor.respond_at(GENERATIONS, json_answer(422, json!({ "code": "Client specified an invalid argument", "error": refusal })));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let maker = provider.images().unwrap();
    let asked = ImageRequest { aspect: Some(Aspect { width: 4, height: 5 }), ..request("grok-imagine-image", Vec::new()) };

    match maker.make(&demi_provider_common::ProviderHttp::new(reqwest::Client::new()), &catalog(), asked).await.result.unwrap_err() {
        MediaFailure::Shape { takes, .. } => assert_eq!(
            takes,
            ["1:1", "3:4", "4:3", "9:16", "16:9", "2:3", "3:2", "9:19.5", "19.5:9", "9:20", "20:9", "1:2", "2:1", "21:9", "5:2"]
        ),
        other => panic!("not a refused shape: {other:?}"),
    }
}

/// A list read whole is told as shapes, its last entry after `or` included. A
/// list with a part not understood is told in the proxy's own words rather than
/// as a shorter list.
///
/// Planted defect this catches: a list read up to the first part not
/// understood, which drops the shapes after it.
// One scripted proxy, two requests: milliseconds.
#[tokio::test]
async fn a_shape_list_not_read_whole_is_told_in_the_proxys_words() {
    let vendor = MockVendor::start().await;
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let maker = provider.images().unwrap();
    let asked = || ImageRequest { aspect: Some(Aspect { width: 4, height: 5 }), ..request("grok-imagine-image", Vec::new()) };
    let refusing = |refusal: &str| json_answer(422, json!({ "code": "Client specified an invalid argument", "error": refusal }));

    vendor.respond_at(GENERATIONS, refusing("aspect_ratio: unknown variant `4:5`, expected one of `1:1`, `3:4` or `16:9`"));
    match maker.make(&demi_provider_common::ProviderHttp::new(reqwest::Client::new()), &catalog(), asked()).await.result.unwrap_err() {
        MediaFailure::Shape { takes, .. } => assert_eq!(takes, ["1:1", "3:4", "16:9"]),
        other => panic!("not a refused shape: {other:?}"),
    }

    let odd = "aspect_ratio: unknown variant `4:5`, expected one of `1:1`, `3:4`, and wider ones";
    vendor.respond_at(GENERATIONS, refusing(odd));
    match maker.make(&demi_provider_common::ProviderHttp::new(reqwest::Client::new()), &catalog(), asked()).await.result.unwrap_err() {
        MediaFailure::Refused { message, .. } => assert!(message.contains(odd), "{message}"),
        other => panic!("not the proxy's own words: {other:?}"),
    }
}
