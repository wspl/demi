//! The deployment's namespace at the preview domain (`preview.md` § The
//! preview domain service). The backend registers one when it first starts,
//! with the origins it serves its pages on, keeps it as a control record with
//! its secret sealed by the vault, renews it once a day, replaces its origins
//! when they change, and registers a new one when the old one is gone. None
//! of it holds the backend up: until a namespace is registered the pages are
//! told none, and a request that fails is retried with backoff.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_backend_database::StorageError;
use demi_backend_database::preview::PreviewRecord;
use demi_backend_page_sync::Part;
use demi_backend_providers::vault::seal::Row;
use demi_provider_common::Secret;
use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::PreviewNamespace;
use demi_web_api_protocol::state::{PreviewDomain, PreviewScheme};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use url::{Host, Url};

use crate::services::Services;
use crate::tuning::PreviewTuning;

/// How long a namespace lives after its creation or its last renewal, as
/// the service keeps it: 90 days.
const LIFETIME_MS: i64 = 90 * DAY_MS;
/// How often the backend renews its namespace: once a day.
const RENEWAL_MS: i64 = DAY_MS;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// How long one request to the service may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// `DEMI_PREVIEW_DOMAIN`: a domain name, such as `demi-preview.dev`. One
/// under `.localhost` may carry a port and is served over HTTP, which Chrome
/// treats as a secure context there; any other is HTTPS on its default port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewDomainName {
    /// The host, and the port if it has one, as written.
    authority: String,
    host: String,
}

/// Why a text is not a preview domain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreviewDomainError {
    #[error("must be a lowercase domain name, such as demi-preview.dev, with nothing around it")]
    NotDomain,
    #[error("only a domain under .localhost may carry a port")]
    Port,
}

impl FromStr for PreviewDomainName {
    type Err = PreviewDomainError;

    fn from_str(text: &str) -> Result<Self, PreviewDomainError> {
        let url = Url::parse(&format!("http://{text}/")).map_err(|_| PreviewDomainError::NotDomain)?;
        let Some(Host::Domain(host)) = url.host() else {
            return Err(PreviewDomainError::NotDomain);
        };
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        };
        // The text names a host and a port only: no user, path, query or
        // fragment, and no letters the parser changed.
        if authority != text || url.path() != "/" {
            return Err(PreviewDomainError::NotDomain);
        }
        let domain = Self {
            authority,
            host: host.to_owned(),
        };
        if url.port().is_some() && !domain.local() {
            return Err(PreviewDomainError::Port);
        }
        Ok(domain)
    }
}

impl fmt::Display for PreviewDomainName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.authority)
    }
}

impl PreviewDomainName {
    /// Whether the domain is under `.localhost`, development's.
    fn local(&self) -> bool {
        self.host == "localhost" || self.host.ends_with(".localhost")
    }

    /// The scheme the domain is served over, which the pages are told.
    fn scheme(&self) -> PreviewScheme {
        if self.local() {
            PreviewScheme::Http
        } else {
            PreviewScheme::Https
        }
    }

    /// The address of `path` at the domain's root.
    fn url(&self, path: &str) -> String {
        format!("{}://{}{path}", self.scheme().as_str(), self.authority)
    }

    /// The client the service is asked with. A `.localhost` name is the
    /// loopback interface, as Chrome resolves it, whatever the system's
    /// resolver says, and is never reached through a proxy.
    fn client(&self) -> reqwest::Result<reqwest::Client> {
        let builder = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
        let builder = if self.local() {
            // Port 0 keeps the port the address names.
            builder
                .no_proxy()
                .resolve(&self.host, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        } else {
            builder
        };
        builder.build()
    }
}

/// An origin the web app is served on that the preview domain admits, as
/// `URL.origin` writes it: HTTPS, or HTTP on `localhost` or `127.0.0.1`,
/// whose host holds only letters, digits, dots and hyphens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOrigin(String);

/// Why a text is not an origin the preview domain admits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "must be origins such as https://demi.example.com or http://127.0.0.1:18934: HTTPS, or HTTP on localhost or 127.0.0.1, with nothing after the port"
)]
pub struct PageOriginError;

impl FromStr for PageOrigin {
    type Err = PageOriginError;

    fn from_str(text: &str) -> Result<Self, PageOriginError> {
        let url = Url::parse(text).map_err(|_| PageOriginError)?;
        let origin = Self::of(&url).ok_or(PageOriginError)?;
        if origin.0 != text.trim_end_matches('/') {
            return Err(PageOriginError);
        }
        Ok(origin)
    }
}

impl PageOrigin {
    /// The origin of `url` when the preview domain admits it.
    pub fn of(url: &Url) -> Option<Self> {
        let host = url.host_str()?;
        if !host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        {
            return None;
        }
        let admitted = match url.scheme() {
            "https" => true,
            "http" => matches!(host, "localhost" | "127.0.0.1"),
            _ => false,
        };
        admitted.then(|| Self(url.origin().ascii_serialization()))
    }
}

