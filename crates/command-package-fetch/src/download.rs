//! The HTTP fetch from the Host (`web.md` § demi web fetch): the URL asked
//! for Markdown first, redirects followed to any host, the Host's proxies
//! and trust store, no cookie or credential, and the body read up to its
//! limit as text or as a file's bytes.

use std::collections::BTreeMap;

use demi_command_package_fetch_protocol::{MAX_FILE_BYTES, MAX_REDIRECTS, MAX_TEXT_BYTES};
use demi_command_protocol::is_binary;
use mime::Mime;
use reqwest::{
    Client, NoProxy, Proxy, StatusCode, Url,
    header::{ACCEPT, CONTENT_TYPE, HeaderValue},
    redirect::Policy,
};

/// What the fetch asks for: Markdown first, so a site that serves it sends
/// it, then HTML, then anything.
const ACCEPT_TYPES: &str = "text/markdown, text/html;q=0.9, */*;q=0.8";

/// The User-Agent, a crawler's form naming the program, since a site
/// refuses a request that names none.
const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; demi-fetch/",
    env!("CARGO_PKG_VERSION"),
    ")"
);

/// The header Cloudflare puts on a challenge page, which only a browser
/// running its script passes.
const CLOUDFLARE_MITIGATED: &str = "cf-mitigated";

/// The proxy variables of the calling environment, by the scheme each one
/// serves, in the order a request tries them.
const PROXY_VARIABLES: [(&str, &str); 3] = [
    ("https_proxy", "https"),
    ("http_proxy", "http"),
    ("all_proxy", "all"),
];

/// A response, its body read.
pub struct Downloaded {
    /// Where the last redirect led.
    pub final_url: Url,
    pub status: StatusCode,
    /// The media type the response names, without its parameters, such as
    /// `text/html`.
    pub media_type: Option<String>,
    pub body: Body,
}

/// A body: text, decoded by its charset, or a file's bytes.
pub enum Body {
    Text { text: String, html: bool },
    File(Vec<u8>),
}

/// Why a fetch failed; its message is what the agent reads after the
/// command's name.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error(
        "{url} answered with a Cloudflare challenge, which only a browser passes; open the page with demi browser open {url}"
    )]
    Challenge { url: Url },
    #[error("{url} redirected more than {MAX_REDIRECTS} times")]
    Redirects { url: String },
    #[error("{url}: {cause}")]
    Request { url: String, cause: String },
    #[error("{url} sends more than {} MiB of {kind}, the most it reads", .limit / (1024 * 1024))]
    TooLarge {
        url: Url,
        kind: &'static str,
        limit: u64,
    },
    #[error("the proxy {variable} names is refused: {cause}")]
    Proxy { variable: String, cause: String },
    #[error("the HTTP client could not start: {0}")]
    Client(String),
}

/// What a media type says of the body.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Html,
    Text,
    File,
    /// No media type: the bytes tell.
    Unknown,
}

impl Kind {
    fn of(media: Option<&Mime>) -> Self {
        let Some(media) = media else {
            return Self::Unknown;
        };
        let (kind, subtype) = (media.type_(), media.subtype());
        let suffix = media.suffix();
        if (kind == mime::TEXT && subtype == mime::HTML)
            || (kind == mime::APPLICATION && subtype == "xhtml")
        {
            Self::Html
        } else if kind == mime::TEXT
            || suffix == Some(mime::JSON)
            || suffix == Some(mime::XML)
            || (kind == mime::APPLICATION
                && [
                    "json",
                    "xml",
                    "javascript",
                    "ecmascript",
                    "x-javascript",
                    "x-ndjson",
                    "yaml",
                    "x-yaml",
                    "toml",
                ]
                .contains(&subtype.as_str()))
        {
            Self::Text
        } else {
            Self::File
        }
    }

    /// The most bytes a body of this kind is read up to.
    fn limit(self) -> u64 {
        match self {
            Self::Html | Self::Text => MAX_TEXT_BYTES,
            Self::File | Self::Unknown => MAX_FILE_BYTES,
        }
    }
}

