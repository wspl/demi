//! Grok Build's image role (`images.md` § Vendors): the chat proxy's image
//! endpoints, which Grok's own CLI calls, with the account's identity as
//! inference sends it. The image comes back as base64: an image the proxy
//! answers by URL refuses an immediate download.

use bytes::Bytes;
use demi_provider_common::{
    images::{ImageAnswer, ImageCall, ImageMaker, ImageRequest},
    json_body, read_http_failure,
};
use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::{GrokProvider, LABEL, request::identity_headers, run::with_usage_limit};

/// The image models, the default first: xAI's published IDs the
/// subscription accepts (`images.md` § Vendors); the proxy refuses a model
/// it does not have.
const MODELS: &[&str] = &[
    "grok-imagine-image",
    "grok-imagine-image-2026-03-02",
    "grok-imagine-image-2.0",
    "grok-imagine-image-quality",
    "grok-imagine-image-quality-20260403",
    "grok-imagine-image-quality-latest",
    "grok-imagine-image-pro",
];

#[derive(Serialize)]
struct Body<'a> {
    prompt: &'a str,
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    response_format: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<Image>,
}

#[derive(Serialize)]
struct Image {
    url: String,
    #[serde(rename = "type")]
    kind: &'static str,
}

fn encode(request: &ImageRequest) -> Bytes {
    let body = Body {
        prompt: &request.prompt,
        model: &request.model,
        aspect_ratio: request.aspect.map(|aspect| aspect.to_string()),
        response_format: "b64_json",
        images: request
            .images
            .iter()
            .map(|image| Image {
                url: image.data_url(),
                kind: "image_url",
            })
            .collect(),
    };
    Bytes::from(json_body(&body))
}

impl ImageMaker for GrokProvider {
    fn vendor(&self) -> &'static str {
        "Grok"
    }

    fn models(&self) -> &'static [&'static str] {
        MODELS
    }

    fn make<'a>(
        &'a self,
        http: &'a reqwest::Client,
        request: ImageRequest,
    ) -> BoxFuture<'a, ImageAnswer> {
        Box::pin(async move {
            let shared = &self.shared;
            let url = if request.images.is_empty() {
                &shared.image_generations_url
            } else {
                &shared.image_edits_url
            };
            let call = ImageCall {
                label: LABEL,
                family: crate::auth::FAMILY,
                reader: read_http_failure,
                used_up: with_usage_limit,
                clock: &*shared.clock,
                quota: &shared.quota,
            };
            call.send(http, url, encode(&request), |refused| async move {
                let credentials = shared.auth.credentials(http, refused.as_ref()).await?;
                Ok((identity_headers(&credentials), credentials.access_token))
            })
            .await
        })
    }
}
