//! Grok Build's image role (`images.md` § Vendors): the chat proxy's image
//! endpoints, which Grok's own CLI calls, with the account's identity as
//! inference sends it. The image comes back as base64: an image the proxy
//! answers by URL refuses an immediate download.

use bytes::Bytes;
use demi_provider_common::{
    images::{ImageAnswer, ImageCall, ImageCredential, ImageFailure, ImageMaker, ImageModelInfo, ImageRequest, ImageVendor, read_data_answer, subscription_models, xai},
    media_catalog::{Catalog, Subscription},
    json_body, read_http_failure,
};
use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::{GrokProvider, LABEL, request::identity_headers, run::with_usage_limit};

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
    fn name(&self) -> &str {
        "Grok"
    }

    fn vendor(&self) -> Option<ImageVendor> {
        Some(ImageVendor::Xai)
    }

    /// The catalog's models of a Grok subscription, those it accepted: the
    /// proxy's listing answers 404.
    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<ImageModelInfo>, ImageFailure>> {
        Box::pin(std::future::ready(Ok(subscription_models(catalog, Subscription::Grok))))
    }

    /// The proxy refuses a model it does not have rather than answer with
    /// another.
    fn confirms_model(&self) -> bool {
        true
    }

    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
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
                credential: ImageCredential::Account { family: crate::auth::FAMILY },
                reader: read_http_failure,
                used_up: with_usage_limit,
                clock: &*shared.clock,
                quota: Some(&shared.quota),
                answer: read_data_answer,
            };
            let answer = call
                .send(http, url, encode(&request), |refused| async move {
                    let credentials = shared.auth.credentials(http, refused.as_ref()).await?;
                    Ok((identity_headers(&credentials), Some(credentials.access_token)))
                })
                .await;
            // A refused shape is told with the shapes the proxy named.
            xai::with_shapes(answer)
        })
    }
}
