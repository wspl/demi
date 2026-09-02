//! Streaming HTTP/1.1 client over `hyper`: one connection per request,
//! request bodies from memory or streamed from a file, the response body
//! exposed as a handle that streams to the reader.

use std::io;

use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::{BodyDataStream, BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::http::{HeaderName, HeaderValue, Method, Request, Uri};
use hyper_util::rt::TokioIo;
use rquickjs::{Ctx, Object, Result, Value};
use tokio_util::io::{ReaderStream, StreamReader};

use super::connect::{self, Conn};
use crate::error::{throw_code, throw_io};
use crate::handles::{Resource, Stream};
use crate::state::{state, Activity};

type Body = http_body_util::combinators::BoxBody<Bytes, io::Error>;

fn throw_hyper(ctx: &Ctx<'_>, err: hyper::Error, syscall: &str) -> rquickjs::Error {
    let source = std::error::Error::source(&err).and_then(|s| s.downcast_ref::<io::Error>());
    match source.and_then(|e| e.raw_os_error()) {
        Some(errno) => crate::error::throw_errno(ctx, errno, syscall, None),
        None if err.is_timeout() => throw_code(ctx, "ETIMEDOUT", &err.to_string(), syscall, None),
        None => throw_code(ctx, "EPROTO", &err.to_string(), syscall, None),
    }
}

pub async fn http_request<'js>(ctx: Ctx<'js>, options: Object<'js>) -> Result<Object<'js>> {
    let _activity = Activity::begin(&ctx);
    let method: String = options.get::<_, Option<String>>("method")?.unwrap_or_else(|| "GET".into());
    let url: String = options.get("url")?;
    let headers: Option<Object> = options.get("headers")?;
    let body: Value = options.get("body")?;

    let uri: Uri = url.parse().map_err(|_| throw_code(&ctx, "EINVAL", &format!("invalid URL '{url}'"), "request", None))?;
    let secure = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => false,
        _ => return Err(throw_code(&ctx, "EINVAL", "URL must be http or https", "request", None)),
    };
    let host = uri.host().ok_or_else(|| throw_code(&ctx, "EINVAL", "URL has no host", "request", None))?.to_string();
    let method = Method::from_bytes(method.as_bytes()).map_err(|e| throw_code(&ctx, "EINVAL", &e.to_string(), "request", None))?;

    // The body is prepared before connecting so a bad file fails first.
    let (body, length): (Body, Option<u64>) = if body.is_undefined() || body.is_null() {
        (Full::new(Bytes::new()).map_err(|never| match never {}).boxed(), Some(0))
    } else if let Some(bytes) = body.as_object().and_then(|o| o.as_typed_array::<u8>()) {
        let data = Bytes::copy_from_slice(bytes.as_bytes().unwrap_or(&[]));
        let len = data.len() as u64;
        (Full::new(data).map_err(|never| match never {}).boxed(), Some(len))
    } else if let Some(path) = body.as_object().and_then(|o| o.get::<_, Option<String>>("file").ok().flatten()) {
        let file = tokio::fs::File::open(&path).await.map_err(|e| throw_io(&ctx, e, "open", Some(&path)))?;
        let len = file.metadata().await.map_err(|e| throw_io(&ctx, e, "fstat", Some(&path)))?.len();
        let stream = ReaderStream::with_capacity(file, 256 * 1024).map_ok(Frame::data);
        (StreamBody::new(stream).boxed(), Some(len))
    } else {
        return Err(throw_code(&ctx, "EINVAL", "body must be a Uint8Array or { file }", "request", None));
    };

    let connected = connect::open(&uri, secure).await.map_err(|e| throw_io(&ctx, e, "connect", None))?;
    let mut request_uri = uri.clone();
    let mut proxy_auth = None;
    if let Some(intercept) = &connected.via_proxy {
        proxy_auth = intercept.basic_auth().cloned();
    } else {
        // Origin form for a direct connection or through a tunnel.
        let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        request_uri = path.parse().map_err(|_| throw_code(&ctx, "EINVAL", "invalid path", "request", None))?;
    }
    let stream: Conn = if secure {
        connect::tls_wrap(connected.stream, &host).await.map_err(|e| throw_io(&ctx, e, "connect", None))?
    } else {
        connected.stream
    };

    let mut builder = Request::builder().method(method).uri(request_uri);
    let host_header = match uri.port_u16() {
        Some(p) => format!("{host}:{p}"),
        None => host.clone(),
    };
    builder = builder.header("host", host_header).header("connection", "close");
    if let Some(auth) = proxy_auth {
        builder = builder.header("proxy-authorization", auth);
    }
    if let Some(len) = length {
        builder = builder.header("content-length", len);
    }
    if let Some(headers) = headers {
        for r in headers.props::<String, String>() {
            let (k, v) = r?;
            let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| throw_code(&ctx, "EINVAL", &e.to_string(), "request", None))?;
            let value = HeaderValue::from_str(&v).map_err(|e| throw_code(&ctx, "EINVAL", &e.to_string(), "request", None))?;
            builder = builder.header(name, value);
        }
    }
    let request = builder.body(body).map_err(|e| throw_code(&ctx, "EINVAL", &e.to_string(), "request", None))?;

    let (mut sender, connection) = hyper::client::conn::http1::handshake::<_, Body>(TokioIo::new(stream))
        .await
        .map_err(|e| throw_hyper(&ctx, e, "request"))?;
    // The connection task lives as long as the response body is read.
    ctx.spawn(async move {
        let _ = connection.await;
    });
    let response = sender.send_request(request).await.map_err(|e| throw_hyper(&ctx, e, "request"))?;

    let result = Object::new(ctx.clone())?;
    result.set("status", response.status().as_u16())?;
    let headers_obj = Object::new(ctx.clone())?;
    for name in response.headers().keys() {
        let values: Vec<String> = response
            .headers()
            .get_all(name)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        headers_obj.set(name.as_str(), values.join(", "))?;
    }
    result.set("headers", headers_obj)?;
    let body_stream = BodyDataStream::new(response.into_body()).map_err(io::Error::other);
    let reader = StreamReader::new(body_stream);
    let fd = state(&ctx).handles.borrow_mut().insert(Resource::Stream(Stream::new(Some(Box::new(reader)), None)));
    result.set("body", fd)?;
    Ok(result)
}
