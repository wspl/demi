//! Codex's image role (`images.md` § Vendors): the Codex backend's image
//! endpoints, which the Codex CLI's own image tool calls, with the
//! account's headers as inference sends them. The backend ignores the size
//! today and may honour it later, so the size of the asked shape is sent.

use bytes::Bytes;
use demi_provider_common::{
    images::{ImageAnswer, ImageCall, ImageMaker, ImageModelInfo, ImageRequest},
    json_body,
};
use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::{CodexProvider, LABEL, failure::{read_codex_failure, with_usage_limit}, request::account_headers};

/// What OpenAI's flexible sizes take: `gpt-image-2`'s and GPT Image 2.5's
/// in its image generation guide.
macro_rules! flexible {
    ($more:literal) => {
        concat!(
            "any W×H, each edge a multiple of 16 and at most 3840, long:short at most 3:1, 655,360 to 8,294,400 pixels; ",
            $more
        )
    };
}

/// The three sizes of the GPT Image models before `gpt-image-2`.
const FIXED: &str = "1024×1024, 1536×1024 or 1024×1536";

/// The image models, newest first (`images.md` § Vendors): OpenAI's
/// published IDs, each with the line its page on
/// developers.openai.com/api/docs/models opens with, the date of its
/// release in OpenAI's API changelog (a dated ID's is its snapshot's), and
/// the sizes and transparency its image generation guide and changelog
/// give it.
const MODELS: &[ImageModelInfo] = &[
    ImageModelInfo::new(
        "gpt-image-2.5-sunburst",
        "2026-09-08",
        "Most capable model for image generation and editing",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageModelInfo::new(
        "gpt-image-2.5-sunburst-2026-09-08",
        "2026-09-08",
        "Dated snapshot of gpt-image-2.5-sunburst",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageModelInfo::new(
        "gpt-image-2.5-flare",
        "2026-09-08",
        "Fast, high-quality everyday image generation",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageModelInfo::new(
        "gpt-image-2.5-flare-2026-09-08",
        "2026-09-08",
        "Dated snapshot of gpt-image-2.5-flare",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageModelInfo::new(
        "gpt-image-2",
        "2026-04-21",
        "State-of-the-art image generation model",
        flexible!("transparent in PNG or WebP, in preview"),
    ),
    ImageModelInfo::new(
        "gpt-image-2-2026-04-21",
        "2026-04-21",
        "Dated snapshot of gpt-image-2",
        flexible!("transparent in PNG or WebP, in preview"),
    ),
    ImageModelInfo::new("gpt-image-1.5", "2025-12-16", "Previous image generation model", FIXED),
    ImageModelInfo::new("gpt-image-1.5-2025-12-16", "2025-12-16", "Dated snapshot of gpt-image-1.5", FIXED),
    ImageModelInfo::new("chatgpt-image-latest", "2025-12-16", "Previous image model used in ChatGPT", FIXED),
    ImageModelInfo::new("gpt-image-1-mini", "2025-10-06", "Cost-efficient version of gpt-image-1", FIXED),
    ImageModelInfo::new("gpt-image-1", "2025-04-23", "Previous image generation model, before gpt-image-1.5", FIXED),
];

/// The model the Codex CLI asks for.
const DEFAULT_MODEL: &str = "gpt-image-2";

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

    fn models(&self) -> &'static [ImageModelInfo] {
        MODELS
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    /// The backend's answer names no model, and it may answer with another
    /// than the one asked for.
    fn confirms_model(&self) -> bool {
        false
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
