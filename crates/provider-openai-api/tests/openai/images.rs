//! An OpenAI or xAI key's images against a scripted vendor built from each
//! vendor's documented shapes:
//!
//! - OpenAI's Images API reference
//!   (developers.openai.com/api/reference/resources/images) and its error codes
//!   guide;
//! - xAI's Imagine guide (docs.x.ai/developers/model-capabilities/imagine), its
//!   image generation models listing and its debugging guide's status codes.
//!
//! No test calls a vendor.

use demi_provider_common::media_catalog::Catalog;
use std::sync::Arc;

use demi_provider_common::{
    Provider, Secret, VendorPolicy,
    images::{Aspect, ImageRequest, InputImage},
    media::MediaVendor,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::{B64Bytes, WireApi};
use serde_json::{Value, json};

use crate::NOW;

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
