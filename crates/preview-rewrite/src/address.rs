//! Preview addresses: `https://<namespace>--<label>.<preview domain>/<path>`, where the label
//! names a document environment (`docs/browser/preview.md` § Addresses and labels). Paths stay as the
//! site wrote them, so only absolute addresses are mapped.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

/// A document's environment, as Chrome keys storage: its logical origin, the logical
/// top-level site, and whether any ancestor frame is cross-site with it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Environment {
    pub origin: String,
    pub top: String,
    pub cross: bool,
}

/// Where addresses are mapped: the preview domain, the namespace and Host whose labels these
/// are, the files every preview origin serves, and the document whose addresses they are.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    /// The preview domain, with a port when it is not 443: `demi-preview.dev`.
    pub domain: String,
    pub namespace: String,
    pub host: String,
    /// The bootstrap's path on every preview origin: `/__demi/v1/boot.html`.
    pub boot: String,
    /// The client script's path on every preview origin, the first script of every document.
    pub client: String,
    /// The runtime's path on every preview origin.
    pub runtime: String,
    pub document: Environment,
    /// Whether the document is a preview tab's top frame: its own navigations are top-level.
    /// The engine estimates it from the environment; the runtime knows the frame tree and
    /// gives its own, so boot data does not carry the estimate.
    #[serde(skip_serializing)]
    pub top_level: bool,
    /// Whether the realm's origin is opaque: a data: worker, and the modules it loads. Every
    /// request address it maps carries the `opaque` engine parameter, and the engine sends the
    /// request with origin `null`. The runtime gives it for its own realm; the engine takes it
    /// from the request it rewrites a module for.
    #[serde(default, skip_serializing)]
    pub opaque: bool,
    /// Every label this context mapped, for reflection and for the preview relay.
    #[serde(skip)]
    pub labels: Mutex<BTreeMap<String, Environment>>,
}

/// How an address is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A subresource or a fetch: loaded by the requesting document's forwarder.
    Resource,
    /// A module: exactly one address per module, never a bootstrap.
    Module,
    /// A child frame's document.
    Child,
    /// A navigation of the document's own frame: links, forms, `location`, refresh.
    Navigation,
    /// A navigation of a preview tab's top frame or a new tab: `_top`, `_blank`, `window.open`.
    TopNavigation,
}

const LABEL_BYTES: usize = 10;

fn network_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https")
}

fn scheme_site(scheme: &str) -> &str {
    match scheme {
        "wss" => "https",
        "ws" => "http",
        other => other,
    }
}

/// The registrable domain of a host. Natively the public suffix list decides; the page's
/// runtime, which ships without the list, uses the last two labels. Only top-level navigations
/// to a new site use it in the page, and those are mapped again by the engine before they load.
#[cfg(feature = "public-suffix")]
fn registrable_domain(host: &str) -> String {
    // An address is its own site; the list would take its last two numbers
    // for a domain, and the engine's labels would differ from the runtime's.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_owned();
    }
    psl::domain_str(host).unwrap_or(host).to_owned()
}

#[cfg(not(feature = "public-suffix"))]
fn registrable_domain(host: &str) -> String {
    let numeric = host.parse::<std::net::IpAddr>().is_ok();
    let parts: Vec<&str> = host.split('.').collect();
    if numeric || parts.len() <= 2 { host.to_owned() } else { parts[parts.len() - 2..].join(".") }
}

/// The site used for SameSite and storage partitions: scheme and registrable domain. An
/// opaque origin is a site of its own.
pub fn site_of(origin: &str) -> String {
    if origin.is_empty() || origin == "null" {
        return origin.to_owned();
    }
    let Ok(url) = Url::parse(origin) else { return origin.to_owned() };
    let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']');
    format!("{}://{}", scheme_site(url.scheme()), registrable_domain(host))
}

