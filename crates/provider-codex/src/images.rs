//! Codex's image role (`images.md` § Vendors): the Codex backend's image
//! endpoints, which the Codex CLI's own image tool calls, with the
//! account's headers as inference sends them. The backend ignores the size
//! today and may honour it later, so the size of the asked shape is sent.

use bytes::Bytes;
use demi_provider_common::{
    images::{ImageAnswer, ImageCall, ImageMaker, ImageRequest},
    json_body,
};
use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::{CodexProvider, LABEL, failure::{read_codex_failure, with_usage_limit}, request::account_headers};

/// The image models, the default first: the one the Codex CLI asks for,
/// then OpenAI's published IDs, newest first (`images.md` § Vendors).
const MODELS: &[&str] = &[
    "gpt-image-2",
    "gpt-image-2-2026-04-21",
    "gpt-image-2.5-sunburst",
    "gpt-image-2.5-sunburst-2026-09-08",
    "gpt-image-2.5-flare",
    "gpt-image-2.5-flare-2026-09-08",
    "gpt-image-1.5",
    "gpt-image-1.5-2025-12-16",
    "gpt-image-1",
    "gpt-image-1-mini",
    "chatgpt-image-latest",
];

/// The longer side of the size an asked shape is sent as.
const LONG_SIDE: u32 = 1536;

#[derive(Serialize)]
struct Body<'a> {
    prompt: &'a str,
    model: &'a str,
    background: &'static str,
    quality: &'static str,
    size: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<Image>,
}

#[derive(Serialize)]
struct Image {
    image_url: String,
}

fn encode(request: &ImageRequest) -> Bytes {
    let size = match request.aspect {
        Some(aspect) => {
            let (width, height) = aspect.size(LONG_SIDE);
            format!("{width}x{height}")
        }
        None => "auto".to_owned(),
    };
    let body = Body {
        prompt: &request.prompt,
        model: &request.model,
        background: if request.transparent { "transparent" } else { "opaque" },
        quality: "auto",
        size,
        images: request
            .images
            .iter()
            .map(|image| Image { image_url: image.data_url() })
            .collect(),
    };
    Bytes::from(json_body(&body))
}

impl ImageMaker for CodexProvider {
    fn vendor(&self) -> &'static str {
        "ChatGPT"
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
                reader: read_codex_failure,
                used_up: with_usage_limit,
                clock: &*shared.clock,
                quota: &shared.quota,
            };
            call.send(http, url, encode(&request), |refused| async move {
                let credentials = shared.auth.credentials(http, refused.as_ref()).await?;
                let headers = account_headers(&credentials, &shared.user_agent);
                Ok((headers, credentials.access_token))
            })
            .await
        })
    }
}
