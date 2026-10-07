//! The preview engine (`preview.md` § The preview engine): a request a
//! preview document made, handed over by the relay with its real address,
//! its receiving environment and its initiator, made upstream as the logical
//! browser would make it, and answered rewritten. One engine serves every
//! conversation of the Host; it keeps nothing per request once it answered,
//! and no label.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use demi_command_package_browser_protocol::preview::{
    FILES_VERSION, PreviewClient, PreviewCredentials, PreviewEnvironment, PreviewMode,
    PreviewRequest,
};
use demi_preview_rewrite::address::{self, Context, Environment, Role, site_of};
use demi_preview_rewrite::attributes::{REMOVED_CONTENT_POLICIES, rewrite_refresh};
use demi_preview_rewrite::boot::{WorkerScript, boot_data};
use demi_preview_rewrite::css::rewrite_stylesheet;
use demi_preview_rewrite::html::{HtmlOptions, rewrite_html};
use demi_preview_rewrite::javascript::{ScriptOptions, SourceMapMode, rewrite_javascript};
use http::{HeaderMap, Method, header};
use serde_json::json;
use sha2::{Digest, Sha256, Sha384, Sha512};
use tokio_util::task::AbortOnDropHandle;
use url::Url;

use crate::cookies::{self, Api, Jar, JarCookie};
use crate::headers::{self, chrome_headers, preflight_headers};
use crate::policy::{apply_redirect_hsts, corp_allows, cors_allows, document_isolation_for};
use crate::upstream::{Space, Upstream, UpstreamError};

/// Upstream response headers that would act on the preview origin instead
/// of the site, or that the engine writes itself.
const DROPPED_RESPONSE_HEADERS: [&str; 28] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "content-encoding",
    "set-cookie",
    "strict-transport-security",
    "alt-svc",
    "clear-site-data",
    "report-to",
    "reporting-endpoints",
    "nel",
    "public-key-pins",
    "public-key-pins-report-only",
    "expect-ct",
    "set-login",
    "accept-ch",
    "critical-ch",
    "service-worker-allowed",
    "cross-origin-resource-policy",
    "access-control-allow-origin",
    "access-control-allow-credentials",
    "access-control-allow-methods",
];
const DROPPED_CORS_RESPONSE_HEADERS: [&str; 3] = [
    "access-control-allow-headers",
    "access-control-expose-headers",
    "access-control-max-age",
];
/// Request headers a page cannot set or that are CORS-safelisted: the rest
/// make a request non-simple.
const SAFELISTED_REQUEST_HEADERS: [&str; 4] = ["accept", "accept-language", "content-language", "range"];
/// Scripts the engine rewrites, where the runtime runs: a document's, and a
/// worker's own script and the modules it imports, which load as `worker` or
/// `sharedworker`. A worklet runs the site's script as written.
const SCRIPT_DESTINATIONS: [&str; 3] = ["script", "worker", "sharedworker"];
/// The paths of a preview origin the engine answers itself: the document's
/// cookies (`preview.md` § Cookies) and a debugger's source maps
/// (§ Rewriting).
const COOKIE_PATH: &str = "/__demi/host/cookie";
const SOURCE_MAP_PATH: &str = "/__demi/host/source-map";
/// How long the jar waits after a change before it writes its file, so a
/// burst of changes is one write.
const SAVE_DELAY: Duration = Duration::from_millis(500);
const REWRITE_CACHE_CHARACTERS: usize = 128 * 1024 * 1024;

/// Where previews live, as the relay's `hello` names them: the preview
/// domain's scheme and the domain, the deployment's namespace and the Host,
/// which every label the engine computes is a digest of.
#[derive(Clone, Debug)]
pub(crate) struct Place {
    pub scheme: String,
    pub domain: String,
    pub namespace: String,
    pub host: String,
}

impl Place {
    pub fn context(&self, document: Environment) -> Context {
        let top_level = is_top_level(&document);
        Context {
            scheme: self.scheme.clone(),
            domain: self.domain.clone(),
            namespace: self.namespace.clone(),
            host: self.host.clone(),
            boot: format!("/__demi/v{FILES_VERSION}/boot.html"),
            client: format!("/__demi/v{FILES_VERSION}/client.js"),
            // The runtime of the engine's own release, which the rewriter's
            // output calls (`preview.md` § The forwarder and the relay).
            runtime: format!("/__demi/page/runtime/{}.js", env!("CARGO_PKG_VERSION")),
            document,
            top_level,
            opaque: false,
            labels: Mutex::default(),
        }
    }

    /// The CSP of every preview document: no network outside the preview
    /// domain (`preview.md` § Response headers).
    fn content_policy(&self) -> String {
        let scheme = &self.scheme;
        // A page may build a preview address from the host alone, without
        // the port.
        let host = self.domain.split(':').next().unwrap_or_default();
        let previews = if host == self.domain {
            format!("{scheme}://*.{host}")
        } else {
            format!("{scheme}://*.{} {scheme}://*.{host}", self.domain)
        };
        format!("default-src {previews} data: blob: 'unsafe-inline' 'unsafe-eval'")
    }
}

/// A document environment is a preview tab's top frame when nothing above it
/// is cross-site and its top is its own site; a same-site nested frame is
/// counted as top-level too (the labels do not record frames).
pub(crate) fn is_top_level(environment: &Environment) -> bool {
    !environment.cross && environment.top == site_of(&environment.origin)
}

