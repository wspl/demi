//! The Host's cookie jar (`preview.md` § Cookies): the Host's identity toward
//! the sites its previews visit. Storing and matching (domain, path, expiry,
//! `Secure`) are `cookie_store`'s; what a browser adds on top (SameSite,
//! `HttpOnly` against scripts, name prefixes, public suffixes) is here, as
//! Chrome applies it (RFC 6265bis).

use std::collections::HashMap;

use cookie::{Cookie as RawCookie, SameSite};
use cookie_store::{CookieDomain, CookieExpiration, CookieStore};
use url::Url;

/// How a request or a script relates to the site it reaches (RFC 6265bis
/// § 5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Context {
    Strict,
    Lax,
    CrossSite,
}

/// Who reads or writes: an HTTP exchange, or a page's script
/// (`document.cookie`), which never sees or replaces an `HttpOnly` cookie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Api {
    Http,
    Script,
}

/// A cookie's identity in the store: domain, path and name.
type Key = (String, String, String);

#[derive(Default)]
pub(crate) struct Jar {
    store: CookieStore,
    /// When each stored cookie was created, as a sequence: a header lists
    /// cookies with equal paths oldest first (RFC 6265 § 5.4), and replacing
    /// a cookie keeps its creation time.
    created: HashMap<Key, u64>,
    sequence: u64,
}

/// A cookie of the jar with its attributes, as [Page state] moves it to the
/// conversation browser.
///
/// [Page state]: https://github.com/demicodes/demi/blob/main/docs/browser/preview.md#page-state
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JarCookie {
    pub name: String,
    pub value: String,
    /// The host for a host-only cookie, otherwise the `Domain` attribute's
    /// domain, without a leading dot.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// `None` when the site gave no `SameSite`, which counts as `Lax`.
    pub same_site: Option<CookieSameSite>,
    /// When it expires, in seconds since the Unix epoch; `None` for a
    /// session cookie.
    pub expires: Option<i64>,
}

/// A cookie's `SameSite` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieSameSite {
    Strict,
    Lax,
    None,
}

fn key_of(cookie: &cookie_store::Cookie) -> Key {
    (
        String::from(&cookie.domain),
        String::from(&cookie.path),
        cookie.name().to_owned(),
    )
}

/// A cookie without `SameSite` is `Lax`, as in Chrome.
fn same_site(cookie: &RawCookie) -> SameSite {
    cookie.same_site().unwrap_or(SameSite::Lax)
}

fn allowed(cookie: &RawCookie, context: Context) -> bool {
    match same_site(cookie) {
        SameSite::Strict => context == Context::Strict,
        SameSite::Lax => context != Context::CrossSite,
        SameSite::None => true,
    }
}

/// A secure context for cookies: HTTPS, or a loopback host as Chrome treats
/// it.
fn secure(url: &Url) -> bool {
    url.scheme() == "https"
        || url.scheme() == "wss"
        || matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        || url.host_str().is_some_and(|host| host.ends_with(".localhost"))
}

/// The registrable domain of a cookie's or an address's host.
fn registrable(host: &str) -> &str {
    let host = host.trim_start_matches('.');
    psl::domain_str(host).unwrap_or(host)
}

impl Jar {
    /// Stores one `Set-Cookie` or `document.cookie` value. Returns whether
    /// the jar changed.
    pub fn set(&mut self, value: &str, url: &Url, api: Api, context: Context) -> bool {
        let Ok(cookie) = RawCookie::parse(value.to_owned()) else {
            return false;
        };
        if api == Api::Script && cookie.http_only().unwrap_or(false) {
            return false;
        }
        if same_site(&cookie) != SameSite::None && context == Context::CrossSite {
            return false;
        }
        if cookie.same_site() == Some(SameSite::None) && !cookie.secure().unwrap_or(false) {
            return false;
        }
        if cookie.secure().unwrap_or(false) && !secure(url) {
            return false;
        }
        let name = cookie.name();
        if (name.starts_with("__Secure-") || name.starts_with("__Host-"))
            && !(cookie.secure().unwrap_or(false) && secure(url))
        {
            return false;
        }
        if name.starts_with("__Host-") && (cookie.domain().is_some() || cookie.path() != Some("/")) {
            return false;
        }
        // A Domain attribute naming a public suffix would reach every site
        // under it.
        if let Some(domain) = cookie
            .domain()
            .map(|domain| domain.trim_start_matches('.').to_ascii_lowercase())
            && url.host_str() != Some(domain.as_str())
            && psl::suffix_str(&domain) == Some(domain.as_str())
        {
            return false;
        }
        // The store's own identity for the cookie: its Domain or host, its
        // Path or default path.
        let Ok(stored) = cookie_store::Cookie::try_from_raw_cookie(&cookie, url) else {
            return false;
        };
        let key = key_of(&stored);
        if api == Api::Script
            && self
                .store
                .get_any(&key.0, &key.1, &key.2)
                .is_some_and(|existing| existing.http_only().unwrap_or(false))
        {
            return false;
        }
        let existed = self.store.get(&key.0, &key.1, &key.2).is_some();
        if self.store.insert(stored.into_owned(), url).is_err() {
            return false;
        }
        if self.store.get(&key.0, &key.1, &key.2).is_none() {
            // An expired value deletes the cookie.
            self.created.remove(&key);
        } else if !existed {
            self.sequence += 1;
            self.created.insert(key, self.sequence);
        }
        true
    }

