//! Grok Build's image role (`images.md` § Vendors): the chat proxy's image
//! endpoints, which Grok's own CLI calls, with the account's identity as
//! inference sends it. The image comes back as base64: an image the proxy
//! answers by URL refuses an immediate download.

use bytes::Bytes;
use demi_provider_common::{
    Secret,
    credentials::AuthFailure,
    images::{ImageMaker, ImageRequest, MadeImage, read_data_answer, xai},
    media::{MediaAnswer, MediaCall, MediaCredential, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, subscription_models},
    media_catalog::{Catalog, MediaKind, Subscription},
    json_body, read_http_failure,
};
use futures_util::future::BoxFuture;
use reqwest::header::HeaderMap;
use serde::Serialize;

use crate::{LABEL, Shared, request::identity_headers, run::with_usage_limit};

impl Shared {
    /// How the proxy's media requests are sent: with the account's token,
    /// renewed once when refused, its usage limit read as a turn's is.
    pub(crate) fn media_call(&self) -> MediaCall<'_> {
        MediaCall {
            label: LABEL,
            credential: MediaCredential::Account { family: crate::auth::FAMILY },
            reader: read_http_failure,
            used_up: with_usage_limit,
            clock: &*self.clock,
            quota: Some(&self.quota),
        }
    }

    /// The account's headers for a media request, renewed past `refused`,
    /// the token a request was refused with, and the token they carry.
    pub(crate) async fn media_headers(
        &self,
        http: &demi_provider_common::ProviderHttp,
        refused: Option<Secret>,
    ) -> Result<(HeaderMap, Option<Secret>), AuthFailure> {
        let credentials = self.auth.credentials(http, refused.as_ref()).await?;
        Ok((identity_headers(&credentials, &self.releases), Some(credentials.access_token)))
    }
}

/// The image role of the provider's account.
pub(crate) struct GrokImages(pub(crate) std::sync::Arc<Shared>);

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

impl MediaMaker for GrokImages {
    fn name(&self) -> &str {
        "Grok"
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Xai)
    }

    /// The catalog's models of a Grok subscription, those it accepted: the
    /// proxy's listing answers 404.
    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(std::future::ready(Ok(subscription_models(catalog, Subscription::Grok, MediaKind::Image))))
    }
}

/// The proxy refuses a model it does not have rather than answer with
/// another, so its model is confirmed, as [`MediaMaker`] holds by default.
impl ImageMaker for GrokImages {
    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        Box::pin(async move {
            let shared = &*self.0;
            let url = if request.images.is_empty() {
                &shared.image_generations_url
            } else {
                &shared.image_edits_url
            };
            let answer = shared
                .media_call()
                .post_json(http, url, encode(&request), |refused| shared.media_headers(http, refused))
                .await
                .and_then(|body| read_data_answer(&body));
            // A refused shape is told with the shapes the proxy named.
            xai::with_shapes(answer)
        })
    }
}
