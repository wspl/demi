//! The one HTTP client of the requests the backend makes to provider
//! entries: a turn, a model list, an
//! image. A request whose host is a device id goes to the backend's
//! [`DeviceRoute`], which hands it to that device's runner when the device
//! is one of the entry owner's Hosts and the port is exposed; every other
//! request, and one naming no device of the owner's, goes to the network.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use reqwest::{IntoUrl, Url};
use serde::Serialize;

/// The address a request names, when its host may be a device's id: a
/// UUID, the form every device id has, over `http` or `https`, and the
/// port, the scheme's own when the address names none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAddress {
    pub device: String,
    pub secure: bool,
    pub port: u16,
}

impl DeviceAddress {
    /// What `url` names, when it may name a device.
    pub fn of(url: &Url) -> Option<Self> {
        let secure = match url.scheme() {
            "http" => false,
            "https" => true,
            _ => return None,
        };
        let host = url.host_str()?;
        uuid::Uuid::try_parse(host).ok()?;
        Some(Self {
            device: host.to_owned(),
            secure,
            port: url.port_or_known_default()?,
        })
    }
}

/// A request for a device's exposed port, as the route hands it on.
#[derive(Debug, Clone)]
pub struct DeviceRequest {
    pub address: DeviceAddress,
    pub method: Method,
    /// The path and query, from `/`.
    pub path: String,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
}

/// What an exposed port answered: its status and headers, and its body as
/// it arrives.
pub struct DeviceAnswer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: BoxStream<'static, std::io::Result<Bytes>>,
}

/// What the route made of a request.
pub enum Routed {
    /// The address names none of the owner's devices: an ordinary host,
    /// which the request goes to on the network.
    NotADevice,
    Answered(DeviceAnswer),
    Refused(DeviceRefusal),
}

/// Why a request to a device's port got no answer, in the words the user
/// reads (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceRefusal {
    /// The device has not exposed the port.
    #[error("Port {port} of {device} isn’t exposed.")]
    NotExposed { device: String, port: u16 },
    /// The device's runner is not connected, or the Cloud is stopped.
    #[error("{device} is offline.")]
    Offline { device: String },
    /// The runner made the request and it failed, as `0` says.
    #[error("{0}")]
    Failed(String),
}

/// The backend's way to the user's devices: it knows whose devices an
/// address names and whether the port is exposed.
pub trait DeviceRoute: Send + Sync + 'static {
    /// Sends `request` to its device when the device is one of `owner`'s
    /// Hosts.
    fn send(&self, owner: &str, request: DeviceRequest) -> BoxFuture<'static, Routed>;
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
    devices: Option<OwnerDevices>,
}

/// The route to the devices of the entry's owner.
#[derive(Clone)]
struct OwnerDevices {
    route: Arc<dyn DeviceRoute>,
    owner: Arc<str>,
}

impl fmt::Debug for ProviderHttp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderHttp")
            .field("owner", &self.devices.as_ref().map(|devices| &*devices.owner))
            .finish_non_exhaustive()
    }
}

impl ProviderHttp {
    /// A client that reaches no device: every address is the network's.
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, devices: None }
    }

    /// A client whose addresses naming one of `owner`'s devices go through
    /// `route`.
    pub fn with_devices(client: reqwest::Client, route: Arc<dyn DeviceRoute>, owner: &str) -> Self {
        Self {
            client,
            devices: Some(OwnerDevices {
                route,
                owner: owner.into(),
            }),
        }
    }

    /// The same way to the network and the devices on a client that follows
    /// no redirect, for a caller that follows them itself, choosing each
    /// hop's headers: reqwest sets its redirect policy per client, not per
    /// request. It builds a client, its own connection pool included.
    pub fn without_redirects(&self) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
        Ok(Self {
            client,
            devices: self.devices.clone(),
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
            devices: self.devices.clone(),
            inner: self.client.request(method, url),
        }
    }
}

/// A request being built, as reqwest's builder builds it.
pub struct RequestBuilder {
    devices: Option<OwnerDevices>,
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
            devices: self.devices,
            inner: change(self.inner),
        }
    }

    /// Sends the request: to the device its address names, when the
    /// entry's owner has it, otherwise to the network.
    pub async fn send(self) -> Result<reqwest::Response, SendError> {
        let (client, request) = self.inner.build_split();
        let request = request?;
        let routed = match (&self.devices, DeviceAddress::of(request.url())) {
            (Some(devices), Some(address)) => Some((devices, address)),
            _ => None,
        };
        let Some((devices, address)) = routed else {
            return Ok(client.execute(request).await?);
        };
        let body = match request.body() {
            None => None,
            // Every provider request's body is whole when it is sent; a
            // streamed one could not be read twice, and none is made.
            Some(body) => Some(Bytes::copy_from_slice(body.as_bytes().ok_or_else(|| {
                SendError::Device(DeviceRefusal::Failed(
                    "A request with a streamed body cannot reach a device".into(),
                ))
            })?)),
        };
        let mut headers = request.headers().clone();
        if let Some(body) = &body {
            headers.insert(http::header::CONTENT_LENGTH, HeaderValue::from(body.len()));
        }
        let url = request.url();
        let path = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let device_request = DeviceRequest {
            address,
            method: request.method().clone(),
            path,
            headers,
            body,
        };
        let sent = devices.route.send(&devices.owner, device_request);
        // The request's timeout bounds its answer's head, as it bounds a
        // network request's.
        let routed = match request.timeout() {
            Some(timeout) => tokio::time::timeout(*timeout, sent).await.unwrap_or_else(|_| {
                Routed::Refused(DeviceRefusal::Failed("The port did not answer in time".into()))
            }),
            None => sent.await,
        };
        match routed {
            Routed::NotADevice => Ok(client.execute(request).await?),
            Routed::Refused(refusal) => Err(SendError::Device(refusal)),
            Routed::Answered(answer) => {
                let mut response = http::Response::new(reqwest::Body::wrap_stream(answer.body));
                *response.status_mut() = answer.status;
                *response.headers_mut() = answer.headers;
                Ok(reqwest::Response::from(response))
            }
        }
    }
}

/// A file's download goes the provider's way, to the network or to one of
/// its owner's devices (`crates/shared-artifacts/src/download.rs`); its client follows
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
                // A device that refused may answer a later connection.
                SendError::Device(refusal) => demi_shared_artifacts::Unsent::Connection(
                    demi_shared_artifacts::Error::Download(refusal.to_string()),
                ),
            })
        })
    }
}
