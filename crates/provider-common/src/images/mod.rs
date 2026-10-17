//! The image role of a provider (`images.md` § Vendors): a provider that
//! makes images names its vendor, lists its image models and makes one
//! image for its account or key; the backend chooses the entry and the
//! account. The request a vendor takes is the provider's own; what every
//! vendor's call has in common is here: the sending within the deadline,
//! how a refusal reads, and the answer of the OpenAI-shaped image APIs.

pub mod openai;
pub mod xai;

use std::{fmt, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use demi_shared_types::{B64Bytes, Clock, QuotaWindow, Timestamp, TokenUsage};
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use reqwest::Url;
use serde::Deserialize;

use crate::media_catalog::{Catalog, MediaKind, Subscription};
use crate::{
    ErrorCode, FailureReader, HttpFailureRecord, ProviderFailure, Secret,
    credentials::{AuthFailure, AuthReason},
    http_failure,
    quota::{Observation, ProviderQuota},
};

/// How long one image call may take: the token's refresh, the request and
/// the reading of the answer or the refusal. A listing of a key's models
/// takes as long at most.
pub const IMAGE_TIMEOUT: Duration = Duration::from_secs(120);

/// A vendor whose image models a provider makes, by its id in the catalog
/// of media models: a subscription's (ChatGPT's are OpenAI's, Grok's
/// xAI's) or a key's. A compatible entry's maker has none: what it makes is
/// whatever its endpoint serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImageVendor {
    OpenAi,
    Google,
    Xai,
}

impl ImageVendor {
    pub const ALL: [Self; 3] = [Self::OpenAi, Self::Google, Self::Xai];

    /// Its id in the catalog and on models.dev.
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::Xai => "xai",
        }
    }

    /// The vendor of catalog id `id`, when it makes images.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|vendor| vendor.id() == id)
    }

    /// How the user knows it.
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Google => "Google",
            Self::Xai => "xAI",
        }
    }
}

/// The image models `subscription` makes, as the catalog lists them for
/// it: a subscription's vendor lists none to it.
pub fn subscription_models(catalog: &Catalog, subscription: Subscription) -> Vec<ImageModelInfo> {
    catalog
        .models
        .iter()
        .filter(|model| model.kind == MediaKind::Image && model.subscriptions.contains(&subscription))
        .map(|model| ImageModelInfo::id(&model.id))
        .collect()
}

/// A provider's image role.
pub trait ImageMaker: Send + Sync {
    /// How the user knows who answers, such as `ChatGPT` or `OpenAI`.
    fn name(&self) -> &str;

    /// Whose models it makes; none for a compatible entry's endpoint,
    /// whose models are the ones its user added.
    fn vendor(&self) -> Option<ImageVendor>;

    /// The image models it offers: those `catalog` lists for a
    /// subscription, whose vendor lists none to it, what the vendor lists
    /// for a key, read when asked with the HTTP client of the shard that
    /// asks, or the models a compatible entry's user added.
    fn models<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<ImageModelInfo>, ImageFailure>>;

    /// Whether the model that answered is known to be the one asked for.
    /// A vendor whose answer does not name it, the ChatGPT backend's, is
    /// shown as asked, not confirmed (`images.md` § Commands).
    fn confirms_model(&self) -> bool;

    /// Makes one image for the provider's account or key, sending with
    /// `http`, the client of the shard that asks; `catalog` says what the
    /// model takes.
    fn make<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        catalog: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, ImageAnswer>;
}

/// An image model a provider offers, as its listing names it: what the
/// catalog of media models states of it is read by the backend, which
/// joins the listings of every entry (`media.md` § What the agent is told).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageModelInfo {
    pub id: String,
    /// The name the listing gives, such as a compatible entry's model's.
    pub name: Option<String>,
    /// The release date the listing gives, `YYYY-MM-DD`.
    pub released: Option<String>,
    /// What the listing says the model is for.
    pub description: Option<String>,
}