pub(crate) fn environment_of(environment: &PreviewEnvironment) -> Environment {
    Environment {
        origin: environment.origin.clone(),
        top: environment.top.clone(),
        cross: environment.cross,
    }
}

pub(crate) fn preview_environment(environment: Environment) -> PreviewEnvironment {
    PreviewEnvironment {
        origin: environment.origin,
        top: environment.top,
        cross: environment.cross,
    }
}

/// The logical request's environment for SameSite: the top-level site, the
/// initiator's origin (empty for the user's own navigation, `null` for an
/// unknown or tainted one), and whether the document it is for is nested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fields {
    pub top: String,
    pub origin: String,
    pub nested: bool,
}

/// SameSite context per RFC 6265bis § 5.2, from the request's logical
/// environment.
fn same_site_context(fields: &Fields, target: &Url, navigation: bool, method: &Method) -> cookies::Context {
    let target_site = site_of(&origin_of(target));
    if navigation && !fields.nested {
        if fields.origin.is_empty() || site_of(&fields.origin) == target_site {
            return cookies::Context::Strict;
        }
        return if matches!(*method, Method::GET | Method::HEAD) {
            cookies::Context::Lax
        } else {
            cookies::Context::CrossSite
        };
    }
    if target_site == fields.top && site_of(&fields.origin) == fields.top {
        cookies::Context::Strict
    } else {
        cookies::Context::CrossSite
    }
}

/// `document.cookie` sees SameSite cookies only when its document is
/// same-site with the top.
fn script_context(environment: &Environment, target: &Url) -> cookies::Context {
    if site_of(&origin_of(target)) == environment.top {
        cookies::Context::Strict
    } else {
        cookies::Context::CrossSite
    }
}

/// A WebSocket's SameSite context: a subresource of its page.
pub(crate) fn socket_context(environment: &Environment, target: &Url) -> cookies::Context {
    let fields = Fields {
        top: environment.top.clone(),
        origin: environment.origin.clone(),
        nested: true,
    };
    same_site_context(&fields, target, false, &Method::GET)
}

fn fetch_site(initiator: &str, target: &str) -> &'static str {
    if initiator.is_empty() {
        "none"
    } else if initiator == target {
        "same-origin"
    } else if initiator != "null" && site_of(initiator) == site_of(target) {
        "same-site"
    } else {
        "cross-site"
    }
}

/// The policy's Referer for a logical source and target (Referrer Policy
/// § 3).
fn referrer_under(source: &str, policy: &str, target: &Url) -> Option<String> {
    let mut source = Url::parse(source).ok()?;
    source.set_fragment(None);
    // An address without credentials refuses neither change.
    let _ = source.set_username("");
    let _ = source.set_password(None);
    let downgrade = source.scheme() == "https" && target.scheme() == "http";
    let same_origin = source.origin() == target.origin();
    let origin_only = format!("{}/", origin_of(&source));
    match policy {
        "no-referrer" => None,
        "same-origin" => same_origin.then(|| source.into()),
        "origin" => Some(origin_only),
        "strict-origin" => (!downgrade).then_some(origin_only),
        "unsafe-url" => Some(source.into()),
        "no-referrer-when-downgrade" => (!downgrade).then(|| source.into()),
        "origin-when-cross-origin" => Some(if same_origin { source.into() } else { origin_only }),
        _ if downgrade => None,
        _ => Some(if same_origin { source.into() } else { origin_only }),
    }
}

/// A Referrer-Policy header whose effective token is `no-referrer` becomes
/// `same-origin`: the forwarder's navigations must name their initiator
/// (`preview.md` § CORS, CORP and Referer).
fn adapted_referrer_policy(value: &str) -> String {
    let effective = value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .next_back()
        .unwrap_or_default();
    if effective.eq_ignore_ascii_case("no-referrer") {
        "same-origin".into()
    } else {
        value.to_owned()
    }
}

/// The policy's directives but `sync-xhr`: the runtime reads `blob:` and
/// `data:` scripts with synchronous requests, so the browser must allow them
/// in every preview document; the runtime refuses the page's own synchronous
/// requests whatever the policy says (`preview.md` § Response headers).
fn without_sync_xhr(name: &str, value: &str) -> String {
    let (separator, joiner) = if name == "feature-policy" { (';', "; ") } else { (',', ", ") };
    let directive_name = |directive: &str| {
        directive
            .split(|character: char| character == '=' || character.is_whitespace())
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    value
        .split(separator)
        .map(str::trim)
        .filter(|directive| !directive.is_empty() && directive_name(directive) != "sync-xhr")
        .collect::<Vec<_>>()
        .join(joiner)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Html,
    JavaScript,
    Css,
}

fn rewrite_kind(destination: &str, content_type: &str, navigation: bool) -> Option<Kind> {
    let lower = content_type.to_ascii_lowercase();
    if navigation {
        return (lower.contains("text/html") || lower.contains("application/xhtml+xml")).then_some(Kind::Html);
    }
    if SCRIPT_DESTINATIONS.contains(&destination) {
        return ["javascript", "ecmascript", "jscript", "livescript", "text/x-js"]
            .iter()
            .any(|kind| lower.contains(kind))
            .then_some(Kind::JavaScript);
    }
    (destination == "style" && lower.contains("text/css")).then_some(Kind::Css)
}

fn header_text<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn origin_of(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// A request as the stream hands it to the engine, its body read whole.
pub(crate) struct Fetch {
    pub environment: Environment,
    pub request: PreviewRequest,
    pub client: PreviewClient,
    pub body: Bytes,
}

/// The head and body of an answer.
pub(crate) struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub labels: BTreeMap<String, Environment>,
    pub body: AnswerBody,
}

pub(crate) enum AnswerBody {
    Full(Bytes),
    /// Read from upstream as the relay pulls it.
    Upstream(wreq::Response),
}

/// Why a request failed: the page sees a network error.
#[derive(Debug)]
pub(crate) struct Failure(pub String);

impl Failure {
    fn refused(reason: &str, target: &Url) -> Self {
        tracing::debug!(url = target.as_str(), reason, "preview request refused");
        Self(format!("refused: {reason}"))
    }
}

/// Why the engine could not start.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("the preview cookie jar {0}: {1}")]
    Jar(PathBuf, std::io::Error),
    /// The jar's file is not one the engine wrote; it is left for a person
    /// to look at, never replaced.
    #[error("the preview cookie jar {0} is not a cookie jar: {1}")]
    CorruptJar(PathBuf, serde_json::Error),
    #[error("the preview's upstream client: {0}")]
    Client(wreq::Error),
}

