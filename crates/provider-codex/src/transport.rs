//! How a Codex request reaches the Responses backend (`providers.md` §
//! Endpoints): over a WebSocket as one `response.create` message, over
//! server-sent events, or the WebSocket first with server-sent events as the
//! fallback. Every answer of the service, a refusal included, is shown to the
//! account's quota.

use std::{sync::atomic::Ordering, time::Duration};

use demi_provider::{
    ProviderFailure,
    quota::Observation,
    wire::{
        WireError, decode_tagged, decode_untagged,
        responses::{Received, ResponsesEvent, sse_events},
        undecodable,
    },
};
use futures_util::{SinkExt, StreamExt, stream::LocalBoxStream};
use http::{
    HeaderMap, HeaderValue, StatusCode,
    header::{ACCEPT, CONTENT_TYPE},
};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::{
    self, Message,
    client::IntoClientRequest,
    protocol::{CloseFrame, frame::coding::CloseCode},
};
use tokio_util::sync::CancellationToken;

use crate::{LABEL, Shared, TransportMode, request::Encoded};

/// The events of an opened request, each with the text it arrived as, until
/// the response ends or the token fires.
pub(crate) type Events = LocalBoxStream<'static, Result<Received, ProviderFailure>>;

/// Why a request did not open.
pub(crate) enum OpenError {
    /// The service answered with a failure status.
    Refused(Refusal),
    /// No answer: the connection failed or timed out.
    Failed(ProviderFailure),
}

/// A failure answer, whole: the record keeps it.
#[derive(Debug, Clone)]
pub(crate) struct Refusal {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: String,
}

/// Opens the request by the provider's transport. With both, a WebSocket
/// that fails before its first event gives way to server-sent events unless
/// the run was cancelled; after its first event, its failure is the run's.
/// A WebSocket that cannot connect at all, which is how networks that block
/// WebSockets show, sends the provider's later requests over server-sent
/// events at once.
pub(crate) async fn open(
    shared: &Shared,
    http: &reqwest::Client,
    headers: &HeaderMap,
    body: &Encoded,
    cancel: &CancellationToken,
) -> Result<Events, OpenError> {
    match shared.transport {
        TransportMode::Sse => sse(shared, http, headers, body, cancel).await,
        TransportMode::WebSocket => websocket(shared, headers, body, cancel).await,
        TransportMode::Auto => {
            if shared.websocket_unreachable.load(Ordering::Relaxed) {
                return sse(shared, http, headers, body, cancel).await;
            }
            match websocket(shared, headers, body, cancel).await {
                Ok(mut events) => match events.next().await {
                    Some(Ok(first)) => {
                        return Ok(futures_util::stream::once(async { Ok(first) })
                            .chain(events)
                            .boxed_local());
                    }
                    Some(Err(_)) | None if cancel.is_cancelled() => {
                        return Ok(futures_util::stream::empty().boxed_local());
                    }
                    // An error or a close before the first event.
                    Some(Err(_)) | None => {}
                },
                Err(OpenError::Failed(_)) => {
                    shared.websocket_unreachable.store(true, Ordering::Relaxed);
                }
                Err(OpenError::Refused(_)) => {}
            }
            sse(shared, http, headers, body, cancel).await
        }
    }
}

