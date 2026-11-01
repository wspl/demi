//! The one HTTP client of the requests the backend makes to provider entries: a
//! turn, a model list, an image.
//!
//! An entry that connects through one of its owner's devices sends every
//! request to the backend's [`DeviceRoute`], which hands it to that device's
//! runner; the runner makes it as any program on the device would. Every other
//! entry's requests go to the network from the backend.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use reqwest::{IntoUrl, Url};
use serde::Serialize;

/// A request an entry makes through a device, as the route hands it on.
#[derive(Debug, Clone)]
pub struct DeviceRequest {
    /// The device it goes out from, by id.
    pub device: String,
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
}

/// What the address answered a request made through a device: its status
/// and headers, and its body as it arrives.
pub struct DeviceAnswer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: BoxStream<'static, std::io::Result<Bytes>>,
}

/// Why a request made through a device got no answer, in the words the user
/// reads (`crates/backend-user-shard/src/shard/device_requests.rs`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceRefusal {
    /// The device is none of the owner's any more.
    #[error("The device this provider connects through was removed. Choose another in Connect Through.")]
    Removed,
    /// The device's runner is not connected, or the Cloud is stopped.
    #[error("{device} is offline.")]
    Offline { device: String },
    /// The runner made the request and it failed, as `0` says.
    #[error("{0}")]
    Failed(String),
}

/// The backend's way to the user's devices.
pub trait DeviceRoute: Send + Sync + 'static {
    /// Sends `request` from its device, one of `owner`'s Hosts.
    fn send(&self, owner: &str, request: DeviceRequest) -> BoxFuture<'static, Result<DeviceAnswer, DeviceRefusal>>;
}

/// Why a request got no answer.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    #[error(transparent)]
    Device(DeviceRefusal),
}

impl SendError {
    /// The error without the URL, which may hold an endpoint's secret.
    pub fn without_url(self) -> Self {
        match self {
            Self::Transport(error) => Self::Transport(error.without_url()),
            device => device,
        }
    }

    /// Whether the request could not even be built.
    pub fn is_builder(&self) -> bool {
        matches!(self, Self::Transport(error) if error.is_builder())
    }
}

/// The client provider entries' requests go through. Cloning it shares the
/// connection pool and the route.
#[derive(Clone)]
pub struct ProviderHttp {
    client: reqwest::Client,
    through: Option<Through>,
    /// Whether a request follows its redirects, as the client's policy says:
    /// the device's runner follows none, so a request through it follows
    /// them here.
    follows: bool,
}

/// The device of the entry's owner that its requests go out from.
#[derive(Clone)]
struct Through {
    route: Arc<dyn DeviceRoute>,
    owner: Arc<str>,
    device: Arc<str>,
}

impl fmt::Debug for ProviderHttp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderHttp")
            .field("through", &self.through.as_ref().map(|through| &*through.device))
            .finish_non_exhaustive()
    }
}

impl ProviderHttp {
    /// A client whose requests go to the network from the backend.
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, through: None, follows: true }
    }

    /// A client whose requests go out from `owner`'s device `device`, through
    /// `route`.
    pub fn through_device(client: reqwest::Client, route: Arc<dyn DeviceRoute>, owner: &str, device: &str) -> Self {
        Self {
            client,
            through: Some(Through {
                route,
                owner: owner.into(),
                device: device.into(),
            }),
            follows: true,
        }
    }

    /// The same way, to the network or through the device, on a client that
    /// follows no redirect, for a caller that follows them itself, choosing each hop's
    /// headers.
    ///
    /// reqwest sets its redirect policy per client, not per request. It builds
    /// a client, its own connection pool included.
    pub fn without_redirects(&self) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
        Ok(Self {
            client,
            through: self.through.clone(),
            follows: false,
        })
    }

    pub fn get(&self, url: impl IntoUrl) -> RequestBuilder {
        self.request(Method::GET, url)
    }

    pub fn post(&self, url: impl IntoUrl) -> RequestBuilder {
        self.request(Method::POST, url)
    }

    pub fn delete(&self, url: impl IntoUrl) -> RequestBuilder {
        self.request(Method::DELETE, url)
    }

    pub fn request(&self, method: Method, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder {
            through: self.through.clone(),
            follows: self.follows,
            inner: self.client.request(method, url),
        }
    }
}

/// A request being built, as reqwest's builder builds it.
pub struct RequestBuilder {
    through: Option<Through>,
    follows: bool,
    inner: reqwest::RequestBuilder,
}