/// Whether an origin belongs to a known site: the same scheme, and the site's domain or a
/// subdomain of it. Engine and runtime both decide this without the public suffix list, so
/// they map the same address to the same label.
pub fn same_site(origin: &str, site: &str) -> bool {
    let (Ok(origin), Ok(site)) = (Url::parse(origin), Url::parse(site)) else { return false };
    let host = origin.host_str().unwrap_or_default();
    let domain = site.host_str().unwrap_or_default();
    scheme_site(origin.scheme()) == scheme_site(site.scheme()) && (host == domain || host.ends_with(&format!(".{domain}")))
}

/// The label of an environment: the first 80 bits of a SHA-256 digest, in DNS-safe base32.
pub fn label(namespace: &str, host: &str, environment: &Environment) -> String {
    let mut digest = Sha256::new();
    digest.update(format!("{namespace}\n{host}\n{}\n{}\n{}", environment.origin, environment.top, u8::from(environment.cross)));
    data_encoding::BASE32_DNSSEC.encode(&digest.finalize()[..LABEL_BYTES])
}

/// `host[:port]`, as `URL.host` gives it: the port only when it is not the default.
fn authority(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

/// The URL's path, query and fragment as `URL.pathname + search + hash` give them.
pub fn path_query_fragment(url: &Url) -> String {
    let mut text = url.path().to_owned();
    if let Some(query) = url.query().filter(|query| !query.is_empty()) {
        text.push('?');
        text.push_str(query);
    }
    if let Some(fragment) = url.fragment().filter(|fragment| !fragment.is_empty()) {
        text.push('#');
        text.push_str(fragment);
    }
    text
}

fn parse(value: &str, base: Option<&str>) -> Option<Url> {
    match base {
        Some(base) => Url::parse(base).ok()?.join(value).ok(),
        None => Url::parse(value).ok(),
    }
}

fn origin_of(url: &Url) -> String {
    format!("{}://{}", url.scheme(), authority(url))
}

/// A preview address taken apart.
#[derive(Debug)]
pub struct PreviewAddress {
    pub label: String,
    /// What the address leads to: its path, query and fragment, or a bootstrap's target.
    pub target: String,
    pub bootstrap: bool,
}

impl Context {
    fn domain_host(&self) -> &str {
        self.domain.split(':').next().unwrap_or_default()
    }

    fn domain_port(&self) -> Option<u16> {
        self.domain.split_once(':').and_then(|(_, port)| port.parse().ok())
    }

    /// Every preview origin of this namespace starts with this.
    pub fn namespace_prefix(&self) -> String {
        format!("https://{}--", self.namespace)
    }

    /// The preview origin of an environment, remembered for reflection and the relay.
    pub fn preview_origin(&self, environment: &Environment) -> String {
        let label = label(&self.namespace, &self.host, environment);
        let origin = format!("https://{}--{label}.{}", self.namespace, self.domain);
        self.labels.lock().expect("labels lock").entry(label).or_insert_with(|| environment.clone());
        origin
    }

    /// The environment of the document an address with this origin and role leads to.
    pub fn environment_for(&self, origin: &str, role: Role) -> Environment {
        let document = &self.document;
        let top_level = role == Role::TopNavigation || (role == Role::Navigation && self.top_level);
        if origin == document.origin && (!top_level || self.top_level) {
            return document.clone();
        }
        if top_level {
            return Environment { origin: origin.to_owned(), top: site_of(origin), cross: false };
        }
        // A document without a cross-site ancestor is same-site with its top, so comparing
        // with the top covers its own site too.
        let cross = document.cross || !same_site(origin, &document.top);
        Environment { origin: origin.to_owned(), top: document.top.clone(), cross }
    }

    pub fn parse_preview_url(&self, value: &str) -> Option<PreviewAddress> {
        let url = Url::parse(value).ok()?;
        let host = url.host_str()?;
        let label = host.strip_suffix(&format!(".{}", self.domain_host()))?.strip_prefix(&format!("{}--", self.namespace))?;
        // An address a page built from the preview host alone, without the port, still names
        // the same origin wherever the preview domain is served on the default port.
        let port_matches = url.port() == self.domain_port() || url.port().is_none();
        if url.scheme() != "https" || !port_matches || label.contains('.') {
            return None;
        }
        let bootstrap = url.path() == self.boot;
        let target = if bootstrap { bootstrap_target(url.fragment().unwrap_or_default()) } else { path_query_fragment(&url) };
        Some(PreviewAddress { label: label.to_owned(), target, bootstrap })
    }

    pub fn is_preview_url(&self, value: &str) -> bool {
        self.parse_preview_url(value).is_some()
    }

    /// The environment a label stands for, if this context has met it.
    pub fn environment_of(&self, label: &str) -> Option<Environment> {
        self.labels.lock().expect("labels lock").get(label).cloned()
    }

    /// Labels met elsewhere (the document's boot data), for reflection.
    pub fn learn(&self, labels: BTreeMap<String, Environment>) {
        self.labels.lock().expect("labels lock").extend(labels);
    }

    pub fn labels(&self) -> BTreeMap<String, Environment> {
        self.labels.lock().expect("labels lock").clone()
    }
}

/// A bootstrap's target from its fragment: `to=<path>`, after an optional `token=<id>&`.
fn bootstrap_target(fragment: &str) -> String {
    let rest = fragment.strip_prefix("token=").map_or(fragment, |rest| rest.split_once('&').map_or("", |(_, rest)| rest));
    rest.strip_prefix("to=").unwrap_or("/").to_owned()
}

/// Map an absolute or base-relative logical address to its preview address. Other schemes,
/// and preview addresses, come back resolved and unchanged. A navigation to another label goes
/// through that label's bootstrap, since its forwarder may not be installed yet.
pub fn map_url(value: &str, base: Option<&str>, role: Role, context: &Context) -> Option<String> {
    let url = parse(value, base)?;
    if !network_scheme(url.scheme()) || context.is_preview_url(url.as_str()) {
        return Some(url.into());
    }
    let environment = context.environment_for(&origin_of(&url), role);
    let origin = context.preview_origin(&environment);
    let target = path_query_fragment(&url);
    let navigation = matches!(role, Role::Child | Role::Navigation | Role::TopNavigation);
    if navigation && environment != context.document {
        return Some(format!("{origin}{}#to={target}", context.boot));
    }
    let address = format!("{origin}{target}");
    Some(if context.opaque && !navigation { with_parameter(&address, "opaque", "1") } else { address })
}

/// The logical address behind a preview address; any other absolute URL unchanged. A label
/// this context has not met has no logical address.
pub fn logical_url(value: &str, context: &Context) -> Option<String> {
    let Some(address) = context.parse_preview_url(value) else { return Url::parse(value).ok().map(Into::into) };
    let environment = context.environment_of(&address.label)?;
    // The engine's parameters are the preview's, never part of the page's address.
    let mut url = Url::parse(&format!("{}{}", environment.origin, address.target)).ok()?;
    for name in ENGINE_PARAMETERS {
        take_parameter(&mut url, name);
    }
    Some(url.into())
}

/// Parameters the rewriter adds to an address for the engine.
pub const ENGINE_PARAMETERS: [&str; 4] = ["integrity", "tainted", "opaque", "worker"];

fn is_absolute(text: &str) -> bool {
    let mut characters = text.chars();
    characters.next().is_some_and(|first| first.is_ascii_alphabetic())
        && characters.take_while(|&character| character != ':').all(|character| character.is_ascii_alphanumeric() || "+.-".contains(character))
        && text.contains(':')
}

/// Map an address as written in markup or CSS. A relative address stays as written: the
/// browser resolves it against the preview address of its base, which keeps the path. A
/// navigation that a relative address takes to another origin (through a foreign `<base>`)
/// is mapped, so it goes through that origin's bootstrap.
pub fn map_written_url(text: &str, base: &str, role: Role, context: &Context) -> String {
    let trimmed = text.trim();
    let Some(resolved) = parse(trimmed, Some(base)) else { return text.to_owned() };
    if !network_scheme(resolved.scheme()) || context.is_preview_url(resolved.as_str()) {
        return text.to_owned();
    }
    let slashes = trimmed.starts_with("//") || trimmed.starts_with("\\\\") || trimmed.starts_with("/\\") || trimmed.starts_with("\\/");
    let navigation = matches!(role, Role::Child | Role::Navigation | Role::TopNavigation);
    if !is_absolute(trimmed) && !slashes && !(navigation && origin_of(&resolved) != context.document.origin) {
        return text.to_owned();
    }
    map_url(resolved.as_str(), None, role, context).unwrap_or_else(|| text.to_owned())
}

/// The logical address the page wrote: the reverse of [`map_written_url`].
pub fn written_url(text: &str, context: &Context) -> String {
    logical_url(text.trim(), context).filter(|_| context.is_preview_url(text.trim())).unwrap_or_else(|| text.to_owned())
}

/// Convert one written address: forward, or with `reflect` back to the page's text.
pub fn convert_written_url(text: &str, base: &str, reflect: bool, role: Role, context: &Context) -> String {
    if reflect { written_url(text, context) } else { map_written_url(text, base, role, context) }
}

/// How an element's address attribute is used. `target` is the element's target attribute.
pub fn address_role(tag: &str, attribute: &str, script_type: Option<&str>, rel: Option<&str>, target: Option<&str>) -> Role {
    let new_context = target.is_some_and(|target| matches!(target.trim().to_ascii_lowercase().as_str(), "_top" | "_blank"));
    match (tag, attribute) {
        ("iframe" | "frame" | "embed", "src") | ("object", "data") => Role::Child,
        ("script", "src") if script_type.is_some_and(|value| value.trim().eq_ignore_ascii_case("module")) => Role::Module,
        ("link", "href") if rel.is_some_and(|value| value.split_ascii_whitespace().any(|token| token.eq_ignore_ascii_case("modulepreload"))) => Role::Module,
        ("a" | "area", "href") | ("form", "action") | ("button" | "input", "formaction") => {
            if new_context { Role::TopNavigation } else { Role::Navigation }
        }
        _ => Role::Resource,
    }
}

/// A preview address carrying an engine parameter (`integrity`, `tainted`, `opaque`,
/// `worker`), which the engine removes before the request goes upstream.
pub fn with_parameter(address: &str, name: &str, value: &str) -> String {
    let Ok(mut url) = Url::parse(address) else { return address.to_owned() };
    if value.is_empty() {
        return address.to_owned();
    }
    let parameter = format!("__demi_{name}={}", percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC));
    let query = match url.query().filter(|query| !query.is_empty()) {
        Some(query) => format!("{query}&{parameter}"),
        None => parameter,
    };
    url.set_query(Some(&query));
    url.into()
}

