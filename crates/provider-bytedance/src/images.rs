//! Seedream through an Ark key (`images.md` § Vendors):
//! `POST …/images/generations` makes an image from a prompt, and edits
//! one, or combines several, given as data URIs in `image`
//! ([Image generation API](https://docs.byteplus.com/en/docs/modelark/image-generation-api)).
//! A shape asked is sent as the catalog's size of that shape, and none is
//! sent without one, so the model's default stands. The answer is
//! `data[0].b64_json`, or the `error` of an image Ark refused. Ark lists
//! its models only to a request signed with an access key pair, so the
//! models are the catalog's, and a key is checked by an image request
//! with no prompt, which Ark refuses for its body before making anything.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret, endpoint_url, request_id_in,
    images::{ImageMaker, ImageRequest, MadeImage, nearest_size},
    json_body,
    media::{Answered, Ask, Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable},
    media_catalog::{Catalog, MediaKind},
};
use demi_shared_types::{B64Bytes, Clock, VendorTrace};
use futures_util::future::BoxFuture;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::ark;

/// An Ark key's image role.
pub(crate) struct ArkImages {
    vendor: MediaVendor,
    key: Secret,
    generations: Url,
    clock: Arc<dyn Clock>,
}

impl ArkImages {
    pub(crate) fn new(vendor: MediaVendor, key: Secret, base: Url, clock: Arc<dyn Clock>) -> Self {
        Self { vendor, key, generations: endpoint_url(&base, "/images/generations"), clock }
    }

    pub(crate) fn label(&self) -> &'static str {
        self.vendor.name()
    }

    async fn send(&self, http: &demi_provider_common::ProviderHttp, body: Vec<u8>) -> MediaAnswer<Answered> {
        let headers = ark::headers(Some(&self.key));
        ark::call(&*self.clock)
            .send(http, Ask::post_json(&self.generations, Bytes::from(body)), Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
    }
}

/// The models a key checked by `probe` has: the catalog's, `models`, when
/// the vendor refused the probe for its body, as it does a key it knows;
/// a refused key, a used-up one or no answer is the probe's failure.
pub(crate) fn checked<T>(probe: Result<T, MediaFailure>, models: Vec<MediaModelInfo>) -> Result<Vec<MediaModelInfo>, MediaFailure> {
    match probe {
        Ok(_) | Err(MediaFailure::Refused { .. } | MediaFailure::Shape { .. }) => Ok(models),
        Err(failure) => Err(failure),
    }
}

impl MediaMaker for ArkImages {
    fn name(&self) -> &str {
        self.label()
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

    fn check_key<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            let models = catalog_models(catalog, self.vendor, MediaKind::Image);
            let model = models.first().map_or("seedream", |model| model.id.as_str());
            let probe = self.send(http, json_body(&Probe { model, prompt: "" })).await.result.map(|answered| answered.body);
            checked(probe, models)
        })
    }
}

impl ImageMaker for ArkImages {
    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        Box::pin(async move {
            let size = request
                .aspect
                .and_then(|aspect| nearest_size(catalog, self.vendor.id(), &request.model, aspect))
                .map(|size| format!("{}x{}", size.width, size.height));
            let mut images: Vec<String> = request.images.iter().map(|image| image.data_url()).collect();
            let image = match images.len() {
                0 => None,
                1 => images.pop().map(Images::One),
                _ => Some(Images::Several(images)),
            };
            let body = json_body(&Body {
                model: &request.model,
                prompt: &request.prompt,
                image,
                size,
                response_format: "b64_json",
                watermark: false,
                sequential_image_generation: "disabled",
                output_format: "png",
            });
            self.send(http, body).await.and_then(|answered| read_answer(&answered.body, answered.trace_id))
        })
    }
}

#[derive(Serialize)]
struct Probe<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<Images>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<String>,
    response_format: &'static str,
    watermark: bool,
    sequential_image_generation: &'static str,
    output_format: &'static str,
}

/// An edit's images: one data URI, or an array of them.
#[derive(Serialize)]
#[serde(untagged)]
enum Images {
    One(String),
    Several(Vec<String>),
}

#[derive(Deserialize)]
struct Answer {
    data: Vec<Datum>,
}

#[derive(Deserialize)]
struct Datum {
    #[serde(default)]
    b64_json: Option<B64Bytes>,
    #[serde(default)]
    error: Option<DatumError>,
}

#[derive(Deserialize)]
struct DatumError {
    #[serde(default)]
    code: Option<String>,
    message: String,
}

/// The image of Seedream's answer: `data[0].b64_json`, or the refusal its
/// `error` says, traced by its code and the request ID its message quotes,
/// or else `trace_id`, the answer's headers'.
fn read_answer(body: &[u8], trace_id: Option<String>) -> Result<MadeImage, MediaFailure> {
    let answer: Answer =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    let Some(datum) = answer.data.into_iter().next() else {
        return Err(unreadable("its answer held no image".to_owned()));
    };
    match (datum.b64_json, datum.error) {
        (Some(image), _) => Ok(MadeImage { bytes: image.into_bytes(), usage: None, vendor_id: None }),
        (None, Some(error)) => {
            let id = request_id_in(&error.message).or(trace_id);
            Err(MediaFailure::Refused { message: error.message, code: None, trace: VendorTrace { code: error.code, id } })
        }
        (None, None) => Err(unreadable("its answer held no image".to_owned())),
    }
}
