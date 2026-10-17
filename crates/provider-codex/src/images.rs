//! Codex's image role (`images.md` § Vendors): the Codex backend's image
//! endpoints, which the Codex CLI's own image tool calls, with the
//! account's headers as inference sends them. The backend ignores the size
//! today and may honour it later, so the size of the asked shape is sent.

use bytes::Bytes;
use demi_provider_common::{
    images::{ImageAnswer, ImageCall, ImageCredential, ImageFailure, ImageMaker, ImageModelInfo, ImageRequest, ImageVendor, openai, read_data_answer, table},
    json_body,
};
use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::{CodexProvider, LABEL, failure::{read_codex_failure, with_usage_limit}, request::account_headers};

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
    let size = openai::size(&request.model, request.aspect);
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
    fn name(&self) -> &'static str {
        "ChatGPT"
    }

    fn vendor(&self) -> ImageVendor {
        ImageVendor::OpenAi
    }

    /// OpenAI's table: the backend refuses every listing route.
    fn models<'a>(&'a self, _: &'a reqwest::Client) -> BoxFuture<'a, Result<Vec<ImageModelInfo>, ImageFailure>> {
        Box::pin(std::future::ready(Ok(table(ImageVendor::OpenAi))))
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
                credential: ImageCredential::Account { family: crate::auth::FAMILY },
                reader: read_codex_failure,
                used_up: with_usage_limit,
                clock: &*shared.clock,
                quota: Some(&shared.quota),
                answer: read_data_answer,
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
