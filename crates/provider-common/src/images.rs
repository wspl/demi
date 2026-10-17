//! The image role of a provider (`images.md` § Vendors): a provider that
//! makes images states its image models and makes one image for its
//! account; the backend chooses the entry and the account. The request a
//! vendor takes is the provider's own, and what every vendor's answer has
//! in common is read here: one image as `data[0].b64_json`, the counts it
//! reported, and how a refusal reads.

use std::{fmt, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use demi_shared_types::{B64Bytes, Clock, QuotaWindow, Timestamp, TokenUsage};
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use reqwest::Url;
use serde::Deserialize;

use crate::{
    ErrorCode, FailureReader, HttpFailureRecord, ProviderFailure, Secret,
    credentials::{AuthFailure, AuthReason},
    http_failure,
    quota::{Observation, ProviderQuota},
};

/// How long one image call may take: the token's refresh, the request and
/// the reading of the answer or the refusal.
pub const IMAGE_TIMEOUT: Duration = Duration::from_secs(120);

/// A provider's image role.
pub trait ImageMaker: Send + Sync {
    /// How the user knows the vendor, such as `ChatGPT`.
    fn vendor(&self) -> &'static str;

    /// The image models, each with what the vendor publishes of it.
    /// Neither vendor lists them, so the list is the provider's own.
    fn models(&self) -> &'static [ImageModelInfo];

    /// The model a call that names none takes, one of [`Self::models`].
    fn default_model(&self) -> &'static str;

    /// Whether the model that answered is known to be the one asked for.
    /// A vendor whose answer does not name it, the ChatGPT backend's, is
    /// shown as asked, not confirmed (`images.md` § Commands).
    fn confirms_model(&self) -> bool;

    /// Makes one image for the provider's account, sending with `http`, the
    /// client of the shard that asks.
    fn make<'a>(&'a self, http: &'a reqwest::Client, request: ImageRequest) -> BoxFuture<'a, ImageAnswer>;
}

/// An image model a provider offers, with what its vendor publishes of
/// it, so the agent picks a model and a shape from the list
/// (`images.md` § Commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageModelInfo {
    pub id: &'static str,
    /// The vendor's release date, `YYYY-MM-DD`: a dated ID's is its
    /// snapshot's, an alias's that of the model it named when the list was
    /// made.
    pub released: &'static str,
    /// What the vendor says the model is for, in one line.
    pub description: &'static str,
    /// The sizes or shapes it takes, and what the vendor says of its
    /// formats.
    pub takes: &'static str,
}

impl ImageModelInfo {
    pub const fn new(id: &'static str, released: &'static str, description: &'static str, takes: &'static str) -> Self {
        Self { id, released, description, takes }
    }
}

/// How one image call ended, with the windows of the account's quota its
/// responses reported, a refusal's included, which the account's snapshot
/// took in already and the call's usage ledger row keeps.
#[derive(Debug, Clone)]
pub struct ImageAnswer {
    pub result: Result<MadeImage, ImageFailure>,
    pub observed: Vec<QuotaWindow>,
}

/// The shape asked for, such as 16:9, which the plugin read and checks
/// the result against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aspect {
    pub width: u32,
    pub height: u32,
}

impl Aspect {
    /// A size of this shape whose longer side is `long` pixels, each side a
    /// multiple of 16.
    pub fn size(&self, long: u32) -> (u32, u32) {
        let side = |part: u32, whole: u32| {
            let exact = f64::from(long) * f64::from(part) / f64::from(whole);
            ((exact / 16.0).round() as u32).max(1) * 16
        };
        if self.width >= self.height {
            (long, side(self.height, self.width))
        } else {
            (side(self.width, self.height), long)
        }
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.width, self.height)
    }
}

/// One image to make.
#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub model: String,
    pub prompt: String,
    pub aspect: Option<Aspect>,
    pub transparent: bool,
    /// The images an edit changes, combines or follows; none for a
    /// generation.
    pub images: Vec<InputImage>,
}

/// An image an edit sends.
#[derive(Debug, Clone)]
pub struct InputImage {
    /// `image/png`, `image/jpeg` or `image/webp`.
    pub media_type: &'static str,
    pub bytes: Bytes,
}