/// What the registration is configured with.
#[derive(Debug, Clone)]
pub struct PreviewSettings {
    pub domain: PreviewDomainName,
    /// The origins besides the public URL's that serve the web app, such as
    /// a development server's.
    pub origins: Vec<PageOrigin>,
    pub tuning: PreviewTuning,
}

/// The preview domain and the namespace the pages are told, none until one
/// is registered. Cloning it shares it.
#[derive(Clone, Default)]
pub struct CurrentPreview(Arc<Mutex<Option<PreviewDomain>>>);

impl CurrentPreview {
    pub fn get(&self) -> Option<PreviewDomain> {
        // A section only replaces the value, so a poisoned one is whole.
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set(&self, preview: PreviewDomain) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(preview);
    }
}

/// Keeps the deployment's namespace registered with the origins it serves
/// its pages on, until the task running it is dropped. The backend starts
/// it once it listens, which sets the public URL.
pub async fn keep_registered(services: Arc<Services>, settings: PreviewSettings) {
    let public = services
        .public_url
        .get()
        .expect("the backend listens before the namespace is registered")
        .url()
        .clone();
    let mut origins: Vec<String> = Vec::new();
    for origin in PageOrigin::of(&public).into_iter().chain(settings.origins) {
        if !origins.contains(&origin.0) {
            origins.push(origin.0);
        }
    }
    if origins.is_empty() {
        tracing::info!(
            public_url = %public,
            "the web app is served on no secure-context origin, where no preview can run, so no preview namespace is registered"
        );
        return;
    }
    let client = match settings.domain.client() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the preview domain's HTTP client cannot start; no preview namespace is registered"
            );
            return;
        }
    };
    Registration {
        services,
        domain: settings.domain,
        origins,
        client,
        tuning: settings.tuning,
        held: Held::Unread,
    }
    .run()
    .await;
}

/// The registration's state between its steps.
struct Registration {
    services: Arc<Services>,
    domain: PreviewDomainName,
    /// The origins to register, the public URL's first.
    origins: Vec<String>,
    client: reqwest::Client,
    tuning: PreviewTuning,
    held: Held,
}

/// What namespace the backend holds.
enum Held {
    /// The control record was not read yet.
    Unread,
    /// None: the next step registers one.
    Nothing,
    Namespace(Registered),
}

/// A namespace the backend holds, with its secret open.
struct Registered {
    namespace: PreviewNamespace,
    secret: Secret,
    expires_at: Timestamp,
    origins: Vec<String>,
}

/// Why a step did not complete; the step is tried again after a backoff.
#[derive(Debug, thiserror::Error)]
enum StepError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("the preview domain could not be asked: {0}")]
    Request(#[from] reqwest::Error),
    #[error("the preview domain answered {status}: {body}")]
    Refused { status: StatusCode, body: String },
    #[error("the preview domain's answer is not understood: {0}")]
    Answer(#[from] serde_json::Error),
}

/// What the service answers a renewal or a change of origins.
enum Answer {
    Changed(Changed),
    /// The namespace expired, or the service does not know it or its
    /// secret: it cannot be used again, and a new one is registered.
    Gone,
}

#[derive(Serialize)]
struct Origins<'a> {
    origins: &'a [String],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    namespace: PreviewNamespace,
    secret: Secret,
    expires_at: Timestamp,
}

/// What a renewal or a change of origins leaves registered.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Changed {
    origins: Vec<String>,
    expires_at: Timestamp,
}

