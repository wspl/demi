//! The image role of a Gemini API key (`images.md` § Vendors): an image
//! asked of a model's `generateContent` with the key in `x-goog-api-key`,
//! the image read from the answer's first inline part, a blocked prompt
//! or answer told in Google's words, and the models Google lists for the
//! key, read when asked.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    ErrorCode, HttpFailureRecord, ProviderFailure, Secret, json_body, model_words,
    images::{
        ImageAnswer, ImageCall, ImageCredential, ImageFailure, ImageMaker, ImageModelInfo, ImageRequest, ImageVendor,
        MadeImage, listed, unreadable, vendor_message,
    },
    read_http_failure,
};
use demi_shared_types::{B64Bytes, Clock, TokenUsage};
use futures_util::future::BoxFuture;
use reqwest::{
    StatusCode, Url,
    header::{HeaderMap, HeaderName},
};
use serde::{Deserialize, Serialize};

/// How failure messages name the vendor.
const LABEL: &str = "Google";

/// The most models one page of the listing holds, the most Google takes.
const PAGE_SIZE: &str = "1000";

/// A Gemini API key's image role.
pub(crate) struct KeyImages {
    key: Secret,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl KeyImages {
    pub(crate) fn new(key: Secret, base: Url, clock: Arc<dyn Clock>) -> Self {
        Self { key, base, clock }
    }

    fn call(&self) -> ImageCall<'_> {
        ImageCall {
            label: LABEL,
            credential: ImageCredential::Key { refused: refused_key },
            reader: read_http_failure,
            used_up,
            clock: &*self.clock,
            quota: None,
            answer: read_answer,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("x-goog-api-key"), self.key.header_value());
        headers
    }

    /// `…/models` under the base, then `segment` when given.
    fn models_url(&self, segment: Option<&str>) -> Url {
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

    /// Every page of the listing: the models whose ID has the word `image`
    /// or `banana` and that list `generateContent`.
    async fn list(&self, http: &reqwest::Client) -> Result<Vec<ImageModelInfo>, ImageFailure> {
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
                let words = model_words(id);
                let images = words.iter().any(|word| word == "image" || word == "banana");
                let generates = model.supported_generation_methods.iter().any(|method| method == "generateContent");
                if images && generates {
                    models.push(listed(ImageVendor::Google, id, None, model.description.clone()));
                }
            }
            match listing.next_page_token.filter(|token| !token.is_empty()) {
                Some(token) => page = Some(token),
                None => return Ok(models),
            }
        }
    }
}

