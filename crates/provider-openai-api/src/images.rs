//! The image role of an OpenAI or xAI key and of a compatible entry whose
//! image shape is OpenAI Images (`images.md` § Vendors, `media.md` § Which
//! entries make which media): each vendor's image API under the entry's
//! base URL, with the key as a bearer token; the models the vendor lists
//! for the key, read when asked, or the models a compatible entry's user
//! added; and the vendor's failures read as its text failures are, a usage
//! or billing limit used up and a refused key named. A compatible endpoint
//! is sent OpenAI's requests and read as OpenAI's API is.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    ErrorCode, ProviderFailure, Secret, endpoint_url,
    images::{ImageMaker, ImageRequest, MadeImage, openai, read_data_answer, xai},
    media::{
        MediaAnswer, MediaCall, MediaCredential, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, created_date,
        unreadable, vendor_message,
    },
    media_catalog::Catalog,
    json_body, read_http_failure,
};
use demi_shared_types::Clock;
use futures_util::future::BoxFuture;
use reqwest::{
    StatusCode, Url,
    header::{AUTHORIZATION, HeaderMap},
};
use serde::{Deserialize, Serialize};

/// Whose image API an entry's requests go to.
enum Flavor {
    OpenAi,
    Xai,
    /// A compatible endpoint, which the user knows by the entry's name and
    /// whose models are the ones the user added.
    Compatible { name: String, models: Vec<MediaModelInfo> },
}

/// The image role of a key or a compatible endpoint.
pub(crate) struct KeyImages {
    flavor: Flavor,
    /// None for a compatible endpoint that takes none.
    key: Option<Secret>,
    generations: Url,
    edits: Url,
    models: Url,
    clock: Arc<dyn Clock>,
}

impl KeyImages {
    /// The role of an OpenAI or xAI `key` under the API `base`.
    pub(crate) fn new(vendor: MediaVendor, key: Secret, base: &Url, clock: Arc<dyn Clock>) -> Self {
        let flavor = match vendor {
            MediaVendor::Xai => Flavor::Xai,
            MediaVendor::OpenAi | MediaVendor::Google | MediaVendor::Elevenlabs => Flavor::OpenAi,
        };
        Self::of(flavor, Some(key), base, clock)
    }

    fn of(flavor: Flavor, key: Option<Secret>, base: &Url, clock: Arc<dyn Clock>) -> Self {
        let models = match flavor {
            Flavor::Xai => "/image-generation-models",
            Flavor::OpenAi | Flavor::Compatible { .. } => "/models",
        };
        Self {
            flavor,
            generations: endpoint_url(base, "/images/generations"),
            edits: endpoint_url(base, "/images/edits"),
            models: endpoint_url(base, models),
            key,
            clock,
        }
    }

    fn label(&self) -> &str {
        match &self.flavor {
            Flavor::OpenAi => MediaVendor::OpenAi.name(),
            Flavor::Xai => MediaVendor::Xai.name(),
            Flavor::Compatible { name, .. } => name,
        }
    }