/// The jar and its file, written a moment after each change.
struct Persisted {
    jar: Mutex<Jar>,
    file: PathBuf,
    dirty: AtomicBool,
    changed: tokio::sync::Notify,
    /// One write at a time, so an older snapshot never replaces a newer one.
    writing: tokio::sync::Mutex<()>,
}

impl Persisted {
    /// Changes the jar; `change` answers its result and whether the jar
    /// changed.
    fn change<R>(&self, change: impl FnOnce(&mut Jar) -> (R, bool)) -> R {
        let (result, changed) = change(&mut self.jar.lock().expect("jar lock"));
        if changed {
            self.dirty.store(true, Ordering::SeqCst);
            self.changed.notify_one();
        }
        result
    }

    fn read<R>(&self, read: impl FnOnce(&Jar) -> R) -> R {
        read(&self.jar.lock().expect("jar lock"))
    }

    /// Writes the jar when it changed since its last write: whole, to a new
    /// file that then replaces the old one, so a crash leaves one or the
    /// other.
    async fn save(&self) -> std::io::Result<()> {
        let _turn = self.writing.lock().await;
        if !self.dirty.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        let bytes = self.read(Jar::save);
        let file = self.file.clone();
        let written = tokio::task::spawn_blocking(move || write_atomically(&file, &bytes))
            .await
            .unwrap_or_else(|joined| Err(std::io::Error::other(joined)));
        if written.is_err() {
            // The next change, or the engine's close, tries again.
            self.dirty.store(true, Ordering::SeqCst);
        }
        written
    }
}

fn write_atomically(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let directory = file.parent().unwrap_or(Path::new("."));
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    // The jar signs the user in to sites: only its owner reads it.
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(directory)?;
    // tempfile creates the file readable by its owner only.
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(file).map_err(|error| error.error)?;
    Ok(())
}

/// Rewritten scripts and stylesheets by source and environment, with the
/// labels they map to.
#[derive(Default)]
struct RewriteCache {
    entries: HashMap<String, (u64, Arc<(String, BTreeMap<String, Environment>)>)>,
    clock: u64,
    characters: usize,
}

impl RewriteCache {
    fn get(&mut self, key: &str) -> Option<Arc<(String, BTreeMap<String, Environment>)>> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(key).map(|(used, value)| {
            *used = clock;
            value.clone()
        })
    }

    fn insert(&mut self, key: String, value: Arc<(String, BTreeMap<String, Environment>)>) {
        self.clock += 1;
        self.characters += value.0.len();
        if let Some((_, replaced)) = self.entries.insert(key, (self.clock, value)) {
            self.characters -= replaced.0.len();
        }
        while self.characters > REWRITE_CACHE_CHARACTERS {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some((_, value)) = self.entries.remove(&oldest) {
                self.characters -= value.0.len();
            }
        }
    }
}

/// The preview engine of the Host (`crates-and-packages.md`
/// § command-package-browser-preview).
pub struct Engine {
    pub(crate) upstream: Upstream,
    jar: Arc<Persisted>,
    /// The network of each origin (and site) the engine loaded a document
    /// from, as it connected to it: a page may reach no more private network
    /// (`preview.md` § Local network).
    spaces: Mutex<HashMap<String, Space>>,
    rewrites: Mutex<RewriteCache>,
    _saving: AbortOnDropHandle<()>,
}

impl Engine {
    /// The engine, with the cookie jar `file` keeps; a missing file is an
    /// empty jar. Must run inside a Tokio runtime, which writes the jar.
    pub fn open(file: PathBuf) -> Result<Arc<Self>, EngineError> {
        Self::with_upstream(file, Upstream::new().map_err(EngineError::Client)?)
    }

    /// The engine of the tests, whose upstream requests reach `network`.
    #[cfg(feature = "testing")]
    pub fn open_for_tests(file: PathBuf, network: crate::testing::Network) -> Result<Arc<Self>, EngineError> {
        Self::with_upstream(file, Upstream::for_tests(network).map_err(EngineError::Client)?)
    }