/// The request over server-sent events. The response's headers must arrive
/// within the header timeout; dropping the stream stops the download.
async fn sse(
    shared: &Shared,
    http: &reqwest::Client,
    headers: &HeaderMap,
    body: &Encoded,
    cancel: &CancellationToken,
) -> Result<Events, OpenError> {
    let send = http
        .post(shared.responses_url.clone())
        .headers(headers.clone())
        .body(body.http.clone())
        .send();
    let response = match tokio::time::timeout(shared.header_timeout, send).await {
        Err(_) => {
            let message = format!(
                "Codex SSE response headers timed out after {}ms",
                shared.header_timeout.as_millis()
            );
            return Err(OpenError::Failed(ProviderFailure::no_answer(message)));
        }
        Ok(Err(error)) => return Err(OpenError::Failed(ProviderFailure::transport(LABEL, error))),
        Ok(Ok(response)) => response,
    };
    shared.quota.observe(Observation::Response {
        status: response.status(),
        headers: response.headers(),
    });
    if !response.status().is_success() {
        let status = response.status();
        let headers = response.headers().clone();
        // An unreadable body counts as empty; the status and headers still
        // say what failed.
        let body = response.text().await.unwrap_or_default();
        return Err(OpenError::Refused(Refusal {
            status,
            headers,
            body,
        }));
    }
    Ok(sse_events(response.bytes_stream(), LABEL, cancel.clone()).boxed_local())
}

/// The request over a WebSocket: the request's headers but `accept` and
/// `content-type`, the WebSocket beta, a connect timeout, and an optional
/// idle timeout between messages.
async fn websocket(
    shared: &Shared,
    headers: &HeaderMap,
    body: &Encoded,
    cancel: &CancellationToken,
) -> Result<Events, OpenError> {
    let Some(message) = body.websocket.clone() else {
        // The body is encoded with its WebSocket message whenever the
        // transport may use one.
        let failure = ProviderFailure::no_answer("Codex WebSocket request was not built");
        return Err(OpenError::Failed(failure));
    };
    let mut request = match shared.websocket_url.as_str().into_client_request() {
        Ok(request) => request,
        Err(error) => {
            let failure = ProviderFailure::no_answer(format!(
                "Codex WebSocket request cannot be built: {error}"
            ));
            return Err(OpenError::Failed(failure));
        }
    };
    for (name, value) in headers {
        if name != ACCEPT && name != CONTENT_TYPE {
            request.headers_mut().insert(name.clone(), value.clone());
        }
    }
    request.headers_mut().insert(
        "openai-beta",
        HeaderValue::from_static("responses_websockets=2026-02-06"),
    );
    let connect = tokio_tungstenite::connect_async(request);
    let (mut socket, response) = match tokio::time::timeout(shared.connect_timeout, connect).await {
        Err(_) => {
            let message = format!(
                "Codex WebSocket connect timed out after {}ms",
                shared.connect_timeout.as_millis()
            );
            return Err(OpenError::Failed(ProviderFailure::no_answer(message)));
        }
        Ok(Err(tungstenite::Error::Http(response))) => {
            shared.quota.observe(Observation::Response {
                status: response.status(),
                headers: response.headers(),
            });
            let (parts, body) = (*response).into_parts();
            let body = String::from_utf8_lossy(body.as_deref().unwrap_or_default()).into_owned();
            let refusal = Refusal {
                status: parts.status,
                headers: parts.headers,
                body,
            };
            return Err(OpenError::Refused(refusal));
        }
        Ok(Err(error)) => {
            let failure =
                ProviderFailure::no_answer(format!("Codex WebSocket connect failed: {error}"));
            return Err(OpenError::Failed(failure));
        }
        Ok(Ok(connected)) => connected,
    };
    shared.quota.observe(Observation::Response {
        status: response.status(),
        headers: response.headers(),
    });
    if let Err(error) = socket.send(Message::Text(message)).await {
        // The socket connected, so a failed send fails its stream before the
        // first event, not the connection.
        let failure = ProviderFailure::no_answer(format!("Codex WebSocket send failed: {error}"));
        return Ok(futures_util::stream::once(async { Err(failure) }).boxed_local());
    }
    Ok(websocket_events(socket, shared.stream_idle_timeout, cancel.clone()).boxed_local())
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The events a WebSocket sends. A terminal event ends the stream after the
/// socket is closed with `response_done`; cancellation closes it with
/// `aborted`; a socket that idles past `idle` fails and closes with
/// `idle_timeout`. Dropping the stream drops the connection.
fn websocket_events(
    mut socket: Socket,
    idle: Option<Duration>,
    cancel: CancellationToken,
) -> impl futures_util::Stream<Item = Result<Received, ProviderFailure>> {
    async_stream::stream! {
        loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    close(&mut socket, "aborted").await;
                    return;
                }
                next = next_message(&mut socket, idle) => next,
            };
            let text = match next {
                Next::Idle(idle) => {
                    close(&mut socket, "idle_timeout").await;
                    let message = format!("Codex WebSocket stream idled for {}ms", idle.as_millis());
                    yield Err(ProviderFailure::no_answer(message));
                    return;
                }
                Next::Closed => return,
                Next::Failed(error) => {
                    yield Err(ProviderFailure::no_answer(format!("Codex WebSocket failed: {error}")));
                    return;
                }
                Next::NotText(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    let message = format!("{LABEL} API stream sent a binary message that is not UTF-8 text");
                    yield Err(ProviderFailure::protocol(message, text));
                    return;
                }
                Next::Text(text) => text,
            };
            match decode_message(&text) {
                Ok(None) => {}
                Ok(Some(received)) => {
                    let terminal = matches!(
                        received.event,
                        ResponsesEvent::Completed(_)
                            | ResponsesEvent::Failed(_)
                            | ResponsesEvent::Incomplete(_)
                            | ResponsesEvent::Error(_)
                    );
                    if terminal {
                        close(&mut socket, "response_done").await;
                    }
                    yield Ok(received);
                    if terminal {
                        return;
                    }
                }
                Err(error) => {
                    yield Err(undecodable(LABEL, &error, &text));
                    return;
                }
            }
        }
    }
}