    /// The `Cookie` header or `document.cookie` value for `url`: longest
    /// paths first, then oldest first.
    pub fn header(&self, url: &Url, api: Api, context: Context) -> String {
        let mut cookies: Vec<_> = self
            .store
            .matches(url)
            .into_iter()
            .filter(|cookie| api == Api::Http || !cookie.http_only().unwrap_or(false))
            .filter(|cookie| allowed(cookie, context))
            .collect();
        cookies.sort_by_key(|cookie| {
            (
                std::cmp::Reverse(cookie.path().map_or(0, str::len)),
                self.created.get(&key_of(cookie)).copied().unwrap_or_default(),
            )
        });
        cookies
            .iter()
            .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// `Clear-Site-Data: "cookies"`: every cookie of the response's
    /// registrable domain, host-only or not, on any of its hosts.
    pub fn clear_for(&mut self, url: &Url) {
        let Some(site) = url.host_str().map(registrable) else {
            return;
        };
        let keys: Vec<Key> = self
            .store
            .iter_any()
            .map(key_of)
            .filter(|key| registrable(&key.0) == site)
            .collect();
        for key in keys {
            self.store.remove(&key.0, &key.1, &key.2);
            self.created.remove(&key);
        }
    }

    /// The unexpired cookies of the sites of `hosts`, with their attributes,
    /// oldest first.
    pub fn cookies_of(&self, hosts: &[&str]) -> Vec<JarCookie> {
        let sites: Vec<&str> = hosts.iter().map(|host| registrable(host)).collect();
        self.in_creation_order()
            .into_iter()
            .filter(|cookie| sites.contains(&registrable(&String::from(&cookie.domain))))
            .map(|cookie| JarCookie {
                name: cookie.name().to_owned(),
                value: cookie.value().to_owned(),
                domain: String::from(&cookie.domain),
                host_only: matches!(cookie.domain, CookieDomain::HostOnly(_)),
                path: String::from(&cookie.path),
                secure: cookie.secure().unwrap_or(false),
                http_only: cookie.http_only().unwrap_or(false),
                same_site: cookie.same_site().map(|same_site| match same_site {
                    SameSite::Strict => CookieSameSite::Strict,
                    SameSite::Lax => CookieSameSite::Lax,
                    SameSite::None => CookieSameSite::None,
                }),
                expires: match &cookie.expires {
                    CookieExpiration::AtUtc(at) => Some(at.unix_timestamp()),
                    CookieExpiration::SessionEnd => None,
                },
            })
            .collect()
    }

    fn in_creation_order(&self) -> Vec<&cookie_store::Cookie<'static>> {
        let mut cookies: Vec<_> = self.store.iter_unexpired().collect();
        cookies.sort_by_key(|cookie| self.created.get(&key_of(cookie)).copied().unwrap_or_default());
        cookies
    }

    /// The jar as its file keeps it: its unexpired cookies, session cookies
    /// included, oldest first. A preview tab outlives the engine's restart
    /// as a restored browser session does, and keeps its sign-in.
    pub fn save(&self) -> Vec<u8> {
        serde_json::to_vec(&self.in_creation_order()).expect("cookies serialize")
    }

    /// The jar a file kept, its cookies created in the order they are
    /// listed. Those that expired meanwhile are dropped.
    pub fn load(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let cookies: Vec<cookie_store::Cookie<'static>> = serde_json::from_slice(bytes)?;
        let created = cookies
            .iter()
            .filter(|cookie| !cookie.is_expired())
            .map(key_of)
            .zip(1..)
            .collect::<HashMap<_, _>>();
        let sequence = created.values().copied().max().unwrap_or_default();
        let store = CookieStore::from_cookies(
            cookies.into_iter().map(Ok::<_, std::convert::Infallible>),
            false,
        )
        .unwrap_or_else(|never| match never {});
        Ok(Self {
            store,
            created,
            sequence,
        })
    }
}
