//! A page's state in the conversation's browser (`preview.md` § Page state):
//! its cookies, and its top-level origin's localStorage, sessionStorage and
//! IndexedDB, read from a tab for the user's browser, and written into a new
//! tab before its page loads. The storage is the page-state codec's, the
//! preview domain's `state.js`, which the user's browser runs too; here it
//! runs in a world of its own, so the page never sees it.

use chromiumoxide::cdp::browser_protocol::fetch::{
    DisableParams, EnableParams, EventRequestPaused, FulfillRequestParams, HeaderEntry, RequestPattern,
};
use chromiumoxide::cdp::browser_protocol::network::{
    Cookie, CookieParam, CookieSameSite, SetCookiesParams, TimeSinceEpoch,
};
use chromiumoxide::cdp::browser_protocol::page::{CreateIsolatedWorldParams, NavigateParams};
use chromiumoxide::cdp::browser_protocol::storage::GetCookiesParams;
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use demi_command_package_browser_protocol::preview::PageStorage;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::driver::operation::{BrowserError, Result};
use crate::tabs::tab::BrowserTab;

/// The page-state codec, one expression that defines `__demiPageState`. Its
/// one copy is the preview domain's published file, which the user's browser
/// loads from every preview origin; this crate includes it from
/// `services/preview-domain`, across the tree, so both browsers run the same
/// code, and a published version of it never changes.
const CODEC: &str = include_str!("../../../../services/preview-domain/static/__demi/v1/state.js");
/// The world the codec runs in, apart from the page's own scripts.
const WORLD: &str = "demi-page-state";
/// The address of the blank document a new tab's storage is written in,
/// on the target's origin; the browser never requests it.
const SEED_PATH: &str = "/__demi_page_state";

/// A cookie as a page state moves it between the two browsers.
#[derive(Debug, Clone, PartialEq)]
pub struct PageCookie {
    pub name: String,
    pub value: String,
    /// Its host for a host-only cookie, otherwise its `Domain`, without a
    /// leading dot.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// `Strict`, `Lax` or `None`, as the site wrote it; none when it wrote none.
    pub same_site: Option<&'static str>,
    /// When it expires, in seconds since the Unix epoch; none for a session
    /// cookie.
    pub expires: Option<f64>,
}

impl PageCookie {
    fn of(cookie: Cookie) -> Self {
        // A partitioned cookie becomes an ordinary one: the jar knows no
        // partitions (`preview.md` § Page state).
        Self {
            host_only: !cookie.domain.starts_with('.'),
            domain: cookie.domain.trim_start_matches('.').to_owned(),
            name: cookie.name,
            value: cookie.value,
            path: cookie.path,
            secure: cookie.secure,
            http_only: cookie.http_only,
            same_site: cookie.same_site.map(|same_site| match same_site {
                CookieSameSite::Strict => "Strict",
                CookieSameSite::Lax => "Lax",
                CookieSameSite::None => "None",
            }),
            expires: cookie.expires.filter(|_| !cookie.session),
        }
    }

    fn param(self) -> CookieParam {
        let scheme = if self.secure { "https" } else { "http" };
        let mut param = CookieParam::new(self.name, self.value);
        // A host-only cookie is set through its address, which keeps it on
        // that host alone; a domain cookie names its domain.
        if self.host_only {
            param.url = Some(format!("{scheme}://{}{}", self.domain, self.path));
        } else {
            param.domain = Some(format!(".{}", self.domain));
        }
        param.path = Some(self.path);
        param.secure = Some(self.secure);
        param.http_only = Some(self.http_only);
        param.same_site = self.same_site.map(|same_site| match same_site {
            "Strict" => CookieSameSite::Strict,
            "None" => CookieSameSite::None,
            _ => CookieSameSite::Lax,
        });
        param.expires = self.expires.map(TimeSinceEpoch::new);
        param
    }
}

/// Every cookie of the conversation's browser, `HttpOnly` ones included:
/// the browser is the conversation's alone.
pub async fn cookies(tab: &BrowserTab) -> Result<Vec<PageCookie>> {
    let browser = tab.browser().call()?;
    let cookies = browser.execute(GetCookiesParams::default()).await?.result.cookies;
    Ok(cookies.into_iter().map(PageCookie::of).collect())
}