    fn with_upstream(file: PathBuf, upstream: Upstream) -> Result<Arc<Self>, EngineError> {
        let jar = match std::fs::read(&file) {
            Ok(bytes) => Jar::load(&bytes).map_err(|error| EngineError::CorruptJar(file.clone(), error))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Jar::default(),
            Err(error) => return Err(EngineError::Jar(file, error)),
        };
        let jar = Arc::new(Persisted {
            jar: Mutex::new(jar),
            file,
            dirty: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
            writing: tokio::sync::Mutex::new(()),
        });
        let saving = tokio::spawn({
            let jar = jar.clone();
            async move {
                loop {
                    jar.changed.notified().await;
                    tokio::time::sleep(SAVE_DELAY).await;
                    if let Err(error) = jar.save().await {
                        tracing::warn!(file = %jar.file.display(), %error, "the preview cookie jar was not written");
                    }
                }
            }
        });
        Ok(Arc::new(Engine {
            upstream,
            jar,
            spaces: Mutex::default(),
            rewrites: Mutex::default(),
            _saving: AbortOnDropHandle::new(saving),
        }))
    }

    /// Writes the jar's latest changes now, as the engine's program ends.
    pub async fn close(&self) -> std::io::Result<()> {
        self.jar.save().await
    }

    /// The jar's cookies of the sites of `addresses`, with their attributes,
    /// for the conversation browser (`preview.md` § Page state).
    pub fn cookies(&self, addresses: &[Url]) -> Vec<JarCookie> {
        let hosts: Vec<&str> = addresses.iter().filter_map(Url::host_str).collect();
        self.jar.read(|jar| jar.cookies_of(&hosts))
    }

