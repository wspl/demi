//! MiniMax's images through its key.
//!
//! - `POST /v1/image_generation` on the entry's origin makes an image from a
//!   prompt, with `aspect_ratio` when a shape is asked ([Text to
//!   image](https://platform.minimax.io/docs/api-reference/image-generation-t2i)).
//! - It also follows a portrait given as `subject_reference`, which only
//!   `image-01` and `image-01-live` take ([Image to
//!   image](https://platform.minimax.io/docs/api-reference/image-generation-i2i)).
//! - The answer is `data.image_base64[0]`, or the refusal its `base_resp`
//!   states.
//! - MiniMax lists only its text models, so the models are the catalog's.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret, endpoint_url,
    images::{ImageMaker, ImageRequest, MadeImage},
    json_body,
    media::{Ask, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable},
    media_catalog::{Catalog, MediaKind},
};
use demi_shared_types::{B64Bytes, Clock};
use futures_util::future::BoxFuture;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::minimax;

/// A MiniMax key's image role.
pub(crate) struct MinimaxImages {
    vendor: MediaVendor,
    key: Secret,
    generation: Url,
    clock: Arc<dyn Clock>,
}

impl MinimaxImages {
    pub(crate) fn new(vendor: MediaVendor, key: Secret, origin: &Url, clock: Arc<dyn Clock>) -> Self {
        Self { vendor, key, generation: endpoint_url(origin, "/v1/image_generation"), clock }
    }
}

impl MediaMaker for MinimaxImages {
    fn name(&self) -> &str {
        self.vendor.name()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(self.vendor)
    }

    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(std::future::ready(Ok(catalog_models(catalog, self.vendor, MediaKind::Image))))
    }
}

impl ImageMaker for MinimaxImages {
    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        Box::pin(async move {
            let body = json_body(&Body {
                model: &request.model,
                prompt: &request.prompt,
                aspect_ratio: request.aspect.map(|aspect| aspect.to_string()),
                subject_reference: request
                    .images
                    .iter()
                    .map(|image| Reference { kind: "character", image_file: image.data_url() })
                    .collect(),
                response_format: "base64",
                n: 1,
            });
            minimax::send_checked(&*self.clock, http, &self.key, Ask::post_json(&self.generation, Bytes::from(body)))
                .await
                .and_then(|body| read_answer(&body))
        })
    }
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subject_reference: Vec<Reference>,
    response_format: &'static str,
    n: u32,
}

#[derive(Serialize)]
struct Reference {
    #[serde(rename = "type")]
    kind: &'static str,
    image_file: String,
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    data: Option<Data>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct Data {
    #[serde(default)]
    image_base64: Vec<B64Bytes>,
}

/// The image of MiniMax's answer; an answer of no image is one MiniMax
/// blocked for its content, as its `metadata.failed_count` counts.
fn read_answer(body: &[u8]) -> Result<MadeImage, MediaFailure> {
    let answer: Answer =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    let image = answer.data.and_then(|data| data.image_base64.into_iter().next());
    match image {
        Some(image) => Ok(MadeImage { bytes: image.into_bytes(), usage: None, vendor_id: answer.id }),
        None => Err(MediaFailure::Refused { message: "it blocked the image for its content".to_owned(), code: None, trace: Default::default() }),
    }
}