/// The storage of the tab's top-level origin; none for a page that has no
/// web origin, such as a new tab's.
pub async fn storage(tab: &BrowserTab) -> Result<Option<PageStorage>> {
    let url = tab.page().url().await?.unwrap_or_default();
    let Some(origin) = web_origin(&url) else {
        return Ok(None);
    };
    let origin = serde_json::to_string(&origin).expect("a string serializes");
    let dumped = evaluate(tab, &format!("__demiPageState.dump({origin})")).await?;
    serde_json::from_value(dumped)
        .map(Some)
        .map_err(|error| BrowserError::InvalidResult(format!("the page state did not read: {error}")))
}

/// What writing a page state left out: the databases another page of the
/// origin kept open.
#[derive(Debug, Default, Deserialize)]
pub struct Seeded {
    pub kept: Vec<String>,
}

/// Writes `cookies` into the browser, and `storage` into its origin in the
/// tab, which shows no page yet, so the page the tab loads next finds them
/// before its first script (`preview.md` § Page state). The tab is left on a
/// blank document of that origin.
pub async fn seed(tab: &BrowserTab, cookies: Vec<PageCookie>, storage: Option<&PageStorage>) -> Result<Seeded> {
    if !cookies.is_empty() {
        let params = cookies.into_iter().map(PageCookie::param).collect();
        tab.page().execute(SetCookiesParams::new(params)).await?;
    }
    let Some(storage) = storage else {
        return Ok(Seeded::default());
    };
    let address = format!("{}{SEED_PATH}", storage.origin);
    let pattern = RequestPattern::builder().url_pattern(address.clone()).build();
    let mut paused = tab.page().event_listener::<EventRequestPaused>().await?;
    tab.page()
        .execute(EnableParams::builder().pattern(pattern).build())
        .await?;
    // The browser asks for the blank document, which its interception
    // answers, and the navigation's answer comes once it loaded.
    let blank = async {
        let request = paused
            .next()
            .await
            .ok_or_else(|| BrowserError::InvalidResult("the tab ended before its storage was written".into()))?
            .map_err(|error| BrowserError::InvalidResult(format!("the tab's requests could not be read: {error}")))?;
        let answer = FulfillRequestParams::builder()
            .request_id(request.request_id.clone())
            .response_code(200)
            .response_header(HeaderEntry::new("content-type", "text/html"))
            .build()
            .map_err(BrowserError::Configuration)?;
        tab.page().execute(answer).await?;
        Ok::<_, BrowserError>(())
    };
    let navigated = tab.page().execute(NavigateParams::new(address));
    let (answered, navigated) = futures_util::join!(blank, navigated);
    // Interception ends whatever happened, so the target loads uninterrupted.
    let disabled = tab.page().execute(DisableParams::default()).await;
    answered?;
    navigated?;
    disabled?;
    let storage = serde_json::to_string(storage).expect("a page state serializes");
    let seeded = evaluate(tab, &format!("__demiPageState.seed({storage})")).await?;
    serde_json::from_value(seeded)
        .map_err(|error| BrowserError::InvalidResult(format!("the page state's writing answered {error}")))
}

/// The origin of `url` when it is a web page's.
fn web_origin(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.origin().ascii_serialization())
}

/// Runs the codec and then `expression` in the page-state world of the
/// tab's main document, and answers the value its promise resolves to.
async fn evaluate(tab: &BrowserTab, expression: &str) -> Result<Value> {
    let frame = tab
        .page()
        .mainframe()
        .await?
        .ok_or(BrowserError::TabNotFound)?;
    let world = tab
        .page()
        .execute(
            CreateIsolatedWorldParams::builder()
                .frame_id(frame)
                .world_name(WORLD)
                .build()
                .map_err(BrowserError::Configuration)?,
        )
        .await?
        .result
        .execution_context_id;
    let evaluated = tab
        .page()
        .execute(
            EvaluateParams::builder()
                .expression(format!("{CODEC}\n{expression}"))
                .context_id(world)
                .await_promise(true)
                .return_by_value(true)
                .build()
                .map_err(BrowserError::Configuration)?,
        )
        .await?
        .result;
    if let Some(exception) = evaluated.exception_details {
        let message = exception
            .exception
            .and_then(|thrown| thrown.description)
            .unwrap_or(exception.text);
        return Err(BrowserError::InvalidResult(format!("the page state failed: {message}")));
    }
    evaluated
        .result
        .value
        .ok_or_else(|| BrowserError::InvalidResult("the page state answered nothing".into()))
}
