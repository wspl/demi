//! WebSocket client over `tokio-tungstenite`. Sends resolve once the frame
//! is flushed to the kernel, which is the backpressure a caller sees;
//! receives are pull-model and resolve to `null` once the peer closes.

use std::io;
use std::rc::Rc;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use hyper::http::Uri;
use rquickjs::function::Opt;
use rquickjs::{Ctx, Object, Result, TypedArray, Value};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use super::connect::{self, Conn};
use super::tls;
use crate::error::{bad_handle, busy_handle, throw_code, throw_io};
use crate::handles::{Resource, Slot};
use crate::state::{state, Activity};

type Socket = WebSocketStream<MaybeTlsStream<Conn>>;

pub struct WsResource {
    pub tx: Slot<SplitSink<Socket, Message>>,
    pub rx: Slot<SplitStream<Socket>>,
    pub cancel: Rc<Notify>,
}

fn throw_ws(ctx: &Ctx<'_>, err: WsError, syscall: &str) -> rquickjs::Error {
    match err {
        WsError::Io(e) => throw_io(ctx, e, syscall, None),
        WsError::Tls(e) => throw_code(ctx, "EPROTO", &format!("TLS: {e}"), syscall, None),
        WsError::Url(e) => throw_code(ctx, "EINVAL", &e.to_string(), syscall, None),
        WsError::Http(r) => throw_code(ctx, "EPROTO", &format!("handshake rejected with status {}", r.status()), syscall, None),
        WsError::ConnectionClosed | WsError::AlreadyClosed => throw_code(ctx, "EPIPE", "connection closed", syscall, None),
        other => throw_code(ctx, "EPROTO", &other.to_string(), syscall, None),
    }
}

pub async fn ws_connect<'js>(ctx: Ctx<'js>, url: String, options: Opt<Object<'js>>) -> Result<i32> {
    let _activity = Activity::begin(&ctx);
    let mut request = url.clone().into_client_request().map_err(|e| throw_ws(&ctx, e, "connect"))?;
    if let Some(o) = options.0 {
        if let Some(headers) = o.get::<_, Option<Object>>("headers")? {
            for r in headers.props::<String, String>() {
                let (k, v) = r?;
                let name = hyper::http::HeaderName::from_bytes(k.as_bytes()).map_err(|e| throw_code(&ctx, "EINVAL", &e.to_string(), "connect", None))?;
                let value = hyper::http::HeaderValue::from_str(&v).map_err(|e| throw_code(&ctx, "EINVAL", &e.to_string(), "connect", None))?;
                request.headers_mut().insert(name, value);
            }
        }
    }
    let uri: Uri = request.uri().clone();
    let secure = uri.scheme_str() == Some("wss");
    // The transport is opened here (proxy-aware); tungstenite does TLS on
    // top with the shell's client configuration.
    let target: Uri = url.replacen("ws", "http", 1).parse().map_err(|_| throw_code(&ctx, "EINVAL", "invalid URL", "connect", None))?;
    let connected = connect::open(&target, true).await.map_err(|e| throw_io(&ctx, e, "connect", None))?;
    let connector = if secure {
        Some(Connector::Rustls(tls::client_config().map_err(|e| throw_code(&ctx, "EPROTO", &e, "connect", None))?))
    } else {
        Some(Connector::Plain)
    };
    let (socket, _response) = tokio_tungstenite::client_async_tls_with_config(request, connected.stream, None, connector)
        .await
        .map_err(|e| throw_ws(&ctx, e, "connect"))?;
    let (tx, rx) = socket.split();
    Ok(state(&ctx).handles.borrow_mut().insert(Resource::Ws(WsResource {
        tx: Slot::Ready(tx),
        rx: Slot::Ready(rx),
        cancel: Rc::new(Notify::new()),
    })))
}

fn take_tx<'js>(ctx: &Ctx<'js>, id: i32, syscall: &str) -> Result<(SplitSink<Socket, Message>, Rc<Notify>)> {
    let st = state(ctx);
    let mut handles = st.handles.borrow_mut();
    match handles.get_mut(id) {
        Some(Resource::Ws(ws)) => match std::mem::replace(&mut ws.tx, Slot::Busy) {
            Slot::Ready(tx) => Ok((tx, ws.cancel.clone())),
            Slot::Busy => Err(busy_handle(ctx, syscall)),
            Slot::Absent => Err(bad_handle(ctx, syscall)),
        },
        _ => Err(bad_handle(ctx, syscall)),
    }
}