impl InputImage {
    /// The image as a `data:` URL, as both vendors take an edit's images.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, STANDARD.encode(&self.bytes))
    }
}

/// The image a vendor made, with what the usage ledger keeps of the call.
#[derive(Debug, Clone)]
pub struct MadeImage {
    pub bytes: Bytes,
    /// The counts the vendor reported; none when it reported none.
    pub usage: Option<TokenUsage>,
    /// The vendor's id of the image, when it gives one.
    pub vendor_id: Option<String>,
}

/// Why no image came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageFailure {
    /// The account is used up, until the moment the vendor named, if any.
    UsageLimit { retry_at: Option<Timestamp> },
    /// The account's sign-in could not be used or renewed.
    SignIn(AuthFailure),
    /// The vendor refused the prompt or an image, in its own words.
    Refused { message: String, code: Option<ErrorCode> },
    /// The vendor refused the shape asked for, naming the shapes it takes,
    /// in its own order.
    Shape { takes: Vec<String>, code: Option<ErrorCode> },
    /// The vendor failed, or did not answer in time.
    Unanswered { cause: String, code: Option<ErrorCode> },
}

impl ImageFailure {
    /// How the call ended, as its usage ledger row says.
    pub fn outcome(&self) -> String {
        let code = match self {
            Self::UsageLimit { .. } => Some(ErrorCode::UsageLimit),
            Self::SignIn(failure) => failure.failure().code,
            Self::Refused { code, .. } | Self::Shape { code, .. } | Self::Unanswered { code, .. } => code.clone(),
        };
        code.map_or_else(|| "failed".to_owned(), |code| code.as_str().to_owned())
    }
}

/// How a provider's image requests are sent and their failures read: the
/// label its failure records carry, the reader that finds a moment in one,
/// its reading of a used-up account, its clock, and the account's quota.
pub struct ImageCall<'a> {
    pub label: &'static str,
    /// How the account's sign-in failures name the family.
    pub family: &'static str,
    pub reader: FailureReader,
    pub used_up: fn(ProviderFailure) -> ProviderFailure,
    pub clock: &'a dyn Clock,
    /// The account's quota, which each response's windows update, as a
    /// turn's do (`usage-and-quota.md` § Vendor quota).
    pub quota: &'a ProviderQuota,
}

