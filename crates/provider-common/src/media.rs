//! What every kind of a provider's media role has in common (`media.md`):
//! the vendor whose models it makes, the models it lists, how a request to
//! the vendor is sent within its deadline with the account's or the key's
//! credentials, and how a refusal reads. The image and video roles
//! ([`crate::images`], [`crate::videos`]) build on it; the backend chooses
//! the entry and the account.

use std::time::Duration;

use bytes::Bytes;
use demi_shared_types::{Clock, QuotaWindow, Timestamp};
use futures_util::future::BoxFuture;
use http::{HeaderMap, StatusCode};
use reqwest::Url;
use serde::Deserialize;

use crate::media_catalog::{Catalog, MediaKind, Subscription};
use crate::{
    ErrorCode, FailureReader, HttpFailureRecord, ProviderFailure, Secret,
    credentials::{AuthFailure, AuthReason},
    http_failure,
    quota::{Observation, ProviderQuota},
};

/// How long one request to a vendor may take: the token's refresh, the
/// request and the reading of the answer or the refusal. An image is one
/// request; a video's ask, each read of its state and its download are one
/// each (`videos.md` § A call is a job).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// A vendor whose media models a provider makes, by its id in the catalog
/// of media models: a subscription's (ChatGPT's are OpenAI's, Grok's
/// xAI's) or a key's. A compatible entry's maker has none: what it makes is
/// whatever its endpoint serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MediaVendor {
    OpenAi,
    Google,
    Xai,
}

impl MediaVendor {
    pub const ALL: [Self; 3] = [Self::OpenAi, Self::Google, Self::Xai];

    /// Its id in the catalog and on models.dev.
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::Xai => "xai",
        }
    }

    /// The vendor of catalog id `id`, when it makes media.
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

/// The models of `kind` that `subscription` makes, as the catalog lists
/// them for it: a subscription's vendor lists none to it.
pub fn subscription_models(catalog: &Catalog, subscription: Subscription, kind: MediaKind) -> Vec<MediaModelInfo> {
    catalog
        .models
        .iter()
        .filter(|model| model.kind == kind && model.subscriptions.contains(&subscription))
        .map(|model| MediaModelInfo::id(&model.id))
        .collect()
}

/// What every kind of media role says of itself, which the backend lists
/// and chooses entries by (`media.md` § What the agent is told).
pub trait MediaMaker: Send + Sync {
    /// How the user knows who answers, such as `ChatGPT` or `OpenAI`.
    fn name(&self) -> &str;

    /// Whose models it makes; none for a compatible entry's endpoint,
    /// whose models are the ones its user added.
    fn vendor(&self) -> Option<MediaVendor>;

    /// The models of its kind it offers: those `catalog` lists for a
    /// subscription, whose vendor lists none to it, what the vendor lists
    /// for a key, read when asked with the HTTP client of the shard that
    /// asks, or the models a compatible entry's user added.
    fn models<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>>;

    /// Whether the model that answered is known to be the one asked for.
    /// A vendor whose answer does not name it, the ChatGPT backend's, is
    /// shown as asked, not confirmed (`images.md` § Commands).
    fn confirms_model(&self) -> bool {
        true
    }
}

/// A media model a provider offers, as its listing names it: what the
/// catalog of media models states of it is read by the backend, which
/// joins the listings of every entry (`media.md` § What the agent is told).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaModelInfo {
    pub id: String,
    /// The name the listing gives, such as a compatible entry's model's.
    pub name: Option<String>,
    /// The release date the listing gives, `YYYY-MM-DD`.
    pub released: Option<String>,
    /// What the listing says the model is for.
    pub description: Option<String>,
}

