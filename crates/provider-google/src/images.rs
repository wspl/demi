//! The image role of a Gemini API key, and the key its media roles share.
//!
//! - An image is a job (`crate::interactions`): the prompt and its images
//!   in, the image read from the finished interaction's output.
//! - A model that answers in words instead fails in them.
//! - The models are the ones Google lists for the key, read when asked.

use std::sync::Arc;

use demi_provider_common::{
    media_catalog::Catalog,
    ErrorCode, HttpFailureRecord, ProviderFailure, Secret, model_words,
    images::{ImageMaker, ImageRequest, MadeImage},
    media::{
        MediaAnswer, MediaCall, MediaCredential, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable,
        vendor_message, within_limit,
        jobs::{self, JobRef, JobState, Jobs},
    },
    read_http_failure,
};
use demi_shared_types::{Clock, VendorTrace};
use futures_util::future::BoxFuture;
use reqwest::{
    StatusCode, Url,
    header::{HeaderMap, HeaderName},
};
use serde::Deserialize;

use crate::interactions::{self, Standing};

/// How failure messages name the vendor.
pub(crate) const LABEL: &str = "Google";

/// The most models one page of the listing holds, the most Google takes.
const PAGE_SIZE: &str = "1000";

/// A Gemini API key, which its media roles send their requests with.
#[derive(Clone)]
pub(crate) struct GeminiKey {
    key: Secret,
    base: Url,
    pub(crate) clock: Arc<dyn Clock>,
}

impl GeminiKey {
    pub(crate) fn new(key: Secret, base: Url, clock: Arc<dyn Clock>) -> Self {
        Self { key, base, clock }
    }

    pub(crate) fn call(&self) -> MediaCall<'_> {
        MediaCall {
            label: LABEL,
            credential: MediaCredential::Key { refused: refused_key },
            reader: read_http_failure,
            used_up,
            clock: &*self.clock,
            quota: None,
        }
    }

    pub(crate) fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("x-goog-api-key"), self.key.header_value());
        headers
    }

    /// The base, the API's version, such as `…/v1beta`.
    pub(crate) fn base(&self) -> &Url {
        &self.base
    }

    /// `…/models` under the base, then `segment` when given.
    pub(crate) fn models_url(&self, segment: Option<&str>) -> Url {
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                // An http or https URL always has a path to extend.
                .expect("an API base URL has path segments");
            path.pop_if_empty().push("models");
            if let Some(segment) = segment {
                path.push(segment);
            }
        }
        url
    }

    /// Every page of the listing: the models `keeps` keeps, by the words of
    /// their ID and the generation methods they list.
    pub(crate) async fn list(
        &self,
        http: &demi_provider_common::ProviderHttp,
        keeps: fn(&[String], &[String]) -> bool,
    ) -> Result<Vec<MediaModelInfo>, MediaFailure> {
        let mut models = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut url = self.models_url(None);
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("pageSize", PAGE_SIZE);
                if let Some(token) = &page {
                    query.append_pair("pageToken", token);
                }
            }
            let text = self.call().get(http, &url, self.headers()).await?;
            let listing: Listing = serde_json::from_slice(&text)
                .map_err(|error| unreadable(format!("its list of models could not be read: {error}")))?;
            for model in listing.models {
                let id = model.name.strip_prefix("models/").unwrap_or(&model.name);
                if keeps(&model_words(id), &model.supported_generation_methods) {
                    models.push(MediaModelInfo { description: model.description.clone(), ..MediaModelInfo::id(id) });
                }
            }
            match listing.next_page_token.filter(|token| !token.is_empty()) {
                Some(token) => page = Some(token),
                None => return Ok(models),
            }
        }
    }
}

/// A Gemini API key's image role.
pub(crate) struct KeyImages(pub(crate) GeminiKey);

/// The models whose ID has the word `image` or `banana` and that list
/// `generateContent`.
fn makes_images(words: &[String], methods: &[String]) -> bool {
    words.iter().any(|word| word == "image" || word == "banana") && methods.iter().any(|method| method == "generateContent")
}

/// The model is the request's URL: Google answers no other, so it is
/// confirmed, as [`MediaMaker`] holds by default.
impl MediaMaker for KeyImages {
    fn name(&self) -> &str {
        LABEL
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Google)
    }

    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(self.0.list(http, makes_images))
    }
}

impl ImageMaker for KeyImages {
    fn make<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        Box::pin(async move {
            let model = request.model.clone();
            jobs::made(self, http, catalog, request, &model).await
        })
    }

    fn jobs(&self) -> Option<&dyn Jobs<ImageRequest, MadeImage>> {
        Some(self)
    }
}

