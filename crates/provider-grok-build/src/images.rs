//! Grok Build's image role (`images.md` § Vendors): the chat proxy's image
//! endpoints, which Grok's own CLI calls, with the account's identity as
//! inference sends it. The image comes back as base64: an image the proxy
//! answers by URL refuses an immediate download.

use bytes::Bytes;
use demi_provider_common::{
    images::{ImageAnswer, ImageCall, ImageFailure, ImageMaker, ImageModelInfo, ImageRequest},
    json_body, read_http_failure,
};
use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::{GrokProvider, LABEL, request::identity_headers, run::with_usage_limit};

/// What every Grok image model takes: the shapes of `aspect_ratio` in
/// xAI's images API reference, and the JPEG its answers carry.
const SHAPES: &str = "1:1, 3:4, 4:3, 9:16, 16:9, 2:3, 3:2, 9:19.5, 19.5:9, 9:20, 20:9, 1:2, 2:1, 21:9 or 5:2; sends JPEG";

/// The image models, newest first: xAI's published IDs the subscription
/// accepts (`images.md` § Vendors); the proxy refuses a model it does not
/// have. xAI's model pages carry no one-line description, so each says
/// what xAI publishes of it: the price on x.ai/api/imagine, Image 2.0's
/// announcement, the retirement in its release notes, and the aliases on
/// docs.x.ai/developers/models. A date is the dated alias's, or Image
/// 2.0's announcement's.
const MODELS: &[ImageModelInfo] = &[
    ImageModelInfo::new(
        "grok-imagine-image-2.0",
        "2026-08-07",
        "Precise image generation and editing, built for real creative work",
        SHAPES,
    ),
    ImageModelInfo::new(
        "grok-imagine-image-quality",
        "2026-04-03",
        "Retires on November 2, 2026; grok-imagine-image-2.0 covers its uses at a lower price",
        SHAPES,
    ),
    ImageModelInfo::new("grok-imagine-image-quality-20260403", "2026-04-03", "Alias of grok-imagine-image-quality", SHAPES),
    ImageModelInfo::new("grok-imagine-image-quality-latest", "2026-04-03", "Alias of grok-imagine-image-quality", SHAPES),
    ImageModelInfo::new("grok-imagine-image-pro", "2026-04-03", "Alias of grok-imagine-image-quality", SHAPES),
    ImageModelInfo::new("grok-imagine-image", "2026-03-02", "Lowest-cost image model, from $0.02 an image", SHAPES),
    ImageModelInfo::new("grok-imagine-image-2026-03-02", "2026-03-02", "Alias of grok-imagine-image", SHAPES),
];

/// The model the subscription answers by default.
const DEFAULT_MODEL: &str = "grok-imagine-image";

/// The shapes the proxy takes, as its refusal of `aspect_ratio` lists
/// them: ``aspect_ratio: unknown variant `4:5`, expected one of `1:1`,
/// `3:4`, …, `auto` at line 1 column 304``. The whole list is read, each
/// entry quoted and the last after `, ` or ` or `; only the W:H shapes are
/// kept, as `auto` is no shape `--aspect` asks for. None for any other
/// refusal or a list not read whole, which is then told in the proxy's
/// own words.
fn shapes_taken(message: &str) -> Option<Vec<String>> {
    let (_, rest) = message.split_once("aspect_ratio: unknown variant `")?;
    let (_, mut rest) = rest.split_once("`, expected one of ")?;
    let mut takes = Vec::new();
    loop {
        let (entry, after) = rest.strip_prefix('`')?.split_once('`')?;
        if entry.contains(':') {
            takes.push(entry.to_owned());
        } else if entry != "auto" {
            return None;
        }
        match after.strip_prefix(", ").or_else(|| after.strip_prefix(" or ")) {
            Some(next) => rest = next,
            // The list ends where no separator follows: at the message's
            // end or its position, `at line …`.
            None if after.is_empty() || after.starts_with(" at ") => break,
            None => return None,
        }
    }
    (!takes.is_empty()).then_some(takes)
}

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

    fn models(&self) -> &'static [ImageModelInfo] {
        MODELS
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    /// The proxy refuses a model it does not have rather than answer with
    /// another.
    fn confirms_model(&self) -> bool {
        true
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
            let mut answer = call
                .send(http, url, encode(&request), |refused| async move {
                    let credentials = shared.auth.credentials(http, refused.as_ref()).await?;
                    Ok((identity_headers(&credentials), credentials.access_token))
                })
                .await;
            // A refused shape is told with the shapes the proxy named, so a
            // shape xAI adds needs no change here (`images.md` § Commands).
            if let Err(ImageFailure::Refused { message, code }) = &answer.result
                && let Some(takes) = shapes_taken(message)
            {
                answer.result = Err(ImageFailure::Shape { takes, code: code.clone() });
            }
            answer
        })
    }
}