impl ImageMaker for KeyImages {
    fn name(&self) -> &'static str {
        LABEL
    }

    fn vendor(&self) -> ImageVendor {
        ImageVendor::Google
    }

    fn models<'a>(&'a self, http: &'a reqwest::Client) -> BoxFuture<'a, Result<Vec<ImageModelInfo>, ImageFailure>> {
        Box::pin(self.list(http))
    }

    /// The model is the request's URL: Google answers no other.
    fn confirms_model(&self) -> bool {
        true
    }

    fn make<'a>(&'a self, http: &'a reqwest::Client, request: ImageRequest) -> BoxFuture<'a, ImageAnswer> {
        Box::pin(async move {
            let url = self.models_url(Some(&format!("{}:generateContent", request.model)));
            let headers = self.headers();
            let key = self.key.clone();
            self.call()
                .send(http, &url, encode(&request), |_| std::future::ready(Ok((headers.clone(), key.clone()))))
                .await
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Body<'a> {
    contents: [Content<'a>; 1],
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
struct Content<'a> {
    parts: Vec<Part<'a>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part<'a> {
    Text { text: &'a str },
    Image { inline_data: InlineData },
}

#[derive(Serialize)]
struct InlineData {
    mime_type: &'static str,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    response_modalities: [&'static str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    image_config: Option<ImageConfig>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageConfig {
    aspect_ratio: String,
}

fn encode(request: &ImageRequest) -> Bytes {
    let mut parts = vec![Part::Text { text: &request.prompt }];
    parts.extend(request.images.iter().map(|image| Part::Image {
        inline_data: InlineData { mime_type: image.media_type, data: image.base64() },
    }));
    let body = Body {
        contents: [Content { parts }],
        generation_config: GenerationConfig {
            response_modalities: ["IMAGE"],
            image_config: request.aspect.map(|aspect| ImageConfig { aspect_ratio: aspect.to_string() }),
        },
    };
    Bytes::from(json_body(&body))
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

/// A `generateContent` answer, as far as an image call reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default)]
    prompt_feedback: Option<PromptFeedback>,
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
    #[serde(default)]
    response_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    #[serde(default)]
    block_reason: Option<String>,
    #[serde(default)]
    block_reason_message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    #[serde(default)]
    content: Option<CandidateContent>,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    finish_message: Option<String>,
}

#[derive(Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<AnswerPart>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerPart {
    #[serde(default, alias = "inline_data")]
    inline_data: Option<AnswerData>,
}

#[derive(Deserialize)]
struct AnswerData {
    data: B64Bytes,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<u64>,
    #[serde(default)]
    candidates_token_count: Option<u64>,
}

/// The image of the first candidate's first inline part; a prompt Google
/// blocked, or a candidate it ended without an image, is a refusal in its
/// words.
fn read_answer(text: &[u8]) -> Result<MadeImage, ImageFailure> {
    let answer: Answer = serde_json::from_slice(text)
        .map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    if let Some(PromptFeedback { block_reason: Some(reason), block_reason_message }) = answer.prompt_feedback {
        let message = match block_reason_message {
            Some(said) => format!("{said} ({reason})"),
            None => format!("the prompt was blocked ({reason})"),
        };
        return Err(ImageFailure::Refused { message, code: Some(ErrorCode::Vendor(reason)) });
    }
    let candidate = answer.candidates.into_iter().next();
    let image = candidate
        .as_ref()
        .and_then(|candidate| candidate.content.as_ref())
        .and_then(|content| content.parts.iter().find_map(|part| part.inline_data.as_ref()));
    let Some(image) = image else {
        return Err(match candidate.and_then(|candidate| candidate.finish_reason.map(|reason| (reason, candidate.finish_message))) {
            Some((reason, said)) if reason != "STOP" => {
                let message = match said {
                    Some(said) => format!("{said} ({reason})"),
                    None => format!("the answer was stopped ({reason})"),
                };
                ImageFailure::Refused { message, code: Some(ErrorCode::Vendor(reason)) }
            }
            _ => unreadable("its answer held no image".to_owned()),
        });
    };
    let usage = answer.usage_metadata.and_then(|usage| {
        Some(TokenUsage {
            input_tokens: usage.prompt_token_count?,
            output_tokens: usage.candidates_token_count?,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        })
    });
    Ok(MadeImage {
        bytes: image.data.clone().into_bytes(),
        usage,
        vendor_id: answer.response_id,
    })
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

/// A refusal whose quota is the day's, or one the key's tier allows none
/// of: waiting minutes does not lift it. A quota of the minute is a rate
/// limit.
fn used_up(mut failure: ProviderFailure) -> ProviderFailure {
    #[derive(Deserialize)]
    struct Refusal {
        error: Error,
    }
    #[derive(Deserialize)]
    struct Error {
        #[serde(default)]
        details: Vec<Detail>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Detail {
        #[serde(default)]
        violations: Vec<Violation>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Violation {
        #[serde(default)]
        quota_id: Option<String>,
        #[serde(default)]
        quota_value: Option<String>,
    }
    let refusal = failure
        .diagnostics
        .as_deref()
        .and_then(HttpFailureRecord::read)
        .filter(|record| record.status == 429)
        .and_then(|record| serde_json::from_str::<Refusal>(&record.body).ok());
    let spent = refusal.is_some_and(|refusal| {
        refusal.error.details.iter().flat_map(|detail| &detail.violations).any(|violation| {
            violation.quota_id.as_deref().is_some_and(|id| id.contains("PerDay"))
                || violation.quota_value.as_deref() == Some("0")
        })
    });
    if spent {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}
