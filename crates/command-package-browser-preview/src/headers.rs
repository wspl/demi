//! The request headers Chrome sends, rebuilt for an upstream request
//! (`preview.md` § Upstream requests). A request reaches the engine from the
//! page's forwarder, which sees only the page's own headers and `Accept`;
//! the rest are the user's browser's (its client hints and languages), the
//! logical ones the engine computes, and Chrome's fixed values, in the
//! order Chrome sends them.

use demi_command_package_browser_protocol::preview::{PreviewClient, PreviewHeader};
use http::{HeaderMap, HeaderName, HeaderValue};

/// The logical request: what Chrome would send for it, apart from its
/// client's headers.
pub(crate) struct Request<'a> {
    pub method: &'a str,
    /// `navigate`, `cors`, `no-cors` or `same-origin`.
    pub mode: &'a str,
    /// The fetch destination; empty for fetch and XHR.
    pub destination: &'a str,
    pub top_level: bool,
    pub user: bool,
    /// `none`, `same-origin`, `same-site` or `cross-site`.
    pub site: &'a str,
    pub origin: Option<&'a str>,
    pub referrer: Option<&'a str>,
    pub cookie: Option<&'a str>,
    /// The page's own headers and `Accept`, as the forwarder saw them, in
    /// their order.
    pub page: &'a [PreviewHeader],
    pub content_length: Option<usize>,
    pub keepalive: bool,
}

const ACCEPT_ENCODING: &str = "gzip, deflate, br, zstd";
const DOCUMENT_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/jxl,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7";
const IMAGE_ACCEPT: &str = "image/jxl,image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8";
/// Headers the engine writes itself; a copy the forwarder saw is ignored.
const WRITTEN: [&str; 6] = [
    "accept",
    "accept-language",
    "content-type",
    "priority",
    "upgrade-insecure-requests",
    "user-agent",
];
/// Header names a page cannot set (Fetch § forbidden request-header): the
/// browser added them.
const FORBIDDEN: [&str; 21] = [
    "accept-charset",
    "accept-encoding",
    "access-control-request-headers",
    "access-control-request-method",
    "connection",
    "content-length",
    "cookie",
    "cookie2",
    "date",
    "dnt",
    "expect",
    "host",
    "keep-alive",
    "origin",
    "referer",
    "set-cookie",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "via",
];

/// Whether a header the forwarder saw is the page's own.
pub(crate) fn is_page_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !FORBIDDEN.contains(&name.as_str())
        && !name.starts_with("sec-")
        && !name.starts_with("proxy-")
        && !WRITTEN.contains(&name.as_str())
}

fn default_accept(destination: &str, navigation: bool) -> &'static str {
    match destination {
        _ if navigation => DOCUMENT_ACCEPT,
        "image" => IMAGE_ACCEPT,
        "style" => "text/css,*/*;q=0.1",
        _ => "*/*",
    }
}

/// Chrome's priority for a destination, as its first request of that kind
/// carries it.
fn priority(request: &Request) -> &'static str {
    match request.destination {
        _ if request.mode == "navigate" => "u=0, i",
        _ if request.keepalive && request.method != "GET" => "u=4, i",
        "style" | "font" => "u=0",
        "script" if request.mode == "cors" => "u=1",
        "script" => "u=2",
        "image" => "u=2, i",
        _ => "u=1, i",
    }
}

struct Builder(HeaderMap);

impl Builder {
    /// Appends a header; a name or value HTTP cannot carry is left out, as
    /// the browser would have refused it from the page.
    fn put(&mut self, name: &str, value: Option<&str>) {
        if let (Some(value), Ok(name)) = (value, HeaderName::try_from(name))
            && let Ok(value) = HeaderValue::try_from(value)
        {
            self.0.append(name, value);
        }
    }
}

pub(crate) fn page_header<'a>(page: &'a [PreviewHeader], name: &str) -> Option<&'a str> {
    page.iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

pub(crate) fn chrome_headers(client: &PreviewClient, request: &Request) -> HeaderMap {
    let navigation = request.mode == "navigate";
    let accept = page_header(request.page, "accept")
        .unwrap_or_else(|| default_accept(request.destination, navigation));
    let content_type = page_header(request.page, "content-type");
    let length = request.content_length.map(|length| length.to_string());
    let platform = format!("\"{}\"", client.platform);
    let mobile = if client.mobile { "?1" } else { "?0" };
    let destination = match request.destination {
        _ if navigation && request.top_level => "document",
        "" if navigation => "iframe",
        "" => "empty",
        other => other,
    };
    let mut headers = Builder(HeaderMap::new());
    headers.put("content-length", length.as_deref());
    if navigation {
        headers.put("sec-ch-ua", Some(&client.brands));
        headers.put("sec-ch-ua-mobile", Some(mobile));
        headers.put("sec-ch-ua-platform", Some(&platform));
        headers.put("upgrade-insecure-requests", Some("1"));
        headers.put("content-type", content_type);
        headers.put("user-agent", Some(&client.user_agent));
        headers.put("origin", request.origin);
        headers.put("accept", Some(accept));
        headers.put("sec-fetch-site", Some(request.site));
        headers.put("sec-fetch-mode", Some("navigate"));
        headers.put("sec-fetch-user", (request.top_level || request.user).then_some("?1"));
        headers.put("sec-fetch-dest", Some(destination));
    } else {
        for header in request.page {
            if is_page_header(&header.name) {
                headers.put(&header.name, Some(&header.value));
            }
        }
        let event_stream = accept == "text/event-stream";
        headers.put("sec-ch-ua-platform", Some(&platform));
        headers.put("user-agent", Some(&client.user_agent));
        // An EventSource sets Accept itself, before the client hints.
        headers.put("accept", event_stream.then_some(accept));
        headers.put("sec-ch-ua", Some(&client.brands));
        headers.put("content-type", content_type);
        headers.put("sec-ch-ua-mobile", Some(mobile));
        headers.put("accept", (!event_stream).then_some(accept));
        headers.put("origin", request.origin);
        headers.put("sec-fetch-site", Some(request.site));
        headers.put("sec-fetch-mode", Some(request.mode));
        headers.put("sec-fetch-dest", Some(destination));
    }
    headers.put("referer", request.referrer);
    headers.put("accept-encoding", Some(ACCEPT_ENCODING));
    headers.put("accept-language", Some(&client.accept_language));
    headers.put("cookie", request.cookie);
    headers.put("priority", Some(priority(request)));
    headers.0
}

/// A CORS preflight for a logical request, in Chrome's order.
pub(crate) fn preflight_headers(
    client: &PreviewClient,
    method: &str,
    author: &[String],
    origin: &str,
    site: &str,
    referrer: Option<&str>,
) -> HeaderMap {
    let mut headers = Builder(HeaderMap::new());
    headers.put("accept", Some("*/*"));
    headers.put("access-control-request-method", Some(method));
    headers.put(
        "access-control-request-headers",
        (!author.is_empty()).then(|| author.join(",")).as_deref(),
    );
    headers.put("origin", Some(origin));
    headers.put("user-agent", Some(&client.user_agent));
    headers.put("sec-fetch-mode", Some("cors"));
    headers.put("sec-fetch-site", Some(site));
    headers.put("sec-fetch-dest", Some("empty"));
    headers.put("referer", referrer);
    headers.put("accept-encoding", Some(ACCEPT_ENCODING));
    headers.put("accept-language", Some(&client.accept_language));
    headers.put("priority", Some("u=1, i"));
    headers.0
}
