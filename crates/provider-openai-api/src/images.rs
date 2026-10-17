//! The image role of an OpenAI or xAI key (`images.md` § Vendors): each
//! vendor's image API under the entry's base URL, with the key as a bearer
//! token; the models the vendor lists for the key, read when asked; and
//! the vendor's failures read as its text failures are, a usage or billing
//! limit used up and a refused key named.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    ErrorCode, ProviderFailure, Secret, endpoint_url,
    images::{
        ImageAnswer, ImageCall, ImageCredential, ImageFailure, ImageMaker, ImageModelInfo, ImageRequest,
        ImageVendor, created_date, listed, openai, read_data_answer, unreadable, vendor_message, xai,
    },
    json_body, read_http_failure,
};
use demi_shared_types::Clock;
use futures_util::future::BoxFuture;
use reqwest::{
    StatusCode, Url,
    header::{AUTHORIZATION, HeaderMap},
};
use serde::{Deserialize, Serialize};

/// An OpenAI or xAI key's image role.
pub(crate) struct KeyImages {
    vendor: ImageVendor,
    key: Secret,
    generations: Url,
    edits: Url,
    models: Url,
    clock: Arc<dyn Clock>,
}

impl KeyImages {
    /// The role of `vendor`'s `key` under the API `base`; `vendor` is
    /// OpenAI or xAI.
    pub(crate) fn new(vendor: ImageVendor, key: Secret, base: &Url, clock: Arc<dyn Clock>) -> Self {
        let models = match vendor {
            ImageVendor::Xai => "/image-generation-models",
            ImageVendor::OpenAi | ImageVendor::Google => "/models",
        };
        Self {
            vendor,
            generations: endpoint_url(base, "/images/generations"),
            edits: endpoint_url(base, "/images/edits"),
            models: endpoint_url(base, models),
            key,
            clock,
        }
    }

    fn call(&self) -> ImageCall<'_> {
        ImageCall {
            label: self.vendor.name(),
            credential: ImageCredential::Key {
                refused: match self.vendor {
                    ImageVendor::Xai => xai_refused_key,
                    ImageVendor::OpenAi | ImageVendor::Google => openai_refused_key,
                },
            },
            reader: read_http_failure,
            used_up: match self.vendor {
                ImageVendor::Xai => xai_used_up,
                ImageVendor::OpenAi | ImageVendor::Google => openai_used_up,
            },
            clock: &*self.clock,
            quota: None,
            answer: read_data_answer,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, self.key.bearer());
        headers
    }

    async fn list(&self, http: &reqwest::Client) -> Result<Vec<ImageModelInfo>, ImageFailure> {
        let text = self.call().get(http, &self.models, self.headers()).await?;
        let unread = |error: serde_json::Error| unreadable(format!("its list of models could not be read: {error}"));
        match self.vendor {
            ImageVendor::Xai => {
                let listing: XaiListing = serde_json::from_slice(&text).map_err(unread)?;
                let mut models = Vec::new();
                for model in listing.models {
                    let released = model.created.and_then(created_date);
                    models.push(listed(self.vendor, &model.id, released.clone(), None));
                    for alias in &model.aliases {
                        let said = format!("Alias of {}", model.id);
                        models.push(listed(self.vendor, alias, released.clone(), Some(said)));
                    }
                }
                Ok(models)
            }
            ImageVendor::OpenAi | ImageVendor::Google => {
                let listing: OpenAiListing = serde_json::from_slice(&text).map_err(unread)?;
                Ok(listing
                    .data
                    .into_iter()
                    .filter(|model| model.id.starts_with("gpt-image-") || model.id.starts_with("chatgpt-image-"))
                    .map(|model| listed(self.vendor, &model.id, model.created.and_then(created_date), None))
                    .collect())
            }
        }
    }

    fn encode(&self, request: &ImageRequest) -> Bytes {
        let body = match self.vendor {
            ImageVendor::Xai => {
                let mut images: Vec<XaiImage> = request
                    .images
                    .iter()
                    .map(|image| XaiImage { url: image.data_url(), kind: "image_url" })
                    .collect();
                // One image goes as `image`, several as `images`.
                let image = if images.len() == 1 { images.pop() } else { None };
                json_body(&XaiBody {
                    prompt: &request.prompt,
                    model: &request.model,
                    aspect_ratio: request.aspect.map(|aspect| aspect.to_string()),
                    response_format: "b64_json",
                    image,
                    images,
                })
            }
            ImageVendor::OpenAi | ImageVendor::Google => {
                let size = openai::size(&request.model, request.aspect);
                json_body(&OpenAiBody {
                    prompt: &request.prompt,
                    model: &request.model,
                    size,
                    background: if request.transparent { "transparent" } else { "opaque" },
                    output_format: "png",
                    quality: "auto",
                    images: request
                        .images
                        .iter()
                        .map(|image| OpenAiImage { image_url: image.data_url() })
                        .collect(),
                })
            }
        };
        Bytes::from(body)
    }
}