enum Next {
    Text(String),
    NotText(Vec<u8>),
    Closed,
    Failed(tungstenite::Error),
    Idle(Duration),
}

/// The next data message: text, or binary read as UTF-8 text. Control
/// messages are the library's; a close ends the stream.
async fn next_message(socket: &mut Socket, idle: Option<Duration>) -> Next {
    loop {
        let message = match idle {
            None => socket.next().await,
            Some(idle) => match tokio::time::timeout(idle, socket.next()).await {
                Ok(message) => message,
                Err(_) => return Next::Idle(idle),
            },
        };
        match message {
            None | Some(Ok(Message::Close(_))) => return Next::Closed,
            Some(Err(error)) => return Next::Failed(error),
            Some(Ok(Message::Text(text))) => return Next::Text(text.as_str().to_owned()),
            Some(Ok(Message::Binary(bytes))) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => return Next::Text(text),
                Err(error) => return Next::NotText(error.into_bytes()),
            },
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
        }
    }
}

/// Closes the socket with `reason`, best effort: the stream ends either way.
async fn close(socket: &mut Socket, reason: &'static str) {
    let frame = CloseFrame {
        code: CloseCode::Normal,
        reason: reason.into(),
    };
    // A socket the vendor already closed has nothing to receive the frame.
    let _ = socket.close(Some(frame)).await;
}

/// The envelope the Codex WebSocket wraps an event in: a completed response
/// arrives as `response.done` with the response beside it, other events
/// nested under `event`, the rest as themselves.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    event: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    response: Option<serde_json::Value>,
}

/// Decodes one message: `None` for an event type Demi does not map.
fn decode_message(text: &str) -> Result<Option<Received>, WireError> {
    let envelope: Envelope = decode_untagged(text)?;
    let event = if envelope.kind.as_deref() == Some("response.done") {
        let completed =
            serde_json::json!({ "type": "response.completed", "response": envelope.response });
        decode_tagged::<ResponsesEvent>(&completed.to_string())?
    } else if let Some(event) = envelope.event {
        decode_tagged::<ResponsesEvent>(&serde_json::Value::Object(event).to_string())?
    } else {
        decode_tagged::<ResponsesEvent>(text)?
    };
    Ok(event.map(|event| Received {
        event,
        text: text.to_owned(),
    }))
}