    fn call(&self) -> MediaCall<'_> {
        key_call(matches!(self.flavor, Flavor::Xai), &*self.clock)
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(key) = &self.key {
            headers.insert(AUTHORIZATION, key.bearer());
        }
        headers
    }

    async fn list(&self, http: &demi_provider_common::ProviderHttp) -> Result<Vec<MediaModelInfo>, MediaFailure> {
        if let Flavor::Compatible { models, .. } = &self.flavor {
            return Ok(models.clone());
        }
        match self.flavor {
            Flavor::Xai => xai_listing(&self.call().get(http, &self.models, self.headers()).await?),
            Flavor::OpenAi | Flavor::Compatible { .. } => {
                let listed = openai_models(&self.call(), http, &self.models, self.headers()).await?;
                Ok(listed
                    .into_iter()
                    .filter(|model| model.id.starts_with("gpt-image-") || model.id.starts_with("chatgpt-image-"))
                    .collect())
            }
        }
    }

    fn encode(&self, catalog: &Catalog, request: &ImageRequest) -> Bytes {
        let body = match self.flavor {
            Flavor::Xai => {
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
            Flavor::OpenAi | Flavor::Compatible { .. } => {
                let size = openai::size(catalog, &request.model, request.aspect);
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

impl MediaMaker for KeyImages {
    fn name(&self) -> &str {
        self.label()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        match self.flavor {
            Flavor::OpenAi => Some(MediaVendor::OpenAi),
            Flavor::Xai => Some(MediaVendor::Xai),
            Flavor::Compatible { .. } => None,
        }
    }

    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(self.list(http))
    }
}

/// Both APIs refuse a model they do not have rather than answer with
/// another, so the model is confirmed, as [`MediaMaker`] holds by default;
/// a compatible endpoint is taken to answer as OpenAI's does.
impl ImageMaker for KeyImages {
    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        Box::pin(async move {
            let url = if request.images.is_empty() { &self.generations } else { &self.edits };
            let headers = self.headers();
            let answer = self
                .call()
                .post_json(http, url, self.encode(catalog, &request), |_| {
                    std::future::ready(Ok((headers.clone(), None)))
                })
                .await
                .and_then(|body| read_data_answer(&body));
            match self.flavor {
                // A refused shape is told with the shapes xAI named.
                Flavor::Xai => xai::with_shapes(answer),
                Flavor::OpenAi | Flavor::Compatible { .. } => answer,
            }
        })
    }
}

/// The image role of a compatible entry whose image shape is OpenAI
/// Images: OpenAI's image requests under its `base`, with its `key` when it
/// has one, offering the `models` its user added; `name` is the entry's,
/// which its failures name.
pub fn compatible_images(
    name: String,
    key: Option<Secret>,
    base: &Url,
    models: Vec<MediaModelInfo>,
    clock: Arc<dyn Clock>,
) -> Arc<dyn ImageMaker> {
    Arc::new(KeyImages::of(Flavor::Compatible { name, models }, key, base, clock))
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

/// The failure of a listing that does not read as documented.
fn unread(error: serde_json::Error) -> MediaFailure {
    unreadable(format!("its list of models could not be read: {error}"))
}

/// The models of xAI's listings of its image or video models, `GET
/// /v1/image-generation-models` and `/v1/video-generation-models` alike:
/// each model, then each of its aliases, with the model's release date.
pub(crate) fn xai_listing(text: &[u8]) -> Result<Vec<MediaModelInfo>, MediaFailure> {
    let listing: XaiListing = serde_json::from_slice(text).map_err(unread)?;
    let mut models = Vec::new();
    for model in listing.models {
        let released = model.created.and_then(created_date);
        models.push(MediaModelInfo {
            released: released.clone(),
            ..MediaModelInfo::id(&model.id)
        });
        for alias in &model.aliases {
            models.push(MediaModelInfo {
                released: released.clone(),
                description: Some(format!("Alias of {}", model.id)),
                ..MediaModelInfo::id(alias)
            });
        }
    }
    Ok(models)
}

/// `GET /v1/image-generation-models` and `/v1/video-generation-models`:
/// `models[]`, each with its aliases.
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

/// How an OpenAI-shaped request with a key is sent and its failures read:
/// as xAI reads them when `xai`, else as OpenAI does, which a compatible
/// endpoint is taken to follow.
/// Every model OpenAI's `GET …/models` at `url` lists for the key, each
/// with the date it was made: the one reader of that listing, which each
/// kind's role narrows to its own models.
pub(crate) async fn openai_models(
    call: &MediaCall<'_>,
    http: &demi_provider_common::ProviderHttp,
    url: &Url,
    headers: HeaderMap,
) -> Result<Vec<MediaModelInfo>, MediaFailure> {
    let text = call.get(http, url, headers).await?;
    let listing: OpenAiListing = serde_json::from_slice(&text).map_err(unread)?;
    Ok(listing
        .data
        .into_iter()
        .map(|model| MediaModelInfo { released: model.created.and_then(created_date), ..MediaModelInfo::id(&model.id) })
        .collect())
}

pub(crate) fn key_call(xai: bool, clock: &dyn Clock) -> MediaCall<'_> {
    MediaCall {
        label: if xai { "xAI" } else { "OpenAI" },
        credential: MediaCredential::Key {
            refused: if xai { xai_refused_key } else { openai_refused_key },
        },
        reader: read_http_failure,
        used_up: if xai { xai_used_up } else { openai_used_up },
        clock,
        quota: None,
    }
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