impl ImageModelInfo {
    /// A model the listing names by its ID alone.
    pub fn id(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: None,
            released: None,
            description: None,
        }
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
    /// The image as a `data:` URL, as the OpenAI-shaped APIs take an
    /// edit's images.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.base64())
    }

    /// The image's bytes in base64, as Gemini's inline data takes them.
    pub fn base64(&self) -> String {
        STANDARD.encode(&self.bytes)
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
    /// The account or key is used up, until the moment the vendor named,
    /// if any.
    UsageLimit { retry_at: Option<Timestamp> },
    /// The account's sign-in could not be used or renewed.
    SignIn(AuthFailure),
    /// The vendor refused the entry's API key, in its own words; `code` is
    /// the failure's, `auth_expired` or `permission_denied`.
    KeyRefused { message: String, code: ErrorCode },
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
            Self::KeyRefused { code, .. } => Some(code.clone()),
            Self::Refused { code, .. } | Self::Shape { code, .. } | Self::Unanswered { code, .. } => code.clone(),
        };
        code.map_or_else(|| "failed".to_owned(), |code| code.as_str().to_owned())
    }
}

/// What an image request is sent with.
#[derive(Clone, Copy)]
pub enum ImageCredential {
    /// A subscription account's token: a request refused with HTTP 401 is
    /// sent again once with the token renewed, and a second refusal is a
    /// sign-in that cannot be used, named by the account's `family`.
    Account { family: &'static str },
    /// An API key, never renewed: a refusal `refused` reads as the key's,
    /// from its status and body, fails as [`ImageFailure::KeyRefused`].
    Key { refused: fn(StatusCode, &str) -> bool },
}

/// How a provider's image requests are sent and their failures read: the
/// label its failure records carry, the reader that finds a moment in one,
/// its reading of a used-up account or key, its clock, the account's
/// quota, what the request is sent with, and how its answer is read.
pub struct ImageCall<'a> {
    pub label: &'static str,
    pub credential: ImageCredential,
    pub reader: FailureReader,
    pub used_up: fn(ProviderFailure) -> ProviderFailure,
    pub clock: &'a dyn Clock,
    /// The account's quota, which each response's windows update, as a
    /// turn's do (`usage-and-quota.md` § Vendor quota); none for a key.
    pub quota: Option<&'a ProviderQuota>,
    /// Reads the image of a successful answer's body.
    pub answer: fn(&[u8]) -> Result<MadeImage, ImageFailure>,
}