fn restore_tx<'js>(ctx: &Ctx<'js>, id: i32, tx: SplitSink<Socket, Message>) {
    if let Some(Resource::Ws(ws)) = state(ctx).handles.borrow_mut().get_mut(id) {
        ws.tx = Slot::Ready(tx);
    }
}

fn take_rx<'js>(ctx: &Ctx<'js>, id: i32) -> Result<(SplitStream<Socket>, Rc<Notify>)> {
    let st = state(ctx);
    let mut handles = st.handles.borrow_mut();
    match handles.get_mut(id) {
        Some(Resource::Ws(ws)) => match std::mem::replace(&mut ws.rx, Slot::Busy) {
            Slot::Ready(rx) => Ok((rx, ws.cancel.clone())),
            Slot::Busy => Err(busy_handle(ctx, "recv")),
            Slot::Absent => Err(bad_handle(ctx, "recv")),
        },
        _ => Err(bad_handle(ctx, "recv")),
    }
}

fn restore_rx<'js>(ctx: &Ctx<'js>, id: i32, rx: SplitStream<Socket>) {
    if let Some(Resource::Ws(ws)) = state(ctx).handles.borrow_mut().get_mut(id) {
        ws.rx = Slot::Ready(rx);
    }
}

pub async fn ws_send<'js>(ctx: Ctx<'js>, id: i32, data: TypedArray<'js, u8>) -> Result<()> {
    let _activity = Activity::begin(&ctx);
    let (mut tx, cancel) = take_tx(&ctx, id, "send")?;
    let bytes = bytes::Bytes::copy_from_slice(data.as_bytes().unwrap_or(&[]));
    let r = tokio::select! {
        r = tx.send(Message::Binary(bytes)) => r,
        _ = cancel.notified() => Err(WsError::Io(io::Error::from_raw_os_error(libc::ECANCELED))),
    };
    restore_tx(&ctx, id, tx);
    r.map_err(|e| throw_ws(&ctx, e, "send"))
}

/// Resolves to the next binary or text frame's bytes, or `null` once the
/// connection is closed. Ping and pong frames are answered by the library.
pub async fn ws_recv<'js>(ctx: Ctx<'js>, id: i32) -> Result<Value<'js>> {
    let _activity = Activity::begin(&ctx);
    let (mut rx, cancel) = take_rx(&ctx, id)?;
    let result = loop {
        let next = tokio::select! {
            m = rx.next() => m,
            _ = cancel.notified() => Some(Err(WsError::Io(io::Error::from_raw_os_error(libc::ECANCELED)))),
        };
        match next {
            None => break Ok(None),
            Some(Ok(Message::Binary(b))) => break Ok(Some(b.to_vec())),
            Some(Ok(Message::Text(t))) => break Ok(Some(t.as_bytes().to_vec())),
            Some(Ok(Message::Close(_))) => break Ok(None),
            Some(Ok(_)) => continue,
            Some(Err(WsError::ConnectionClosed)) | Some(Err(WsError::AlreadyClosed)) => break Ok(None),
            Some(Err(e)) => break Err(e),
        }
    };
    restore_rx(&ctx, id, rx);
    match result {
        Ok(None) => Ok(Value::new_null(ctx)),
        Ok(Some(bytes)) => Ok(TypedArray::new(ctx, bytes)?.into_value()),
        Err(e) => Err(throw_ws(&ctx, e, "recv")),
    }
}

/// Sends a close frame and releases the handle. A pending receive on the
/// same handle ends with `null` when the peer answers, or `ECANCELED`.
pub async fn ws_close<'js>(ctx: Ctx<'js>, id: i32, code: Opt<u16>) -> Result<()> {
    let _activity = Activity::begin(&ctx);
    let (mut tx, cancel) = take_tx(&ctx, id, "close")?;
    let frame = CloseFrame { code: code.0.unwrap_or(1000).into(), reason: "".into() };
    let sent = tokio::time::timeout(std::time::Duration::from_secs(5), tx.send(Message::Close(Some(frame)))).await;
    state(&ctx).handles.borrow_mut().remove(id);
    cancel.notify_waiters();
    drop(tx);
    use tokio_tungstenite::tungstenite::error::ProtocolError;
    match sent {
        // Already closed by the peer, or the handshake is in flight: the
        // handle is released either way.
        Ok(Ok(()))
        | Ok(Err(WsError::ConnectionClosed))
        | Ok(Err(WsError::AlreadyClosed))
        | Ok(Err(WsError::Protocol(ProtocolError::SendAfterClosing)))
        | Err(_) => Ok(()),
        Ok(Err(e)) => Err(throw_ws(&ctx, e, "close")),
    }
}