impl Registration {
    async fn run(mut self) {
        let mut backoff = self.tuning.first_retry;
        loop {
            match self.step().await {
                Ok(None) => backoff = self.tuning.first_retry,
                Ok(Some(wait)) => {
                    backoff = self.tuning.first_retry;
                    tokio::time::sleep(wait).await;
                }
                Err(error) => {
                    tracing::warn!(
                        domain = %self.domain,
                        error = &error as &dyn std::error::Error,
                        retry_in = ?backoff,
                        "the preview namespace was not registered or renewed"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(self.tuning.last_retry);
                }
            }
        }
    }

    /// Takes the next step the namespace needs; answers how long nothing
    /// is due, or none when another step may be due at once.
    async fn step(&mut self) -> Result<Option<Duration>, StepError> {
        let current = match &self.held {
            Held::Unread => {
                self.held = self.load().await?;
                return Ok(None);
            }
            Held::Nothing => {
                self.held = Held::Namespace(self.create().await?);
                return Ok(None);
            }
            Held::Namespace(current) => current,
        };
        let now = self.services.clock.now().as_millisecond();
        if current.expires_at.as_millisecond() <= now {
            self.held = Held::Nothing;
            return Ok(None);
        }
        let due = current.expires_at.as_millisecond() - LIFETIME_MS + RENEWAL_MS;
        let answer = if current.origins != self.origins {
            let path = format!("/api/v1/namespaces/{}/origins", current.namespace);
            let body = serde_json::to_vec(&Origins {
                origins: &self.origins,
            })?;
            self.ask(current, Method::PUT, &path, Some(body)).await?
        } else if now >= due {
            let path = format!("/api/v1/namespaces/{}/renew", current.namespace);
            self.ask(current, Method::POST, &path, None).await?
        } else {
            let until_due = Duration::from_millis(u64::try_from(due - now).unwrap_or(0));
            return Ok(Some(until_due.min(self.tuning.check)));
        };
        let Answer::Changed(changed) = answer else {
            tracing::info!(
                namespace = %current.namespace,
                "the preview namespace is gone; a new one is registered"
            );
            self.held = Held::Nothing;
            return Ok(None);
        };
        let registered = Registered {
            namespace: current.namespace.clone(),
            secret: current.secret.clone(),
            expires_at: changed.expires_at,
            origins: changed.origins,
        };
        self.keep(&registered).await?;
        self.held = Held::Namespace(registered);
        Ok(None)
    }

    /// Reads the namespace an earlier start kept, and tells the pages while
    /// it lives. One whose secret no longer opens, as after the instance
    /// secret changed, cannot be used, and a new one replaces it.
    async fn load(&self) -> Result<Held, StepError> {
        let Some(record) = self.services.control.preview_namespace().await? else {
            return Ok(Held::Nothing);
        };
        let opened = self
            .services
            .vault
            .key()
            .open(Row::PreviewSecret(&record.namespace), &record.sealed_secret)
            .ok()
            .and_then(|secret| String::from_utf8(secret).ok())
            .and_then(|secret| Secret::try_from(secret).ok());
        let Some(secret) = opened else {
            tracing::warn!(
                namespace = %record.namespace,
                "the preview namespace's secret does not open; a new namespace is registered"
            );
            return Ok(Held::Nothing);
        };
        if record.expires_at > self.services.clock.now() {
            self.publish(&record.namespace);
        }
        Ok(Held::Namespace(Registered {
            namespace: record.namespace,
            secret,
            expires_at: record.expires_at,
            origins: record.origins,
        }))
    }

    /// Registers a new namespace with the origins, keeps it, and tells the
    /// pages.
    async fn create(&self) -> Result<Registered, StepError> {
        let body = serde_json::to_vec(&Origins {
            origins: &self.origins,
        })?;
        let response = self
            .client
            .post(self.domain.url("/api/v1/namespaces"))
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        let created: Created = serde_json::from_slice(&success(response).await?)?;
        let registered = Registered {
            namespace: created.namespace,
            secret: created.secret,
            expires_at: created.expires_at,
            origins: self.origins.clone(),
        };
        self.keep(&registered).await?;
        self.publish(&registered.namespace);
        tracing::info!(
            domain = %self.domain,
            namespace = %registered.namespace,
            origins = ?registered.origins,
            "registered the preview namespace"
        );
        Ok(registered)
    }

    /// Sends a request about `current` with its secret.
    async fn ask(
        &self,
        current: &Registered,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Answer, StepError> {
        let mut request = self
            .client
            .request(method, self.domain.url(path))
            .header(AUTHORIZATION, current.secret.bearer());
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }
        let response = request.send().await?;
        if matches!(
            response.status(),
            StatusCode::GONE | StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED
        ) {
            return Ok(Answer::Gone);
        }
        Ok(Answer::Changed(serde_json::from_slice(
            &success(response).await?,
        )?))
    }

    /// Keeps `registered` as the control record, its secret sealed for it.
    async fn keep(&self, registered: &Registered) -> Result<(), StepError> {
        let sealed_secret = self.services.vault.key().seal(
            Row::PreviewSecret(&registered.namespace),
            registered.secret.expose().as_bytes(),
        );
        self.services
            .control
            .set_preview_namespace(PreviewRecord {
                namespace: registered.namespace.clone(),
                sealed_secret,
                expires_at: registered.expires_at,
                origins: registered.origins.clone(),
            })
            .await?;
        Ok(())
    }

    /// Tells every page the namespace to embed previews from.
    fn publish(&self, namespace: &PreviewNamespace) {
        self.services.preview.set(PreviewDomain {
            scheme: self.domain.scheme(),
            domain: self.domain.to_string(),
            namespace: namespace.clone(),
        });
        self.services.sync.mark_everyone(&Part::Preview);
    }
}

/// The body of a successful answer, or the refusal it is.
async fn success(response: reqwest::Response) -> Result<bytes::Bytes, StepError> {
    let status = response.status();
    let body = response.bytes().await?;
    if status.is_success() {
        return Ok(body);
    }
    Err(StepError::Refused {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}