impl Jobs<ImageRequest, MadeImage> for KeyImages {
    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, _: &'a Catalog, request: ImageRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(interactions::create(&self.0, http, encode(&request)))
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState<MadeImage>>> {
        Box::pin(async move {
            interactions::get(&self.0, http, job.id).await.and_then(|interaction| match interaction.standing() {
                Standing::Pending { running: false } => Ok(JobState::Queued),
                Standing::Pending { running: true } => Ok(JobState::Running { progress: None }),
                Standing::Failed(trace) => Ok(JobState::Failed { reason: None, trace }),
                Standing::Completed => read_image(interaction),
            })
        })
    }
}

/// The interaction `request` asks for: the prompt, then its images, and an
/// image back in the shape asked.
fn encode(request: &ImageRequest) -> serde_json::Value {
    let mut content = vec![serde_json::json!({ "type": "text", "text": request.prompt })];
    content.extend(
        request
            .images
            .iter()
            .map(|image| serde_json::json!({ "type": "image", "data": image.base64(), "mime_type": image.media_type })),
    );
    let mut format = serde_json::json!({ "type": "image" });
    if let Some(aspect) = request.aspect {
        format["aspect_ratio"] = serde_json::Value::String(aspect.to_string());
    }
    serde_json::json!({
        "model": request.model,
        "input": [{ "type": "user_input", "content": content }],
        "response_format": format,
    })
}

/// The last image of the finished interaction's output; one without an
/// image fails with what the model said instead, when it said something.
fn read_image(interaction: interactions::Interaction) -> Result<JobState<MadeImage>, MediaFailure> {
    let Some(image) = interaction.outputs().rev().find(|content| content.kind == "image" && content.data.is_some()) else {
        return match interaction.said() {
            Some(said) => Ok(JobState::Failed { reason: None, trace: VendorTrace::said(said) }),
            None => Err(unreadable("its answer held no image".to_owned())),
        };
    };
    let bytes = within_limit(image.data.clone().expect("an image block with data").into_bytes(), "image")?;
    Ok(JobState::Done(MadeImage {
        bytes,
        usage: interaction.usage.as_ref().map(interactions::Usage::counts),
        vendor_id: interaction.id.clone(),
        cost: None,
    }))
}

/// `GET /v1beta/models`: one page of models.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    #[serde(default)]
    models: Vec<ListedModel>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedModel {
    /// `models/<id>`.
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    supported_generation_methods: Vec<String>,
}

/// Google refuses a key it does not accept with HTTP 400 that says *API
/// key not valid* (`API_KEY_INVALID`), and one it does not allow here with
/// HTTP 401 or 403.
fn refused_key(status: StatusCode, body: &str) -> bool {
    matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        || status == StatusCode::BAD_REQUEST
            && (body.contains("API_KEY_INVALID")
                || vendor_message(body).is_some_and(|said| said.to_lowercase().contains("api key")))
}

/// A refusal of HTTP 429 whose quota is used up ([`QuotaDetails::spent`]);
/// any other is a rate limit.
fn used_up(mut failure: ProviderFailure) -> ProviderFailure {
    #[derive(Deserialize)]
    struct Refusal {
        error: QuotaDetails,
    }
    let refusal = failure
        .diagnostics
        .as_deref()
        .and_then(HttpFailureRecord::read)
        .filter(|record| record.status == 429)
        .and_then(|record| serde_json::from_str::<Refusal>(&record.body).ok());
    if refusal.is_some_and(|refusal| refusal.error.spent()) {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}

/// The quotas a Google error's `details` name, an HTTP refusal's and a
/// stream's error alike, read in either case: the Interactions API names
/// its own fields in snake case, `generateContent` in camel case.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct QuotaDetails {
    #[serde(default)]
    details: Vec<Detail>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detail {
    #[serde(default)]
    violations: Vec<Violation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Violation {
    #[serde(default, alias = "quota_id")]
    quota_id: Option<String>,
    #[serde(default, alias = "quota_value")]
    quota_value: Option<String>,
}

impl QuotaDetails {
    /// Whether a quota named is the day's, or one the key's tier allows none
    /// of: waiting minutes does not lift it, so the key is used up. A quota
    /// of the minute, or none named, is a rate limit.
    pub(crate) fn spent(&self) -> bool {
        self.details.iter().flat_map(|detail| &detail.violations).any(|violation| {
            violation.quota_id.as_deref().is_some_and(|id| id.contains("PerDay"))
                || violation.quota_value.as_deref() == Some("0")
        })
    }
}
