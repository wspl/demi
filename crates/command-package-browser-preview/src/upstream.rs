//! Requests to the sites a preview visits (`docs/browser/preview.md`
//! § Upstream requests). The TLS and HTTP/2 fingerprint follows the browser
//! the user's User-Agent names, so a site sees the browser the page's headers
//! describe, and a page's requests reach no network more private than the
//! page's (§ Local network).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Mutex;

use http::{HeaderMap, Method};
use url::Url;
use wreq::dns::{Addrs, GaiResolver, Name, Resolve, Resolving};
use wreq::header::OrigHeaderMap;
use wreq::redirect::Policy;
use wreq::tls::trust::CertStore;
use wreq_util::{Emulation, Platform, Profile};

/// The upstream client of every preview on the Host.
pub(crate) struct Upstream {
    /// Clients by fingerprint (profile, platform, whether Chrome's newer
    /// handshake applies) and by the most private network their requests
    /// may reach.
    clients: Mutex<HashMap<(String, String, bool, Space), wreq::Client>>,
    certificates: CertStore,
    /// Names the tests' fixtures answer on, each standing for a network.
    #[cfg(feature = "testing")]
    hosts: std::sync::Arc<crate::testing::Network>,
}

/// The network an address belongs to, from the most public to the most
/// private, as Chrome's Local Network Access has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Space {
    Public,
    Local,
    Loopback,
}

impl Space {
    pub fn of(ip: IpAddr) -> Space {
        match ip {
            IpAddr::V4(ip) => v4_space(ip),
            IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
                Some(mapped) => v4_space(mapped),
                None => v6_space(ip),
            },
        }
    }

    /// The network a URL's host names by itself: an IP address, or
    /// `localhost`.
    pub fn named_by(url: &Url) -> Option<Space> {
        match url.host()? {
            url::Host::Ipv4(ip) => Some(Space::of(ip.into())),
            url::Host::Ipv6(ip) => Some(Space::of(ip.into())),
            url::Host::Domain(host) => {
                (host == "localhost" || host.ends_with(".localhost")).then_some(Space::Loopback)
            }
        }
    }
}

fn v4_space(ip: Ipv4Addr) -> Space {
    let [a, b, ..] = ip.octets();
    if a == 127 || (a == 198 && (b & 0xfe) == 18) || ip.is_unspecified() {
        Space::Loopback
    } else if a == 10
        || a == 0
        || (a == 172 && (b & 0xf0) == 16)
        || (a == 192 && b == 168)
        || (a == 100 && (b & 0xc0) == 64)
        || (a == 169 && b == 254)
    {
        Space::Local
    } else {
        Space::Public
    }
}