/// Remove an engine parameter from a URL, keeping the rest of its query's exact encoding.
pub fn take_parameter(url: &mut Url, name: &str) -> Option<String> {
    let query = url.query()?.to_owned();
    let key = format!("__demi_{name}=");
    let mut kept = Vec::new();
    let mut value = None;
    for part in query.split('&') {
        match part.strip_prefix(&key) {
            Some(found) if value.is_none() => value = Some(percent_encoding::percent_decode_str(found).decode_utf8_lossy().into_owned()),
            _ => kept.push(part),
        }
    }
    value.as_ref()?;
    let kept = kept.join("&");
    url.set_query(if kept.is_empty() { None } else { Some(&kept) });
    value
}

/// Map a module specifier: an absolute one, or a relative one when its base is known. The
/// browser resolves bare names through the import map.
pub fn map_module_specifier(specifier: &str, base: Option<&str>, context: &Context) -> String {
    // A relative specifier is resolved here when the base is known: the browser would resolve
    // it against the script's response URL, which for a response Demi holds is not the
    // script's own. A bare specifier is the import map's.
    let relative = ["./", "../", "/"].iter().any(|prefix| specifier.starts_with(prefix)) && !specifier.starts_with("//");
    if Url::parse(specifier).is_err() && !specifier.starts_with("//") && !(relative && base.is_some()) {
        return specifier.to_owned();
    }
    map_url(specifier, base, Role::Module, context).unwrap_or_else(|| specifier.to_owned())
}
