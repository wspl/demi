//! The plugins' outbound requests: one
//! HTTPS request through the client the product gives every plugin, with
//! the deadline and the body limit the plugin names and the request's
//! cancellation. A log line names the plugin, the host and the status,
//! never a header or a body.

use demi_plugin_interface::{
    PluginId, PortFailure, PortRefusal, WebFailure, WebHeader, WebMethod, WebRequest, WebResponse,
};
use demi_shared_types::B64Bytes;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tokio_util::sync::CancellationToken;

/// The client of a shard thread's plugins' outbound requests: it sends only
/// `https` and follows no redirect away from it, and trusts the system's
/// authorities and `roots`, which only a test names.
pub fn client(roots: &[reqwest::Certificate]) -> reqwest::Client {
    roots
        .iter()
        .cloned()
        .fold(
            reqwest::Client::builder().https_only(true),
            reqwest::ClientBuilder::add_root_certificate,
        )
        .build()
        // No setting of this builder fails: the shard thread's other client
        // is built the same way with `reqwest::Client::new`.
        .expect("the plugins' HTTPS client builds")
}

/// Sends `request` of `plugin` through `client`, which refuses a redirect
/// away from `https`, and answers the vendor's response whatever its
/// status.
pub(crate) async fn send(
    client: &reqwest::Client,
    plugin: &PluginId,
    request: WebRequest,
    cancel: &CancellationToken,
) -> Result<WebResponse, PortFailure> {
    let host = request.url.host_str().unwrap_or_default().to_owned();
    let answered = tokio::select! {
        answered = tokio::time::timeout(request.deadline(), exchange(client, request)) => answered,
        () = cancel.cancelled() => return Err(refusal(WebFailure::Unreachable {
            message: "the request was cancelled".into(),
        })),
    };
    let answered = answered.unwrap_or(Err(WebFailure::TimedOut));
    match &answered {
        Ok(response) => {
            tracing::info!(%plugin, host, status = response.status, "a plugin's outbound request")
        }
        Err(failure) => {
            tracing::info!(%plugin, host, %failure, "a plugin's outbound request failed")
        }
    }
    answered.map_err(refusal)
}

fn refusal(failure: WebFailure) -> PortFailure {
    PortFailure::Refused(PortRefusal::Web { failure })
}

/// The request's checks, its exchange and its body read to the limit.
async fn exchange(client: &reqwest::Client, request: WebRequest) -> Result<WebResponse, WebFailure> {
    if request.url.scheme() != "https" {
        return Err(WebFailure::NotHttps);
    }
    if request.deadline_ms == 0 || request.deadline() > WebRequest::MAX_DEADLINE {
        return Err(invalid(format!(
            "the deadline is {} ms; it is 1 to {} ms",
            request.deadline_ms,
            WebRequest::MAX_DEADLINE.as_millis()
        )));
    }
    if request.limit_bytes == 0 || request.limit_bytes > WebRequest::MAX_BODY_BYTES {
        return Err(invalid(format!(
            "the body limit is {} bytes; it is 1 to {} bytes",
            request.limit_bytes,
            WebRequest::MAX_BODY_BYTES
        )));
    }
    let mut headers = HeaderMap::new();
    for header in &request.headers {
        let name = HeaderName::try_from(header.name.as_str())
            .map_err(|_| invalid(format!("\"{}\" is not a header's name", header.name)))?;
        let mut value = HeaderValue::try_from(header.value.as_str())
            .map_err(|_| invalid(format!("the value of \"{name}\" is not a header's value")))?;
        // A header may hold a key or a token.
        value.set_sensitive(true);
        headers.append(name, value);
    }
    let method = match request.method {
        WebMethod::Get => reqwest::Method::GET,
        WebMethod::Head => reqwest::Method::HEAD,
        WebMethod::Post => reqwest::Method::POST,
        WebMethod::Put => reqwest::Method::PUT,
        WebMethod::Patch => reqwest::Method::PATCH,
        WebMethod::Delete => reqwest::Method::DELETE,
    };
    let mut builder = client.request(method, request.url).headers(headers);
    if let Some(body) = request.body {
        builder = builder.body(body.into_bytes());
    }
    let mut response = builder.send().await.map_err(failure)?;
    let limit = request.limit_bytes;
    if response.content_length().is_some_and(|length| length > limit) {
        return Err(WebFailure::TooLarge);
    }
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            // A value that is not text, which no plugin could read, is left out.
            Some(WebHeader {
                name: name.as_str().to_owned(),
                value: value.to_str().ok()?.to_owned(),
            })
        })
        .collect();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(failure)? {
        if (body.len() + chunk.len()) as u64 > limit {
            return Err(WebFailure::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(WebResponse {
        status,
        headers,
        body: B64Bytes::new(body),
    })
}

fn invalid(message: String) -> WebFailure {
    WebFailure::Invalid { message }
}

/// A failure of the client: a redirect away from `https` the client
/// refused, or no answer.
fn failure(error: reqwest::Error) -> WebFailure {
    if error.is_redirect() && error.url().is_some_and(|url| url.scheme() != "https") {
        return WebFailure::NotHttps;
    }
    WebFailure::Unreachable {
        message: error.without_url().to_string(),
    }
}