fn v6_space(ip: Ipv6Addr) -> Space {
    let [first, second, ..] = ip.segments();
    if ip.is_loopback() || ip.is_unspecified() {
        Space::Loopback
    } else if (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        || (first == 0x2001 && second == 0x0db8)
        || (first == 0x3fff && (second & 0xf000) == 0)
    {
        Space::Local
    } else {
        Space::Public
    }
}

/// A request refused because it would reach a more private network than
/// the page that made it.
#[derive(Debug, thiserror::Error)]
#[error("refused: the address is in a more private network than the page")]
struct LocalNetworkRefused;

/// The system's resolution, keeping only the addresses no more private than
/// `limit`: the check is on the address the connection uses, so a name the
/// page's site points at a local address after loading it (DNS rebinding)
/// reaches nothing.
struct LimitedResolver {
    limit: Space,
    system: GaiResolver,
    #[cfg(feature = "testing")]
    hosts: std::sync::Arc<crate::testing::Network>,
}

impl Resolve for LimitedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let limit = self.limit;
        #[cfg(feature = "testing")]
        if let Some(space) = self.hosts.space(name.as_str()) {
            let result: Result<Addrs, Box<dyn std::error::Error + Send + Sync>> = if space <= limit {
                Ok(Box::new(std::iter::once(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))))
            } else {
                Err(Box::new(LocalNetworkRefused))
            };
            return Box::pin(std::future::ready(result));
        }
        let resolving = self.system.resolve(name);
        Box::pin(async move {
            let allowed: Vec<SocketAddr> = resolving
                .await?
                .filter(|address| Space::of(address.ip()) <= limit)
                .collect();
            if allowed.is_empty() {
                return Err(Box::new(LocalNetworkRefused) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

/// The browser family and major version a User-Agent names, as a profile
/// name prefix and number.
fn browser(user_agent: &str) -> (&'static str, u32) {
    let version = |marker: &str| {
        user_agent
            .split(marker)
            .nth(1)
            .and_then(|rest| rest.split(|character: char| !character.is_ascii_digit()).next())
            .and_then(|digits| digits.parse().ok())
    };
    if let Some(version) = version("Edg/") {
        return ("edge", version);
    }
    if let Some(version) = version("OPR/") {
        return ("opera", version);
    }
    if let Some(version) = version("Firefox/") {
        return ("firefox", version);
    }
    if let Some(version) = version("Chrome/") {
        return ("chrome", version);
    }
    if user_agent.contains("Safari/")
        && let Some(version) = version("Version/")
    {
        return ("safari", version);
    }
    ("chrome", 999)
}

fn platform(user_agent: &str) -> Platform {
    if user_agent.contains("Android") {
        Platform::Android
    } else if user_agent.contains("iPhone") || user_agent.contains("iPad") {
        Platform::IOS
    } else if user_agent.contains("Windows") {
        Platform::Windows
    } else if user_agent.contains("Linux") || user_agent.contains("X11") {
        Platform::Linux
    } else {
        Platform::MacOS
    }
}

/// The trust anchor IDs Chrome 154 requests in the `trust_anchors`
/// extension (draft-ietf-tls-trust-anchor-ids), in wire format, as
/// tls.peet.ws recorded on 2026-10-07. wreq's newest Chrome profile (149)
/// predates the extension; without it the JA4 of a Chrome 154 User-Agent is
/// wrong.
const CHROME_TRUST_ANCHORS: &[u8] = &[
    0x05, 0x82, 0xdf, 0x13, 0x02, 0x01, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x06, 0x05, 0x82, 0xdf, 0x13,
    0x02, 0x0d, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x0e, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x0f, 0x05, 0x82,
    0xdf, 0x13, 0x02, 0x12, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x13, 0x05, 0x82, 0xdf, 0x13, 0x02, 0x14,
    0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x07, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d,
    0x01, 0x08, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x09, 0x08, 0x83, 0x9a, 0x64, 0x8c,
    0x9b, 0x2d, 0x01, 0x0a, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x0b, 0x08, 0x83, 0x9a,
    0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x0c, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x0d, 0x08,
    0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01, 0x12, 0x08, 0x83, 0x9a, 0x64, 0x8c, 0x9b, 0x2d, 0x01,
    0x13, 0x04, 0xd6, 0x79, 0x09, 0x01, 0x04, 0xd6, 0x79, 0x09, 0x04, 0x04, 0xd6, 0x79, 0x09, 0x05,
    0x04, 0xd6, 0x79, 0x09, 0x06, 0x04, 0xd6, 0x79, 0x09, 0x07, 0x04, 0xd6, 0x79, 0x09, 0x08, 0x04,
    0xd6, 0x79, 0x09, 0x0a, 0x04, 0xd6, 0x79, 0x09, 0x0b, 0x04, 0xd6, 0x79, 0x09, 0x0c, 0x04, 0xd6,
    0x79, 0x09, 0x0d, 0x04, 0xd6, 0x79, 0x09, 0x0f,
];

/// The ML-DSA-44, -65 and -87 signature algorithms Chrome 154 lists first
/// (after a GREASE value), as tls.peet.ws recorded on 2026-10-07.
/// Certificates are not verified with them: no public CA issues ML-DSA
/// certificates, and this BoringSSL does not implement them.
const CHROME_ADVERTISED_SIGALGS: &[u16] = &[0x0904, 0x0905, 0x0906];

/// The first Chrome measured to differ from wreq's newest profile: it sends
/// the trust anchors extension and lists GREASE and ML-DSA signature
/// algorithms.
const CHROME_NEWER_SINCE: u32 = 154;

/// The Chromium major version a User-Agent names (Chrome, Edge and Opera all
/// carry it).
fn chromium_major(user_agent: &str) -> Option<u32> {
    let rest = user_agent.split("Chrome/").nth(1)?;
    rest.split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// The newest profile of the browser's family no newer than its version.
/// wreq-util names its profiles only through serde, so the candidates are
/// tried by name.
fn profile(user_agent: &str) -> Profile {
    let named = |name: String| serde_json::from_value::<Profile>(serde_json::Value::String(name)).ok();
    let (family, version) = browser(user_agent);
    if let Some(profile) = (100..=version.min(400))
        .rev()
        .find_map(|candidate| named(format!("{family}_{candidate}")))
    {
        return profile;
    }
    if family == "safari"
        && let Some(profile) = ["safari_26.4", "safari_18.5", "safari_17.6"]
            .into_iter()
            .find_map(|name| named(name.to_owned()))
    {
        return profile;
    }
    Profile::Chrome149
}

/// The headers' own order, which the request keeps in place of the
/// profile's default order: the engine passes on the browser's.
fn order(headers: &HeaderMap) -> OrigHeaderMap {
    let mut order = OrigHeaderMap::with_capacity(headers.keys_len());
    for name in headers.keys() {
        order.insert(name.clone());
    }
    order
}

impl Upstream {
    pub(crate) fn new() -> Result<Self, wreq::Error> {
        let certificates = CertStore::builder()
            .add_der_certs(
                webpki_root_certs::TLS_SERVER_ROOT_CERTS
                    .iter()
                    .map(|certificate| certificate.as_ref()),
            )
            .build()?;
        Ok(Upstream {
            clients: Mutex::default(),
            certificates,
            #[cfg(feature = "testing")]
            hosts: std::sync::Arc::default(),
        })
    }

    /// The upstream client of the tests: their fixtures' names and the
    /// authority their certificates come from.
    #[cfg(feature = "testing")]
    pub(crate) fn for_tests(network: crate::testing::Network) -> Result<Self, wreq::Error> {
        let certificates = CertStore::builder()
            .add_der_certs(
                webpki_root_certs::TLS_SERVER_ROOT_CERTS
                    .iter()
                    .map(|certificate| certificate.as_ref()),
            )
            .add_pem_cert(network.authority())
            .build()?;
        Ok(Upstream {
            clients: Mutex::default(),
            certificates,
            hosts: std::sync::Arc::new(network),
        })
    }

    /// The network of the address a response came from, which the page it
    /// becomes is judged by (§ Local network).
    pub(crate) fn space_of(&self, url: &Url, address: Option<SocketAddr>) -> Space {
        #[cfg(feature = "testing")]
        if let Some(space) = url.host_str().and_then(|host| self.hosts.space(host)) {
            return space;
        }
        match address {
            Some(address) => Space::of(address.ip()),
            // A response without an address on record came over no socket
            // the engine knows; its name's own network is the best guess.
            None => Space::named_by(url).unwrap_or(Space::Public),
        }
    }

    /// The network a page's address names now, for a page the engine has no
    /// connection on record for (it restarted while the page stayed open):
    /// the most public of the addresses its name resolves to, public when it
    /// resolves to none.
    pub(crate) async fn space_named_now(&self, url: &Url) -> Space {
        #[cfg(feature = "testing")]
        if let Some(space) = url.host_str().and_then(|host| self.hosts.space(host)) {
            return space;
        }
        if let Some(space) = Space::named_by(url) {
            return space;
        }
        let host = url.host_str().unwrap_or_default();
        let port = url.port_or_known_default().unwrap_or(443);
        match tokio::net::lookup_host((host, port)).await {
            Ok(addresses) => addresses
                .map(|address| Space::of(address.ip()))
                .min()
                .unwrap_or(Space::Public),
            // A name that does not resolve reaches nothing more private.
            Err(_) => Space::Public,
        }
    }

    /// One client per emulated browser and network limit: connections are
    /// pooled per fingerprint, and a connection made for one limit is never
    /// reused under a narrower one.
    fn client(&self, user_agent: &str, limit: Space) -> Result<wreq::Client, wreq::Error> {
        let profile = profile(user_agent);
        let platform = platform(user_agent);
        let newer_chrome = chromium_major(user_agent).is_some_and(|major| major >= CHROME_NEWER_SINCE);
        let key = (format!("{profile:?}"), format!("{platform:?}"), newer_chrome, limit);
        if let Some(client) = self.clients.lock().expect("clients lock").get(&key) {
            return Ok(client.clone());
        }
        let mut emulation = wreq::IntoEmulation::into_emulation(
            Emulation::builder().profile(profile).platform(platform).build(),
        );
        if newer_chrome && let Some(tls) = emulation.tls_options.as_mut() {
            tls.trust_anchors = Some(CHROME_TRUST_ANCHORS.into());
            tls.grease_sigalgs = true;
            tls.advertised_sigalgs = Some(CHROME_ADVERTISED_SIGALGS.into());
        }
        let resolver = LimitedResolver {
            limit,
            system: GaiResolver::new(),
            #[cfg(feature = "testing")]
            hosts: self.hosts.clone(),
        };
        let client = wreq::Client::builder()
            .emulation(emulation)
            .redirect(Policy::none())
            .tls_cert_store(self.certificates.clone())
            .dns_resolver(resolver)
            .no_proxy()
            .build()?;
        self.clients
            .lock()
            .expect("clients lock")
            .insert(key, client.clone());
        Ok(client)
    }

    /// A client whose connections reach no network more private than
    /// `limit`. A host that names its network (an IP address, `localhost`)
    /// is checked here; a name, when it is resolved.
    pub(crate) fn client_for(
        &self,
        user_agent: &str,
        url: &Url,
        limit: Space,
    ) -> Result<wreq::Client, UpstreamError> {
        if Space::named_by(url).is_some_and(|space| space > limit) {
            return Err(UpstreamError::LocalNetwork);
        }
        self.client(user_agent, limit).map_err(UpstreamError::Client)
    }

    /// Send one request with the headers in the order given. Like a browser,
    /// a GET, HEAD or OPTIONS without a body is sent once more on a new
    /// connection when its connection failed or closed before the response.
    /// `limit`: the most private network the request may reach, the network
    /// of the page that made it.
    pub(crate) async fn send(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        body: Option<wreq::Body>,
        limit: Space,
    ) -> Result<wreq::Response, UpstreamError> {
        check(url)?;
        let user_agent = headers
            .get(http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let client = self.client_for(&user_agent, url, limit)?;
        let retryable = body.is_none() && matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
        let mut request = client
            .request(method.clone(), url.as_str())
            .headers(headers.clone())
            .orig_headers(order(&headers));
        if let Some(body) = body {
            request = request.body(body);
        }
        match request.send().await {
            Err(error) if refused_by_local_network(&error) => Err(UpstreamError::LocalNetwork),
            // A connection that failed, or closed before any response: a
            // server ends an idle connection the client reuses at that
            // moment, and Chrome sends again.
            Err(error)
                if retryable
                    && (error.is_connect() || caused_by(&error, wreq_proto::Error::is_incomplete_message)) =>
            {
                tracing::debug!(url = url.as_str(), %error, "upstream request sent again");
                client
                    .request(method, url.as_str())
                    .orig_headers(order(&headers))
                    .headers(headers)
                    .send()
                    .await
                    .map_err(UpstreamError::Request)
            }
            result => result.map_err(UpstreamError::Request),
        }
    }
}

/// Only http(s) and ws(s) targets without credentials in the URL.
pub(crate) fn check(url: &Url) -> Result<(), UpstreamError> {
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") || !url.username().is_empty() || url.password().is_some() {
        return Err(UpstreamError::Target(url.to_string()));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum UpstreamError {
    #[error("not a previewed target: {0}")]
    Target(String),
    /// The address is in a more private network than the page that made
    /// the request.
    #[error("{LocalNetworkRefused}")]
    LocalNetwork,
    #[error("upstream client: {0}")]
    Client(wreq::Error),
    /// The causes say what failed: DNS, connect, TLS, or the peer.
    #[error("upstream request: {}", causes(.0))]
    Request(wreq::Error),
}

/// The error with each of its causes, which say what failed.
fn causes(error: &wreq::Error) -> String {
    let mut text = error.to_string();
    let mut cause = std::error::Error::source(error);
    while let Some(error) = cause {
        text.push_str(": ");
        text.push_str(&error.to_string());
        cause = error.source();
    }
    text
}

/// Whether the local network rule refused the error's connection.
pub(crate) fn refused_by_local_network(error: &wreq::Error) -> bool {
    caused_by::<LocalNetworkRefused>(error, |_| true)
}

/// Whether one of the error's causes is a `T` that passes `test`.
fn caused_by<T: std::error::Error + 'static>(error: &wreq::Error, test: impl Fn(&T) -> bool) -> bool {
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        if cause.downcast_ref::<T>().is_some_and(&test) {
            return true;
        }
        source = cause.source();
    }
    false
}