impl ImageCall<'_> {
    /// Posts `body`, a JSON object, to `url` with the headers `credentials`
    /// gives and reads the one image of the answer, all within
    /// [`IMAGE_TIMEOUT`]. A request refused with HTTP 401 is sent again once, with
    /// the credentials renewed past the token it was refused with;
    /// `credentials` answers the headers and that token.
    pub async fn send<F, Fut>(&self, http: &reqwest::Client, url: &Url, body: Bytes, credentials: F) -> ImageAnswer
    where
        F: FnMut(Option<Secret>) -> Fut,
        Fut: Future<Output = Result<(HeaderMap, Secret), AuthFailure>>,
    {
        let mut observed = Vec::new();
        let call = self.attempts(http, url, body, credentials, &mut observed);
        let result = match tokio::time::timeout(IMAGE_TIMEOUT, call).await {
            Ok(result) => result,
            Err(_) => Err(ImageFailure::Unanswered {
                cause: format!("no answer in {} s", IMAGE_TIMEOUT.as_secs()),
                code: Some(ErrorCode::Overloaded),
            }),
        };
        ImageAnswer { result, observed }
    }

    async fn attempts<F, Fut>(
        &self,
        http: &reqwest::Client,
        url: &Url,
        body: Bytes,
        mut credentials: F,
        observed: &mut Vec<QuotaWindow>,
    ) -> Result<MadeImage, ImageFailure>
    where
        F: FnMut(Option<Secret>) -> Fut,
        Fut: Future<Output = Result<(HeaderMap, Secret), AuthFailure>>,
    {
        let mut refused: Option<Secret> = None;
        loop {
            let retried = refused.is_some();
            let (mut headers, token) = credentials(refused.take()).await.map_err(ImageFailure::SignIn)?;
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            let response = http
                .post(url.clone())
                .headers(headers)
                .body(body.clone())
                .send()
                .await
                .map_err(transport)?;
            let status = response.status();
            let reported = self.quota.observe(Observation::Response {
                status,
                headers: response.headers(),
            });
            // The row merges a window reported twice, as a turn's does.
            observed.extend(reported.into_iter().flatten());
            if status == StatusCode::UNAUTHORIZED && !retried {
                // Dropping the answer releases its connection.
                refused = Some(token);
                continue;
            }
            if !status.is_success() {
                return Err(self.refusal(response).await);
            }
            let text = response.bytes().await.map_err(transport)?;
            return read_answer(&text);
        }
    }

    /// The failure of a refused request.
    async fn refusal(&self, response: reqwest::Response) -> ImageFailure {
        let status = response.status();
        let failure = (self.used_up)(http_failure(response, self.label, self.reader, self.clock).await);
        let diagnostics = failure.diagnostics.as_deref();
        if failure.code == Some(ErrorCode::UsageLimit) {
            let retry_at = diagnostics.and_then(|diagnostics| (self.reader)(diagnostics, self.clock.now()).retry_at);
            return ImageFailure::UsageLimit { retry_at };
        }
        if status == StatusCode::UNAUTHORIZED {
            return ImageFailure::SignIn(AuthFailure::new(self.family, AuthReason::SignedOut));
        }
        let body = diagnostics
            .and_then(HttpFailureRecord::read)
            .map(|record| record.body)
            .unwrap_or_default();
        let said = vendor_message(&body);
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::REQUEST_TIMEOUT {
            let cause = match said {
                Some(said) => format!("HTTP {}: {said}", status.as_u16()),
                None => format!("HTTP {}", status.as_u16()),
            };
            return ImageFailure::Unanswered { cause, code: failure.code };
        }
        let message = said.unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        ImageFailure::Refused { message, code: failure.code }
    }
}

/// The failure of a request that did not reach the vendor or broke off.
fn transport(error: reqwest::Error) -> ImageFailure {
    ImageFailure::Unanswered {
        cause: crate::failure::transport_cause(error),
        code: Some(ErrorCode::Overloaded),
    }
}

/// What a vendor said in a refusal's body: `error.message`, `error` when
/// it is text, or `message`; a body of none of these is its text, trimmed.
fn vendor_message(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Refusal {
        #[serde(default)]
        error: Option<serde_json::Value>,
        #[serde(default)]
        message: Option<String>,
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Ok(refusal) = serde_json::from_str::<Refusal>(trimmed) else {
        return Some(trimmed.to_owned());
    };
    let said = match refusal.error {
        Some(serde_json::Value::String(text)) => Some(text),
        Some(serde_json::Value::Object(error)) => error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        _ => None,
    };
    Some(said.or(refusal.message).unwrap_or_else(|| trimmed.to_owned()))
}

/// The answer both vendors give: the image as base64, and the counts the
/// ChatGPT backend reports.
#[derive(Deserialize)]
struct Answer {
    data: Vec<Datum>,
    #[serde(default)]
    usage: Option<AnswerUsage>,
}

#[derive(Deserialize)]
struct Datum {
    b64_json: B64Bytes,
    #[serde(default)]
    generation_id: Option<String>,
}

#[derive(Deserialize)]
struct AnswerUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

fn read_answer(text: &[u8]) -> Result<MadeImage, ImageFailure> {
    let unreadable = |cause: String| ImageFailure::Unanswered { cause, code: None };
    let answer: Answer = serde_json::from_slice(text)
        .map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    let Some(datum) = answer.data.into_iter().next() else {
        return Err(unreadable("its answer held no image".to_owned()));
    };
    let usage = answer.usage.and_then(|usage| {
        Some(TokenUsage {
            input_tokens: usage.input_tokens?,
            output_tokens: usage.output_tokens?,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        })
    });
    Ok(MadeImage {
        bytes: datum.b64_json.into_bytes(),
        usage,
        vendor_id: datum.generation_id,
    })
}