impl MediaModelInfo {
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

/// How one request to a vendor ended, with the windows of the account's
/// quota its responses reported, a refusal's included, which the account's
/// snapshot took in already and the call's usage ledger row keeps.
#[derive(Debug, Clone)]
pub struct MediaAnswer<T> {
    pub result: Result<T, MediaFailure>,
    pub observed: Vec<QuotaWindow>,
}

impl<T> MediaAnswer<T> {
    /// The answer with its result changed by `change`, its windows kept.
    pub fn and_then<U>(self, change: impl FnOnce(T) -> Result<U, MediaFailure>) -> MediaAnswer<U> {
        MediaAnswer {
            result: self.result.and_then(change),
            observed: self.observed,
        }
    }
}

/// Why a vendor did not answer with what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaFailure {
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

impl MediaFailure {
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

/// What a request to a vendor is sent with.
#[derive(Clone, Copy)]
pub enum MediaCredential {
    /// A subscription account's token: a request refused with HTTP 401 is
    /// sent again once with the token renewed, and a second refusal is a
    /// sign-in that cannot be used, named by the account's `family`.
    Account { family: &'static str },
    /// An API key, never renewed: a refusal `refused` reads as the key's,
    /// from its status and body, fails as [`MediaFailure::KeyRefused`].
    Key { refused: fn(StatusCode, &str) -> bool },
}

/// How a provider's media requests are sent and their failures read: the
/// label its failure records carry, the reader that finds a moment in one,
/// its reading of a used-up account or key, its clock, the account's
/// quota and what the request is sent with.
pub struct MediaCall<'a> {
    pub label: &'static str,
    pub credential: MediaCredential,
    pub reader: FailureReader,
    pub used_up: fn(ProviderFailure) -> ProviderFailure,
    pub clock: &'a dyn Clock,
    /// The account's quota, which each response's windows update, as a
    /// turn's do (`usage-and-quota.md` § Vendor quota); none for a key.
    pub quota: Option<&'a ProviderQuota>,
}

/// A successful answer: its status, such as 202 for a job still running,
/// and its body.
#[derive(Debug, Clone)]
pub struct Answered {
    pub status: StatusCode,
    pub body: Bytes,
}

impl MediaCall<'_> {
    /// Posts `body`, a JSON object, to `url` with the headers `credentials`
    /// gives, and answers the body of the vendor's success; see
    /// [`MediaCall::send`].
    pub async fn post_json<F, Fut>(&self, http: &crate::ProviderHttp, url: &Url, body: Bytes, credentials: F) -> MediaAnswer<Bytes>
    where
        F: FnMut(Option<Secret>) -> Fut,
        Fut: Future<Output = Result<(HeaderMap, Option<Secret>), AuthFailure>>,
    {
        let request = |headers: HeaderMap| {
            http.post(url.clone())
                .headers(headers)
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(body.clone())
        };
        self.send(request, credentials).await.and_then(|answered| Ok(answered.body))
    }

    /// Sends the request `request` builds with the headers `credentials`
    /// gives, and answers the vendor's success, all within
    /// [`REQUEST_TIMEOUT`]. For an account, a request refused with HTTP 401
    /// is sent again once, with the credentials renewed past the token it
    /// was refused with; `credentials` answers the headers and that token,
    /// none for a key, which is never renewed.
    pub async fn send<B, F, Fut>(&self, request: B, credentials: F) -> MediaAnswer<Answered>
    where
        B: Fn(HeaderMap) -> crate::RequestBuilder,
        F: FnMut(Option<Secret>) -> Fut,
        Fut: Future<Output = Result<(HeaderMap, Option<Secret>), AuthFailure>>,
    {
        let mut observed = Vec::new();
        let call = self.attempts(request, credentials, &mut observed);
        let result = within_deadline(call).await;
        MediaAnswer { result, observed }
    }

    /// The body of a successful `GET` of `url` with `headers`, such as a
    /// listing of a key's models, within [`REQUEST_TIMEOUT`]; a refusal
    /// reads as a call's does.
    pub async fn get(&self, http: &crate::ProviderHttp, url: &Url, headers: HeaderMap) -> Result<Bytes, MediaFailure> {
        within_deadline(async {
            let response = http.get(url.clone()).headers(headers).send().await.map_err(transport)?;
            if !response.status().is_success() {
                return Err(self.refusal(response).await);
            }
            response.bytes().await.map_err(transport)
        })
        .await
    }