    /// Takes cookies as `Set-Cookie` values, each as the site at its address
    /// would set it, `HttpOnly` ones included (`preview.md` § Page state).
    /// Answers how many the jar kept: it refuses what a browser refuses.
    pub fn take_cookies<'a>(&self, cookies: impl IntoIterator<Item = (&'a Url, &'a str)>) -> usize {
        self.jar.change(|jar| {
            let kept = cookies
                .into_iter()
                .filter(|(url, value)| jar.set(value, url, Api::Http, cookies::Context::Strict))
                .count();
            (kept, kept > 0)
        })
    }

    pub(crate) fn cookie_header(&self, url: &Url, context: cookies::Context) -> String {
        self.jar.read(|jar| jar.header(url, Api::Http, context))
    }

    pub(crate) fn store_set_cookies(&self, headers: &HeaderMap, target: &Url, context: cookies::Context) -> bool {
        self.jar.change(|jar| {
            let mut changed = false;
            for value in headers.get_all(header::SET_COOKIE) {
                if let Ok(value) = value.to_str() {
                    changed |= jar.set(value, target, Api::Http, context);
                }
            }
            if let Some(types) = header_text(headers, "clear-site-data") {
                let types: Vec<&str> = types.split(',').map(|value| value.trim().trim_matches('"')).collect();
                if types.contains(&"*") || types.contains(&"cookies") {
                    jar.clear_for(target);
                    changed = true;
                }
            }
            (changed, changed)
        })
    }

    /// The network of a page's origin (or a tab's top site): as the engine
    /// connected to it when it loaded the page, not as its name resolves
    /// now, which the site controls (DNS rebinding). `localhost` and an
    /// address written as an IP are what they say. A page the engine has no
    /// record of (it restarted while the page stayed open) is taken as its
    /// name resolves now.
    pub(crate) async fn space_of_origin(&self, origin: &str) -> Space {
        let Ok(url) = Url::parse(origin) else {
            return Space::Public;
        };
        let recorded = Space::named_by(&url).or_else(|| self.spaces.lock().expect("spaces lock").get(origin).copied());
        match recorded {
            Some(space) => space,
            None => self.upstream.space_named_now(&url).await,
        }
    }

    /// Whether the logical request is CORS-simple; otherwise the preflight's
    /// answer.
    async fn preflight_allows(
        &self,
        request: &PreviewRequest,
        method: &Method,
        target: &Url,
        fields: &Fields,
        credentials: bool,
        client: &PreviewClient,
        site: &str,
        referrer: Option<&str>,
        limit: Space,
    ) -> bool {
        let safelisted_type = |value: &str| {
            matches!(
                value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase().as_str(),
                "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
            )
        };
        let mut author: Vec<String> = request
            .headers
            .iter()
            .map(|header| (header.name.to_ascii_lowercase(), header.value.as_str()))
            .filter(|(name, value)| {
                (headers::is_page_header(name) || name == "content-type")
                    && !SAFELISTED_REQUEST_HEADERS.contains(&name.as_str())
                    && !(name == "content-type" && safelisted_type(value))
            })
            .map(|(name, _)| name)
            .collect();
        author.sort();
        author.dedup();
        if matches!(*method, Method::GET | Method::HEAD | Method::POST) && author.is_empty() {
            return true;
        }
        let preflight = preflight_headers(client, method.as_str(), &author, &fields.origin, site, referrer);
        let Ok(response) = self.upstream.send(Method::OPTIONS, target, preflight, None, limit).await else {
            return false;
        };
        let answer = response.headers();
        if !response.status().is_success()
            || !cors_allows(
                header_text(answer, "access-control-allow-origin"),
                header_text(answer, "access-control-allow-credentials"),
                &fields.origin,
                credentials,
            )
        {
            return false;
        }
        let list = |name: &str| -> Vec<String> {
            header_text(answer, name)
                .unwrap_or_default()
                .split(',')
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect()
        };
        let methods = list("access-control-allow-methods");
        let method_name = method.as_str().to_ascii_lowercase();
        if !matches!(*method, Method::GET | Method::HEAD | Method::POST)
            && !methods.contains(&method_name)
            && !(methods.iter().any(|value| value == "*") && !credentials)
        {
            return false;
        }
        let allowed = list("access-control-allow-headers");
        author.iter().all(|name| {
            allowed.contains(name) || (allowed.iter().any(|value| value == "*") && !credentials && name != "authorization")
        })
    }

    /// One request of a preview document.
    pub(crate) async fn fetch(&self, place: &Place, fetch: Fetch) -> Result<Answer, Failure> {
        let Fetch {
            environment: receiver,
            request,
            client,
            body,
        } = fetch;
        let Ok(mut target) = Url::parse(&request.url) else {
            return Err(Failure(format!("not an address: {}", request.url)));
        };
        if origin_of(&target) == receiver.origin {
            match target.path() {
                COOKIE_PATH => return self.document_cookie(&receiver, &request, &target, &body),
                SOURCE_MAP_PATH => return Ok(self.source_map(place, &receiver, &target, &client).await),
                _ => {}
            }
        }
        let integrity = address::take_parameter(&mut target, "integrity");
        // A worker's own script, `classic` or `module`, which starts the
        // worker's runtime: the runtime marks it, since its request looks the
        // same for both.
        let worker = address::take_parameter(&mut target, "worker");
        let tainted = address::take_parameter(&mut target, "tainted").is_some();
        // From an opaque-origin realm (a data: worker), whose origin
        // serializes as "null".
        let opaque = address::take_parameter(&mut target, "opaque").is_some();
        let Ok(method) = Method::from_bytes(request.method.as_bytes()) else {
            return Err(Failure(format!("not a method: {}", request.method)));
        };
        let mode = request.mode.to_string();
        let navigation = request.mode == PreviewMode::Navigate;
        let top_level = navigation && is_top_level(&receiver);
        // After a redirect across origins, a CORS request's origin is "null"
        // (Fetch § HTTP-redirect fetch).
        let initiator = match (&request.initiator, request.user) {
            _ if tainted || opaque => "null".to_owned(),
            (_, true) => String::new(),
            (Some(initiator), false) => initiator.origin.clone(),
            (None, false) => "null".to_owned(),
        };
        let fields = Fields {
            top: receiver.top.clone(),
            origin: initiator,
            nested: !top_level,
        };
        let context = same_site_context(&fields, &target, navigation, &method);
        let target_origin = origin_of(&target);
        let site = fetch_site(&fields.origin, &target_origin);
        let credentials = match request.mode {
            PreviewMode::Navigate => true,
            // Its response reaches the page readable (`preview.md` § The
            // forwarder and the relay): across sites it would hand the page
            // another site's answer to the user.
            PreviewMode::NoCors => site != "cross-site",
            _ => match request.credentials {
                PreviewCredentials::Include => true,
                PreviewCredentials::Omit => false,
                PreviewCredentials::SameOrigin => fields.origin == target_origin,
            },
        };
        let cross_origin_cors = request.mode == PreviewMode::Cors && !fields.origin.is_empty() && target_origin != fields.origin;
        let cross_origin_no_cors = request.mode == PreviewMode::NoCors && target_origin != receiver.origin;
        // The user's own navigation may go anywhere; a page's request no
        // further than its network. A navigation whose page the browser does
        // not name (one from an about:blank frame, or under no-referrer) goes
        // no further than its preview tab's top site.
        let limit = match (&request.initiator, request.user) {
            (_, true) => Space::Loopback,
            (Some(initiator), false) => self.space_of_origin(&initiator.origin).await,
            (None, false) => self.space_of_origin(&receiver.top).await,
        };
        let cookie = credentials
            .then(|| self.cookie_header(&target, context))
            .filter(|cookie| !cookie.is_empty());
        let has_body = !matches!(method, Method::GET | Method::HEAD);
        // Chrome sends Origin with every CORS request but a same-origin fetch,
        // and with every request that has a body.
        let sends_origin = !fields.origin.is_empty()
            && (has_body || (request.mode == PreviewMode::Cors && (target_origin != fields.origin || !request.destination.is_empty())));
        let referrer = if request.referrer.is_empty() {
            None
        } else {
            referrer_under(&request.referrer, &request.referrer_policy, &target)
        };
        let logical = headers::Request {
            method: method.as_str(),
            mode: &mode,
            destination: &request.destination,
            top_level,
            user: request.user,
            site,
            origin: sends_origin.then_some(fields.origin.as_str()),
            referrer: referrer.as_deref(),
            cookie: cookie.as_deref(),
            page: &request.headers,
            content_length: has_body.then_some(body.len()),
            keepalive: request.keepalive,
        };
        let request_headers = chrome_headers(&client, &logical);
        if cross_origin_cors
            && !self
                .preflight_allows(&request, &method, &target, &fields, credentials, &client, site, referrer.as_deref(), limit)
                .await
        {
            return Err(Failure::refused("cors-preflight", &target));
        }
        let upload = has_body.then(|| wreq::Body::from(body));
        let upstream = match self.upstream.send(method.clone(), &target, request_headers, upload, limit).await {
            Ok(upstream) => upstream,
            Err(UpstreamError::LocalNetwork) => return Err(Failure::refused("local-network", &target)),
            Err(error) => return Err(Failure(error.to_string())),
        };
        let status = upstream.status().as_u16();
        let upstream_headers = upstream.headers().clone();
        if navigation {
            // By origin, for the page's own requests, and by site, for a
            // tab's top.
            let space = self.upstream.space_of(&target, upstream.remote_addr());
            let mut spaces = self.spaces.lock().expect("spaces lock");
            spaces.insert(site_of(&target_origin), space);
            spaces.insert(target_origin.clone(), space);
        }
        if cross_origin_cors
            && !cors_allows(
                header_text(&upstream_headers, "access-control-allow-origin"),
                header_text(&upstream_headers, "access-control-allow-credentials"),
                &fields.origin,
                credentials,
            )
        {
            return Err(Failure::refused("cors", &target));
        }
        if request.mode == PreviewMode::NoCors
            && !corp_allows(header_text(&upstream_headers, "cross-origin-resource-policy"), &fields, &target_origin)
        {
            return Err(Failure::refused("corp", &target));
        }
        let cookie_changed = self.store_set_cookies(&upstream_headers, &target, context);
        // The modules an opaque-origin realm loads map their requests as its
        // own.
        let mut mapping = place.context(receiver.clone());
        mapping.opaque = opaque;
        let mut output = response_headers(&upstream_headers, &target, navigation, &mapping);
        if let Some(location) = output.iter_mut().find(|(name, _)| name == "location")
            && request.mode == PreviewMode::Cors
            && (tainted
                || address::logical_url(&location.1, &mapping)
                    .and_then(|logical| Url::parse(&logical).ok())
                    .is_some_and(|logical| origin_of(&logical) != target_origin))
        {
            location.1 = address::with_parameter(&location.1, "tainted", "1");
        }
        if cross_origin_cors {
            // The browser checks CORS between preview origins: the
            // initiator's is the receiving document's own, or "null" after a
            // redirect it followed. An opaque-origin realm runs on its
            // creator's preview origin, which the browser sends.
            let allowed = if fields.origin == "null" && !opaque {
                "null".to_owned()
            } else {
                mapping.preview_origin(&receiver)
            };
            output.push(("access-control-allow-origin".into(), allowed));
            if request.credentials == PreviewCredentials::Include {
                output.push(("access-control-allow-credentials".into(), "true".into()));
            }
        }
        if cross_origin_cors || cross_origin_no_cors {
            // The browser keeps every header for its own use and shows page
            // code the exposed ones: the upstream's list for a logically
            // cross-origin response. A no-cors one shows what a CORS request
            // without credentials would.
            let with_credentials = cross_origin_cors && credentials;
            let mut exposed: Vec<String> = header_text(&upstream_headers, "access-control-expose-headers")
                .unwrap_or_default()
                .split(',')
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty() && !(with_credentials && name == "*"))
                .collect();
            exposed.push("x-demi-cookie-changed".into());
            output.push(("access-control-expose-headers".into(), exposed.join(", ")));
        }
        if cookie_changed {
            output.push(("x-demi-cookie-changed".into(), "1".into()));
        }
        if navigation {
            // A browser applies CSP from a document's response only.
            output.push(("content-security-policy".into(), place.content_policy()));
        }
        output.push(("cross-origin-resource-policy".into(), "cross-origin".into()));
        let content_type = output
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        let kind = rewrite_kind(&request.destination, &content_type, navigation);
        if integrity.is_none() && (kind.is_none() || method == Method::HEAD || status == 204 || status == 304) {
            return Ok(Answer {
                status,
                headers: output,
                labels: mapping.labels(),
                body: AnswerBody::Upstream(upstream),
            });
        }
        let bytes = upstream.bytes().await.map_err(|error| Failure(error.to_string()))?;
        if let Some(integrity) = integrity.as_deref()
            && !integrity_matches(&bytes, integrity)
        {
            return Err(Failure::refused("integrity", &target));
        }
        let Some(kind) = kind else {
            return Ok(Answer {
                status,
                headers: output,
                labels: mapping.labels(),
                body: AnswerBody::Full(bytes),
            });
        };
        let (source, charset_free_type) = decode(&bytes, &content_type);
        output.retain(|(name, _)| name != "content-type" && name != "etag");
        output.push(("content-type".into(), charset_free_type));
        let body = if kind == Kind::Html {
            let mut boot = boot_data(&mapping, target.as_str(), target.as_str(), referrer.as_deref().unwrap_or_default());
            boot["cookie"] = json!(self.jar.read(|jar| jar.header(&target, Api::Script, script_context(&receiver, &target))));
            boot["ancestors"] = json!(if top_level {
                Vec::new()
            } else {
                request.initiator.iter().map(|initiator| initiator.origin.clone()).collect::<Vec<_>>()
            });
            // What the runtime shows page scripts of the device
            // (`preview.md` § Mobile).
            boot["device"] = json!({
                "userAgent": client.user_agent,
                "mobile": client.mobile,
                "platform": client.platform,
            });
            rewrite_html(&source, target.as_str(), HtmlOptions { fragment: false, boot: Some(&boot) }, &mapping)
        } else {
            let worker = worker.as_deref().map(|kind| WorkerScript {
                module: kind == "module",
                referrer: referrer.as_deref().unwrap_or_default(),
            });
            self.rewrite_subresource(kind, &source, &target, &mapping, worker)
        };
        Ok(Answer {
            status,
            headers: output,
            labels: mapping.labels(),
            body: AnswerBody::Full(Bytes::from(body)),
        })
    }

    /// A script or stylesheet, rewritten once per source and environment.
    fn rewrite_subresource(&self, kind: Kind, source: &str, target: &Url, mapping: &Context, worker: Option<WorkerScript>) -> String {
        let digest = STANDARD.encode(Sha256::digest(source.as_bytes()));
        let document = &mapping.document;
        let started_by = worker.map(|worker| format!("{} {}", worker.module, worker.referrer));
        let key = format!(
            "{kind:?}\n{target}\n{}\n{}\n{}\n{}\n{started_by:?}\n{digest}",
            document.origin, document.top, document.cross, mapping.opaque
        );
        let cached = self.rewrites.lock().expect("rewrite cache lock").get(&key);
        let entry = match cached {
            Some(entry) => entry,
            None => {
                let text = match kind {
                    // A stylesheet's selectors match the document that loads
                    // it.
                    Kind::Css => rewrite_stylesheet(source, target.as_str(), &format!("{}/", document.origin), false, mapping),
                    _ => {
                        // The map is served on the document's own preview
                        // origin, whose channel names its environment.
                        let mut map = format!(
                            "{}{SOURCE_MAP_PATH}?url={}",
                            mapping.preview_origin(document),
                            url::form_urlencoded::byte_serialize(target.as_str().as_bytes()).collect::<String>()
                        );
                        if mapping.opaque {
                            map.push_str("&opaque");
                        }
                        // The map knows the worker's first line, which its
                        // script does not have.
                        match worker {
                            Some(worker) if worker.module => map.push_str("&worker=module"),
                            Some(_) => map.push_str("&worker=classic"),
                            None => {}
                        }
                        let options = ScriptOptions {
                            filename: target.as_str(),
                            base: None,
                            module_url: None,
                            source_map: SourceMapMode::External(&map),
                            handler: false,
                            worker,
                        };
                        rewrite_javascript(source, options, mapping).unwrap_or_else(|error| {
                            // The browser reports the script's own syntax
                            // error.
                            tracing::debug!(url = target.as_str(), error = error.0, "a script was not rewritten");
                            source.to_owned()
                        })
                    }
                };
                let entry = Arc::new((text, mapping.labels()));
                self.rewrites.lock().expect("rewrite cache lock").insert(key, entry.clone());
                entry
            }
        };
        mapping.learn(entry.1.clone());
        entry.0.clone()
    }

    /// `document.cookie` of a preview document (`preview.md` § Cookies): the
    /// `url` parameter names the document's address; a POST's body is one
    /// cookie to set. Answers the cookies the document's script sees.
    fn document_cookie(&self, receiver: &Environment, request: &PreviewRequest, target: &Url, body: &Bytes) -> Result<Answer, Failure> {
        let Some(address) = target
            .query_pairs()
            .find(|(name, _)| name == "url")
            .and_then(|(_, value)| Url::parse(&value).ok())
            .filter(|address| origin_of(address) == receiver.origin)
        else {
            return Err(Failure("a cookie request names no address of its document".into()));
        };
        let context = script_context(receiver, &address);
        if request.method.eq_ignore_ascii_case("POST") {
            let value = String::from_utf8_lossy(body);
            self.jar.change(|jar| ((), jar.set(&value, &address, Api::Script, context)));
        }
        let value = self.jar.read(|jar| jar.header(&address, Api::Script, context));
        Ok(Answer {
            status: 200,
            headers: vec![
                ("content-type".into(), "text/plain; charset=utf-8".into()),
                ("cache-control".into(), "no-store".into()),
            ],
            labels: BTreeMap::new(),
            body: AnswerBody::Full(Bytes::from(value)),
        })
    }

    /// A debugger's map for a rewritten script (`preview.md` § Rewriting).
    async fn source_map(&self, place: &Place, receiver: &Environment, target: &Url, client: &PreviewClient) -> Answer {
        let query: HashMap<String, String> = target.query_pairs().into_owned().collect();
        let mut context = place.context(receiver.clone());
        context.opaque = query.contains_key("opaque");
        let map = match query.get("url").and_then(|address| Url::parse(address).ok()) {
            Some(script) => {
                // A worker's script starts with one more line; what it holds
                // does not move the map.
                let worker = query.get("worker").map(|kind| WorkerScript {
                    module: kind == "module",
                    referrer: "",
                });
                crate::sourcemap::compose(self, client, &script, &context, worker).await
            }
            None => Err("no script address".into()),
        };
        let (status, content_type, body) = match map {
            Ok(map) => (200, "application/json", map),
            Err(error) => (404, "text/plain; charset=utf-8", error.to_string()),
        };
        Answer {
            status,
            headers: vec![("content-type".into(), content_type.into())],
            labels: BTreeMap::new(),
            body: AnswerBody::Full(Bytes::from(body)),
        }
    }

    /// A script or its map as the document's own script request would fetch
    /// it, for a debugger: within the document's network, and with cookies
    /// only within its site.
    pub(crate) async fn fetch_text(&self, client: &PreviewClient, receiver: &Environment, target: &Url) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let fields = Fields {
            top: receiver.top.clone(),
            origin: receiver.origin.clone(),
            nested: true,
        };
        let site = fetch_site(&receiver.origin, &origin_of(target));
        let cookie = (site != "cross-site")
            .then(|| self.cookie_header(target, same_site_context(&fields, target, false, &Method::GET)))
            .filter(|cookie| !cookie.is_empty());
        let logical = headers::Request {
            method: "GET",
            mode: "no-cors",
            destination: "script",
            top_level: false,
            user: false,
            site,
            origin: None,
            referrer: None,
            cookie: cookie.as_deref(),
            page: &[],
            content_length: None,
            keepalive: false,
        };
        let limit = self.space_of_origin(&receiver.origin).await;
        let response = self
            .upstream
            .send(Method::GET, target, chrome_headers(client, &logical), None, limit)
            .await?;
        if !response.status().is_success() {
            return Err(format!("HTTP {} for {target}", response.status()).into());
        }
        Ok(response.text().await?)
    }
}

