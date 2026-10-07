//! A preview document's WebSockets (`preview.md` § The forwarder and the
//! relay): a service worker never sees them, so the page's runtime asks the
//! relay, which sends `socket_open` on the stream, and the engine connects
//! upstream from the Host with the Host's cookies and the logical `Origin`.

use demi_command_package_browser_protocol::preview::PreviewClient;
use demi_preview_rewrite::address::Environment;
use http::{HeaderMap, HeaderValue, header};
use url::Url;

use crate::engine::{Engine, socket_context};
use crate::upstream::{UpstreamError, check, refused_by_local_network};

/// A socket open upstream, with what the server chose.
pub(crate) struct Opened {
    pub socket: wreq::ws::WebSocket,
    pub protocol: String,
    pub extensions: String,
}

/// Connects the socket `environment`'s document asks for: no further than
/// the document's network, as a subresource of it.
pub(crate) async fn connect(
    engine: &Engine,
    environment: &Environment,
    address: &str,
    protocols: Vec<String>,
    client: &PreviewClient,
) -> Result<Opened, String> {
    let url = Url::parse(address).map_err(|error| format!("not an address: {error}"))?;
    if !matches!(url.scheme(), "ws" | "wss") {
        return Err(format!("not a WebSocket address: {address}"));
    }
    check(&url).map_err(|error| error.to_string())?;
    let context = socket_context(environment, &url);
    let mut headers = HeaderMap::new();
    for (name, value) in [
        (header::USER_AGENT, &client.user_agent),
        (header::ACCEPT_LANGUAGE, &client.accept_language),
        (header::ORIGIN, &environment.origin),
    ] {
        // A value HTTP cannot carry is one the browser would not send.
        if let Ok(value) = HeaderValue::try_from(value.as_str()) {
            headers.insert(name, value);
        }
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    let cookie = engine.cookie_header(&url, context);
    if let Ok(cookie) = HeaderValue::try_from(cookie)
        && !cookie.is_empty()
    {
        headers.insert(header::COOKIE, cookie);
    }
    let limit = engine.space_of_origin(&environment.origin).await;
    let client = engine
        .upstream
        .client_for(&client.user_agent, &url, limit)
        .map_err(|error| refusal(&error))?;
    let response = client
        .websocket(url.as_str())
        .headers(headers)
        .protocols(protocols)
        .send()
        .await
        .map_err(|error| failure(&error))?;
    engine.store_set_cookies(response.headers(), &url, context);
    let extensions = response
        .headers()
        .get(header::SEC_WEBSOCKET_EXTENSIONS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let socket = response.into_websocket().await.map_err(|error| failure(&error))?;
    let protocol = socket
        .protocol()
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    Ok(Opened {
        socket,
        protocol,
        extensions,
    })
}

fn refusal(error: &UpstreamError) -> String {
    match error {
        UpstreamError::LocalNetwork => "refused: local-network".into(),
        other => other.to_string(),
    }
}

fn failure(error: &wreq::Error) -> String {
    if refused_by_local_network(error) {
        "refused: local-network".into()
    } else {
        error.to_string()
    }
}