impl ImageMaker for KeyImages {
    fn name(&self) -> &'static str {
        self.vendor.name()
    }

    fn vendor(&self) -> ImageVendor {
        self.vendor
    }

    fn models<'a>(&'a self, http: &'a reqwest::Client) -> BoxFuture<'a, Result<Vec<ImageModelInfo>, ImageFailure>> {
        Box::pin(self.list(http))
    }

    /// Both APIs refuse a model they do not have rather than answer with
    /// another.
    fn confirms_model(&self) -> bool {
        true
    }

    fn make<'a>(&'a self, http: &'a reqwest::Client, request: ImageRequest) -> BoxFuture<'a, ImageAnswer> {
        Box::pin(async move {
            let url = if request.images.is_empty() { &self.generations } else { &self.edits };
            let headers = self.headers();
            let key = self.key.clone();
            let answer = self
                .call()
                .send(http, url, self.encode(&request), |_| {
                    std::future::ready(Ok((headers.clone(), key.clone())))
                })
                .await;
            match self.vendor {
                // A refused shape is told with the shapes xAI named.
                ImageVendor::Xai => xai::with_shapes(answer),
                ImageVendor::OpenAi | ImageVendor::Google => answer,
            }
        })
    }
}

/// `GET /v1/models`: `data[].id` and `created`, in seconds.
#[derive(Deserialize)]
struct OpenAiListing {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
    #[serde(default)]
    created: Option<i64>,
}

/// `GET /v1/image-generation-models`: `models[]`, each with its aliases.
#[derive(Deserialize)]
struct XaiListing {
    models: Vec<XaiModel>,
}

#[derive(Deserialize)]
struct XaiModel {
    id: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Serialize)]
struct OpenAiBody<'a> {
    prompt: &'a str,
    model: &'a str,
    size: String,
    background: &'static str,
    output_format: &'static str,
    quality: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<OpenAiImage>,
}

#[derive(Serialize)]
struct OpenAiImage {
    image_url: String,
}

#[derive(Serialize)]
struct XaiBody<'a> {
    prompt: &'a str,
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    response_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<XaiImage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<XaiImage>,
}

#[derive(Serialize)]
struct XaiImage {
    url: String,
    #[serde(rename = "type")]
    kind: &'static str,
}

/// OpenAI refuses a key it does not accept with HTTP 401
/// (`invalid_api_key`).
fn openai_refused_key(status: StatusCode, _: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
}

/// xAI refuses a key with HTTP 401, and an unknown one with HTTP 400 that
/// says *Incorrect API key provided*.
fn xai_refused_key(status: StatusCode, body: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
        || status == StatusCode::BAD_REQUEST
            && vendor_message(body).is_some_and(|said| said.to_lowercase().contains("api key"))
}

/// An OpenAI failure whose `error.code` or `error.type` is
/// `insufficient_quota` or `billing_hard_limit_reached`: the key's credit
/// or its budget is spent, which waiting does not fix.
fn openai_used_up(failure: ProviderFailure) -> ProviderFailure {
    with_usage_limit(failure, |error| {
        let field = |name| error.get(name).and_then(serde_json::Value::as_str);
        [field("code"), field("type")]
            .into_iter()
            .flatten()
            .any(|value| matches!(value, "insufficient_quota" | "billing_hard_limit_reached"))
    })
}

/// An xAI failure that says the team's credits are spent or its spending
/// limit is reached.
fn xai_used_up(failure: ProviderFailure) -> ProviderFailure {
    with_usage_limit(failure, |error| {
        let said = error.as_str().map(str::to_owned).unwrap_or_else(|| error.to_string()).to_lowercase();
        said.contains("credits") || said.contains("spending limit")
    })
}

/// `failure` marked as a usage limit when its body's `error` is one by
/// `used_up`.
fn with_usage_limit(mut failure: ProviderFailure, used_up: impl Fn(&serde_json::Value) -> bool) -> ProviderFailure {
    #[derive(Deserialize)]
    struct Refusal {
        error: serde_json::Value,
    }
    let refusal = failure
        .diagnostics
        .as_deref()
        .and_then(demi_provider_common::HttpFailureRecord::read)
        .and_then(|record| serde_json::from_str::<Refusal>(&record.body).ok());
    if refusal.is_some_and(|refusal| used_up(&refusal.error)) {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}