/// The upstream response's own headers, mapped; those that would act on the
/// preview origin are dropped. A redirect continues the request (a
/// navigation's goes through the target's boot page); Link and Refresh act
/// for the document the response becomes.
fn response_headers(upstream: &HeaderMap, target: &Url, navigation: bool, mapping: &Context) -> Vec<(String, String)> {
    let hsts = header_text(upstream, "strict-transport-security");
    let redirect_role = if navigation { Role::Navigation } else { Role::Resource };
    let mut output = Vec::new();
    for (name, value) in upstream {
        let name = name.as_str();
        if DROPPED_RESPONSE_HEADERS.contains(&name)
            || DROPPED_CORS_RESPONSE_HEADERS.contains(&name)
            || REMOVED_CONTENT_POLICIES.contains(&name)
        {
            continue;
        }
        // A value that is not text is no header a page reads.
        let Ok(value) = value.to_str() else { continue };
        let value = match name {
            "location" => address::map_url(&apply_redirect_hsts(value, target, hsts), Some(target.as_str()), redirect_role, mapping)
                .unwrap_or_else(|| value.to_owned()),
            "content-location" => {
                address::map_url(value, Some(target.as_str()), Role::Resource, mapping).unwrap_or_else(|| value.to_owned())
            }
            "refresh" => rewrite_refresh(value, target.as_str(), mapping),
            "link" => map_link(value, target, mapping),
            "referrer-policy" => adapted_referrer_policy(value),
            "permissions-policy" | "feature-policy" => {
                let kept = without_sync_xhr(name, value);
                if kept.is_empty() {
                    continue;
                }
                kept
            }
            _ => value.to_owned(),
        };
        output.push((name.to_owned(), value));
    }
    let find = |name: &str| output.iter().find(|(existing, _)| existing == name).map(|(_, value)| value.as_str());
    if let Some(isolation) = document_isolation_for(find("cross-origin-embedder-policy"), find("document-isolation-policy")) {
        output.push(("document-isolation-policy".into(), isolation.into()));
    }
    output
}