impl RequestBuilder {
    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.map(|inner| inner.header(key, value))
    }

    pub fn headers(self, headers: HeaderMap) -> Self {
        self.map(|inner| inner.headers(headers))
    }

    pub fn body(self, body: impl Into<reqwest::Body>) -> Self {
        self.map(|inner| inner.body(body))
    }

    pub fn form<T: Serialize + ?Sized>(self, form: &T) -> Self {
        self.map(|inner| inner.form(form))
    }

    pub fn timeout(self, timeout: Duration) -> Self {
        self.map(|inner| inner.timeout(timeout))
    }

    fn map(self, change: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder) -> Self {
        Self {
            through: self.through,
            follows: self.follows,
            inner: change(self.inner),
        }
    }

    /// Sends the request: through the entry's device when it connects
    /// through one, otherwise to the network.
    pub async fn send(self) -> Result<reqwest::Response, SendError> {
        let (client, request) = self.inner.build_split();
        let request = request?;
        let Some(through) = &self.through else {
            return Ok(client.execute(request).await?);
        };
        let body = match request.body() {
            None => None,
            // Every provider request's body is whole when it is sent; a
            // streamed one could not be read twice, and none is made.
            Some(body) => Some(Bytes::copy_from_slice(body.as_bytes().ok_or_else(|| {
                SendError::Device(DeviceRefusal::Failed(
                    "A request with a streamed body cannot go through a device".into(),
                ))
            })?)),
        };
        let mut headers = request.headers().clone();
        if let Some(body) = &body {
            headers.insert(http::header::CONTENT_LENGTH, HeaderValue::from(body.len()));
        }
        let mut hop = DeviceRequest {
            device: through.device.to_string(),
            method: request.method().clone(),
            url: request.url().clone(),
            headers,
            body,
        };
        for _ in 0..=REDIRECTS {
            let sent = through.route.send(&through.owner, hop.clone());
            // The request's timeout bounds each answer's head, as it bounds a
            // network request's.
            let answered = match request.timeout() {
                Some(timeout) => tokio::time::timeout(*timeout, sent).await.unwrap_or_else(|_| {
                    Err(DeviceRefusal::Failed("The address did not answer in time".into()))
                }),
                None => sent.await,
            };
            let answer = answered.map_err(SendError::Device)?;
            let next = if self.follows { redirected(&hop, &answer) } else { None };
            let Some(next) = next else {
                let mut response = http::Response::new(reqwest::Body::wrap_stream(answer.body));
                *response.status_mut() = answer.status;
                *response.headers_mut() = answer.headers;
                return Ok(reqwest::Response::from(response));
            };
            hop = next;
        }
        Err(SendError::Device(DeviceRefusal::Failed(format!(
            "The address redirected more than {REDIRECTS} times"
        ))))
    }
}

/// How many redirects a request follows, as reqwest's default policy does.
pub(crate) const REDIRECTS: usize = 10;

/// Whether a redirect of `status` is followed with `method` and its body, as
/// browsers and reqwest follow it: a 303, and a 301 or a 302 of a `POST`, is
/// followed with a `GET` and no body; a 307 or a 308 keeps both.
pub(crate) fn redirect_keeps_method(status: StatusCode, method: &Method) -> bool {
    matches!(status, StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT)
        || (status != StatusCode::SEE_OTHER && *method != Method::POST)
}

/// The request `answer` redirects `hop` to, as reqwest's default policy
/// follows it; none for an answer that is no redirect, or names no address.
/// A hop to another host or port goes without the credentials reqwest drops
/// there.
fn redirected(hop: &DeviceRequest, answer: &DeviceAnswer) -> Option<DeviceRequest> {
    if !answer.status.is_redirection() {
        return None;
    }
    let location = answer.headers.get(http::header::LOCATION)?.to_str().ok()?;
    let url = hop.url.join(location).ok()?;
    let mut next = DeviceRequest { url, ..hop.clone() };
    if !redirect_keeps_method(answer.status, &hop.method) {
        next.method = Method::GET;
        next.body = None;
        for name in [
            http::header::CONTENT_LENGTH,
            http::header::CONTENT_TYPE,
            http::header::CONTENT_ENCODING,
            http::header::TRANSFER_ENCODING,
        ] {
            next.headers.remove(name);
        }
    }
    if next.url.host_str() != hop.url.host_str() || next.url.port_or_known_default() != hop.url.port_or_known_default() {
        for name in [
            http::header::AUTHORIZATION,
            http::header::COOKIE,
            HeaderName::from_static("cookie2"),
            http::header::PROXY_AUTHORIZATION,
            http::header::WWW_AUTHENTICATE,
        ] {
            next.headers.remove(name);
        }
    }
    Some(next)
}

/// A file's download goes the provider's way, to the network or through
/// its owner's device (`crates/shared-artifacts/src/download.rs`); its client follows
/// no redirect, as [`ProviderHttp::without_redirects`] makes it.
impl demi_shared_artifacts::Fetch for ProviderHttp {
    fn fetch(
        &self,
        url: reqwest::Url,
        headers: HeaderMap,
    ) -> futures_util::future::BoxFuture<'_, Result<reqwest::Response, demi_shared_artifacts::Unsent>> {
        Box::pin(async move {
            self.get(url).headers(headers).send().await.map_err(|error| match error {
                SendError::Transport(error) => demi_shared_artifacts::Unsent::of(error),
                // A removed device never answers again; one that was offline
                // or failed may answer a later connection.
                SendError::Device(refusal @ DeviceRefusal::Removed) => demi_shared_artifacts::Unsent::Refused(
                    demi_shared_artifacts::Error::Download(refusal.to_string()),
                ),
                SendError::Device(refusal) => demi_shared_artifacts::Unsent::Connection(
                    demi_shared_artifacts::Error::Download(refusal.to_string()),
                ),
            })
        })
    }
}