/// Fetches `url` with the proxies `env`, the calling environment, names.
pub async fn download(
    url: &Url,
    env: &BTreeMap<String, String>,
) -> Result<Downloaded, DownloadError> {
    let client = client(env)?;
    let mut response = client
        .get(url.clone())
        .header(ACCEPT, HeaderValue::from_static(ACCEPT_TYPES))
        .send()
        .await
        .map_err(|error| request_error(url, &error))?;
    let final_url = response.url().clone();
    let status = response.status();
    let headers = response.headers();
    if headers
        .get(CLOUDFLARE_MITIGATED)
        .is_some_and(|value| value == "challenge")
    {
        return Err(DownloadError::Challenge { url: url.clone() });
    }
    let media: Option<Mime> = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    let kind = Kind::of(media.as_ref());
    let limit = kind.limit();
    let too_large = || DownloadError::TooLarge {
        url: url.clone(),
        kind: if limit == MAX_TEXT_BYTES { "text" } else { "a file" },
        limit,
    };
    if response.content_length().is_some_and(|length| length > limit) {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| request_error(url, &error))?
    {
        if (bytes.len() + chunk.len()) as u64 > limit {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = match kind {
        Kind::Html | Kind::Text => text(&bytes, media.as_ref(), kind == Kind::Html),
        Kind::File => Body::File(bytes),
        Kind::Unknown if is_binary(&bytes) || bytes.len() as u64 > MAX_TEXT_BYTES => {
            Body::File(bytes)
        }
        Kind::Unknown => {
            let opening = String::from_utf8_lossy(&bytes[..bytes.len().min(512)])
                .trim_start()
                .to_ascii_lowercase();
            let html = opening.starts_with("<!doctype html") || opening.starts_with("<html");
            text(&bytes, None, html)
        }
    };
    Ok(Downloaded {
        final_url,
        status,
        media_type: media.map(|media| media.essence_str().to_owned()),
        body,
    })
}

/// The client of one fetch: at most [`MAX_REDIRECTS`] redirects, to any
/// host; no cookie store; the proxies the calling environment names, or
/// else the Host's own, which reqwest reads from the process's environment
/// and the system's settings.
fn client(env: &BTreeMap<String, String>) -> Result<Client, DownloadError> {
    let variable = |name: &str| {
        env.get(name)
            .or_else(|| env.get(&name.to_ascii_uppercase()))
            .filter(|value| !value.is_empty())
    };
    let no_proxy = variable("no_proxy").and_then(|list| NoProxy::from_string(list));
    let mut builder = Client::builder()
        .redirect(Policy::limited(MAX_REDIRECTS))
        .user_agent(USER_AGENT);
    for (name, scheme) in PROXY_VARIABLES {
        let Some(address) = variable(name) else {
            continue;
        };
        let proxy = match scheme {
            "https" => Proxy::https(address),
            "http" => Proxy::http(address),
            _ => Proxy::all(address),
        }
        .map_err(|error| DownloadError::Proxy {
            variable: name.to_owned(),
            cause: cause(&error),
        })?;
        builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
    }
    builder
        .build()
        .map_err(|error| DownloadError::Client(cause(&error)))
}

/// A body read as text, decoded by the charset its media type names, or
/// else as UTF-8; a byte order mark decides over both.
fn text(bytes: &[u8], media: Option<&Mime>, html: bool) -> Body {
    let encoding = media
        .and_then(|media| media.get_param(mime::CHARSET))
        .and_then(|charset| encoding_rs::Encoding::for_label(charset.as_str().as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(bytes);
    Body::Text {
        text: text.into_owned(),
        html,
    }
}

fn request_error(url: &Url, error: &reqwest::Error) -> DownloadError {
    let url = error.url().unwrap_or(url).to_string();
    if error.is_redirect() {
        DownloadError::Redirects { url }
    } else {
        DownloadError::Request {
            url,
            cause: cause(error),
        }
    }
}

/// The innermost cause of `error`, which names what went wrong, such as
/// `Connection refused`, where the outer ones only say a request failed.
fn cause(error: &(dyn std::error::Error + 'static)) -> String {
    let mut innermost = error;
    while let Some(source) = innermost.source() {
        innermost = source;
    }
    match innermost.downcast_ref::<std::io::Error>() {
        Some(error) => demi_command_sdk::errors::reason(error),
        None => innermost.to_string(),
    }
}