/// Decodes a text body by its declared charset (a BOM wins) and gives the
/// content type that now says UTF-8.
fn decode(bytes: &[u8], content_type: &str) -> (String, String) {
    let parsed: Option<mime::Mime> = content_type.parse().ok();
    let label = parsed
        .as_ref()
        .and_then(|mime| mime.get_param(mime::CHARSET))
        .map(|charset| charset.as_str().to_owned())
        .unwrap_or_else(|| "utf-8".into());
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(bytes);
    let rebuilt = match parsed {
        Some(mime) => {
            let mut value = format!("{}/{}", mime.type_(), mime.subtype());
            if let Some(suffix) = mime.suffix() {
                value.push('+');
                value.push_str(suffix.as_str());
            }
            for (name, parameter) in mime.params().filter(|(name, _)| *name != mime::CHARSET) {
                value.push_str(&format!("; {name}={parameter}"));
            }
            format!("{value}; charset=utf-8")
        }
        None => "text/html; charset=utf-8".into(),
    };
    (text.into_owned(), rebuilt)
}

fn map_link(value: &str, target: &Url, context: &Context) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        output.push_str(&rest[..=start]);
        let inner = &rest[start + 1..start + end];
        output.push_str(&address::map_url(inner, Some(target.as_str()), Role::Resource, context).unwrap_or_else(|| inner.to_owned()));
        output.push('>');
        rest = &rest[start + end + 1..];
    }
    output.push_str(rest);
    output
}

/// Subresource Integrity over the upstream bytes: any digest of the
/// strongest algorithm named must match (SRI § 3.3.5).
fn integrity_matches(bytes: &[u8], metadata: &str) -> bool {
    let mut entries: Vec<(u8, &str)> = Vec::new();
    for item in metadata.split_ascii_whitespace() {
        let token = item.split('?').next().unwrap_or_default();
        let Some((algorithm, digest)) = token.split_once('-') else { continue };
        let strength = match algorithm.to_ascii_lowercase().as_str() {
            "sha256" => 1,
            "sha384" => 2,
            "sha512" => 3,
            _ => continue,
        };
        entries.push((strength, digest));
    }
    let Some(strongest) = entries.iter().map(|(strength, _)| *strength).max() else {
        return true;
    };
    entries
        .iter()
        .filter(|(strength, _)| *strength == strongest)
        .any(|(strength, digest)| {
            let actual = match strength {
                1 => STANDARD.encode(Sha256::digest(bytes)),
                2 => STANDARD.encode(Sha384::digest(bytes)),
                _ => STANDARD.encode(Sha512::digest(bytes)),
            };
            actual == *digest
                || actual.trim_end_matches('=') == digest.replace('-', "+").replace('_', "/").trim_end_matches('=')
        })
}