    async fn attempts<B, F, Fut>(
        &self,
        request: B,
        mut credentials: F,
        observed: &mut Vec<QuotaWindow>,
    ) -> Result<Answered, MediaFailure>
    where
        B: Fn(HeaderMap) -> crate::RequestBuilder,
        F: FnMut(Option<Secret>) -> Fut,
        Fut: Future<Output = Result<(HeaderMap, Option<Secret>), AuthFailure>>,
    {
        let mut refused: Option<Secret> = None;
        let mut retried = false;
        loop {
            let (headers, token) = credentials(refused.take()).await.map_err(MediaFailure::SignIn)?;
            let response = request(headers).send().await.map_err(transport)?;
            let status = response.status();
            if let Some(quota) = self.quota {
                let reported = quota.observe(Observation::Response {
                    status,
                    headers: response.headers(),
                });
                // The row merges a window reported twice, as a turn's does.
                observed.extend(reported.into_iter().flatten());
            }
            let renews = matches!(self.credential, MediaCredential::Account { .. });
            if status == StatusCode::UNAUTHORIZED && renews && !retried {
                // Dropping the answer releases its connection.
                refused = token;
                retried = true;
                continue;
            }
            if !status.is_success() {
                return Err(self.refusal(response).await);
            }
            let body = response.bytes().await.map_err(transport)?;
            return Ok(Answered { status, body });
        }
    }

    /// The failure of a refused request.
    pub(crate) async fn refusal(&self, response: reqwest::Response) -> MediaFailure {
        let status = response.status();
        let failure = (self.used_up)(http_failure(response, self.label, self.reader, self.clock).await);
        let diagnostics = failure.diagnostics.as_deref();
        if failure.code == Some(ErrorCode::UsageLimit) {
            let retry_at = diagnostics.and_then(|diagnostics| (self.reader)(diagnostics, self.clock.now()).retry_at);
            return MediaFailure::UsageLimit { retry_at };
        }
        let body = diagnostics
            .and_then(HttpFailureRecord::read)
            .map(|record| record.body)
            .unwrap_or_default();
        let said = vendor_message(&body);
        match self.credential {
            MediaCredential::Account { family } if status == StatusCode::UNAUTHORIZED => {
                return MediaFailure::SignIn(AuthFailure::new(family, AuthReason::SignedOut));
            }
            MediaCredential::Key { refused } if refused(status, &body) => {
                let message = said.unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                // A 401 or a 403 has the code its status gives; a vendor's
                // own sign of a key it does not know, as Google's 400
                // `API_KEY_INVALID`, is a refused credential.
                let code = failure
                    .code
                    .filter(|code| matches!(code, ErrorCode::AuthExpired | ErrorCode::PermissionDenied))
                    .unwrap_or(ErrorCode::AuthExpired);
                return MediaFailure::KeyRefused { message, code };
            }
            _ => {}
        }
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::REQUEST_TIMEOUT {
            let cause = match said {
                Some(said) => format!("HTTP {}: {said}", status.as_u16()),
                None => format!("HTTP {}", status.as_u16()),
            };
            return MediaFailure::Unanswered { cause, code: failure.code };
        }
        let message = said.unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        MediaFailure::Refused { message, code: failure.code }
    }
}

/// `call`'s result, or a failure once [`REQUEST_TIMEOUT`] has passed.
pub(crate) async fn within_deadline<T>(call: impl Future<Output = Result<T, MediaFailure>>) -> Result<T, MediaFailure> {
    match tokio::time::timeout(REQUEST_TIMEOUT, call).await {
        Ok(result) => result,
        Err(_) => Err(MediaFailure::Unanswered {
            cause: format!("no answer in {} s", REQUEST_TIMEOUT.as_secs()),
            code: Some(ErrorCode::Overloaded),
        }),
    }
}

/// The failure of a request that did not reach the vendor or broke off.
/// A port the device has not exposed is `port_not_exposed`, which waiting
/// cannot fix, and an offline device `device_offline`.
pub(crate) fn transport(error: impl Into<crate::SendError>) -> MediaFailure {
    let error = error.into();
    let code = match &error {
        crate::SendError::Device(crate::DeviceRefusal::NotExposed { .. }) => ErrorCode::PortNotExposed,
        crate::SendError::Device(crate::DeviceRefusal::Offline { .. }) => ErrorCode::DeviceOffline,
        _ => ErrorCode::Overloaded,
    };
    MediaFailure::Unanswered {
        cause: crate::failure::transport_cause(error),
        code: Some(code),
    }
}

/// The failure of an answer that is not what the vendor documents.
pub fn unreadable(cause: String) -> MediaFailure {
    MediaFailure::Unanswered { cause, code: None }
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

/// A Unix time in seconds, as OpenAI's and xAI's listings give `created`,
/// as the date `YYYY-MM-DD` it falls on in UTC.
pub fn created_date(seconds: i64) -> Option<String> {
    let moment = jiff::Timestamp::from_second(seconds).ok()?;
    Some(moment.strftime("%Y-%m-%d").to_string())
}