impl ImageCall<'_> {
    /// Posts `body`, a JSON object, to `url` with the headers `credentials`
    /// gives and reads the one image of the answer, all within
    /// [`IMAGE_TIMEOUT`]. For an account, a request refused with HTTP 401
    /// is sent again once, with the credentials renewed past the token it
    /// was refused with; `credentials` answers the headers and that token,
    /// none for a key, which is never renewed.
    pub async fn send<F, Fut>(&self, http: &crate::ProviderHttp, url: &Url, body: Bytes, credentials: F) -> ImageAnswer
    where
        F: FnMut(Option<Secret>) -> Fut,
        Fut: Future<Output = Result<(HeaderMap, Option<Secret>), AuthFailure>>,
    {
        let mut observed = Vec::new();
        let call = self.attempts(http, url, body, credentials, &mut observed);
        let result = within_deadline(call).await;
        ImageAnswer { result, observed }
    }

    /// The body of a successful `GET` of `url` with `headers`, a listing of
    /// a key's models, within [`IMAGE_TIMEOUT`]; a refusal reads as a
    /// call's does.
    pub async fn get(&self, http: &crate::ProviderHttp, url: &Url, headers: HeaderMap) -> Result<Bytes, ImageFailure> {
        within_deadline(async {
            let response = http.get(url.clone()).headers(headers).send().await.map_err(transport)?;
            if !response.status().is_success() {
                return Err(self.refusal(response).await);
            }
            response.bytes().await.map_err(transport)
        })
        .await
    }

    async fn attempts<F, Fut>(
        &self,
        http: &crate::ProviderHttp,
        url: &Url,
        body: Bytes,
        mut credentials: F,
        observed: &mut Vec<QuotaWindow>,
    ) -> Result<MadeImage, ImageFailure>
    where
        F: FnMut(Option<Secret>) -> Fut,
        Fut: Future<Output = Result<(HeaderMap, Option<Secret>), AuthFailure>>,
    {
        let mut refused: Option<Secret> = None;
        let mut retried = false;
        loop {
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
            if let Some(quota) = self.quota {
                let reported = quota.observe(Observation::Response {
                    status,
                    headers: response.headers(),
                });
                // The row merges a window reported twice, as a turn's does.
                observed.extend(reported.into_iter().flatten());
            }
            let renews = matches!(self.credential, ImageCredential::Account { .. });
            if status == StatusCode::UNAUTHORIZED && renews && !retried {
                // Dropping the answer releases its connection.
                refused = token;
                retried = true;
                continue;
            }
            if !status.is_success() {
                return Err(self.refusal(response).await);
            }
            let text = response.bytes().await.map_err(transport)?;
            return (self.answer)(&text);
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
        let body = diagnostics
            .and_then(HttpFailureRecord::read)
            .map(|record| record.body)
            .unwrap_or_default();
        let said = vendor_message(&body);
        match self.credential {
            ImageCredential::Account { family } if status == StatusCode::UNAUTHORIZED => {
                return ImageFailure::SignIn(AuthFailure::new(family, AuthReason::SignedOut));
            }
            ImageCredential::Key { refused } if refused(status, &body) => {
                let message = said.unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                // A 401 or a 403 has the code its status gives; a vendor's
                // own sign of a key it does not know, as Google's 400
                // `API_KEY_INVALID`, is a refused credential.
                let code = failure
                    .code
                    .filter(|code| matches!(code, ErrorCode::AuthExpired | ErrorCode::PermissionDenied))
                    .unwrap_or(ErrorCode::AuthExpired);
                return ImageFailure::KeyRefused { message, code };
            }
            _ => {}
        }
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

/// `call`'s result, or a failure once [`IMAGE_TIMEOUT`] has passed.
async fn within_deadline<T>(call: impl Future<Output = Result<T, ImageFailure>>) -> Result<T, ImageFailure> {
    match tokio::time::timeout(IMAGE_TIMEOUT, call).await {
        Ok(result) => result,
        Err(_) => Err(ImageFailure::Unanswered {
            cause: format!("no answer in {} s", IMAGE_TIMEOUT.as_secs()),
            code: Some(ErrorCode::Overloaded),
        }),
    }
}

/// The failure of a request that did not reach the vendor or broke off.
/// A port the device has not exposed is `port_not_exposed`, which waiting
/// cannot fix, and an offline device `device_offline`.
fn transport(error: impl Into<crate::SendError>) -> ImageFailure {
    let error = error.into();
    let code = match &error {
        crate::SendError::Device(crate::DeviceRefusal::NotExposed { .. }) => ErrorCode::PortNotExposed,
        crate::SendError::Device(crate::DeviceRefusal::Offline { .. }) => ErrorCode::DeviceOffline,
        _ => ErrorCode::Overloaded,
    };
    ImageFailure::Unanswered {
        cause: crate::failure::transport_cause(error),
        code: Some(code),
    }
}

/// The failure of an answer that is not what the vendor documents.
pub fn unreadable(cause: String) -> ImageFailure {
    ImageFailure::Unanswered { cause, code: None }
}

/// What a vendor said in a refusal's body: `error.message`, `error` when
/// it is text, or `message`; a body of none of these is its text, trimmed.
pub fn vendor_message(body: &str) -> Option<String> {
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

/// The answer of the OpenAI-shaped image APIs, OpenAI's, xAI's and the
/// subscriptions': the image as base64, and the counts the vendor reports.
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

/// Reads the answer of an OpenAI-shaped image API: `data[0].b64_json`, and
/// `usage`'s counts when it gives both.
pub fn read_data_answer(text: &[u8]) -> Result<MadeImage, ImageFailure> {
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

/// A Unix time in seconds, as OpenAI's and xAI's listings give `created`,
/// as the date `YYYY-MM-DD` it falls on in UTC.
pub fn created_date(seconds: i64) -> Option<String> {
    let moment = jiff::Timestamp::from_second(seconds).ok()?;
    Some(moment.strftime("%Y-%m-%d").to_string())
}
