use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream::BoxStream};
use std::{io, sync::Arc};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct PipeClient {
    http: reqwest::Client,
    origin: reqwest::Url,
    token: Arc<RwLock<Option<String>>>,
}

impl PipeClient {
    pub fn new(backend: &str, token: Arc<RwLock<Option<String>>>) -> io::Result<Self> {
        let mut origin = crate::state::backend_url(backend)?;
        match origin.scheme() {
            "ws" => origin
                .set_scheme("http")
                .map_err(|_| io::Error::other("invalid HTTP origin"))?,
            "wss" => origin
                .set_scheme("https")
                .map_err(|_| io::Error::other("invalid HTTPS origin"))?,
            _ => {}
        }
        origin.set_path("/");
        origin.set_query(None);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            http,
            origin,
            token,
        })
    }

    pub async fn get(
        &self,
        path: &str,
        cancel: CancellationToken,
    ) -> io::Result<BoxStream<'static, io::Result<Bytes>>> {
        let request = self.request(reqwest::Method::GET, path).await?;
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            response = async { expect_ok(request.send().await.map_err(io::Error::other)?).await } => response?,
        };
        let stream = response
            .bytes_stream()
            .map(|result| result.map_err(io::Error::other));
        Ok(
            futures_util::stream::unfold(Some((Box::pin(stream), cancel)), |state| async move {
                let (mut stream, cancel) = state?;
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => Some((Err(cancelled()), None)),
                    chunk = stream.next() => chunk.map(|chunk| (chunk, Some((stream, cancel)))),
                }
            })
            .boxed(),
        )
    }

    pub async fn put<S>(&self, path: &str, body: S, cancel: &CancellationToken) -> io::Result<()>
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        let request = self
            .request(reqwest::Method::PUT, path)
            .await?
            .body(reqwest::Body::wrap_stream(body));
        tokio::select! {
            _ = cancel.cancelled() => Err(cancelled()),
            result = async {
                let mut response = expect_ok(request.send().await.map_err(io::Error::other)?).await?;
                // Consume the confirmation with a bound; a pipe upload has no payload response.
                let mut size = 0usize;
                while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
                    size += chunk.len();
                    if size > 16 * 1024 { return Err(io::Error::new(io::ErrorKind::InvalidData, "oversized pipe confirmation")); }
                }
                Ok(())
            } => result,
        }
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> io::Result<reqwest::RequestBuilder> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pipe URL must be origin-relative",
            ));
        }
        let url = self.origin.join(path).map_err(io::Error::other)?;
        if url.origin() != self.origin.origin() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pipe URL changed backend origin",
            ));
        }
        let token = self.token.read().await;
        let token = token.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runner has no device token",
            )
        })?;
        Ok(self.http.request(method, url).bearer_auth(token))
    }
}

async fn expect_ok(mut response: reqwest::Response) -> io::Result<reqwest::Response> {
    if response.status() == reqwest::StatusCode::OK {
        return Ok(response);
    }
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
        let count = (16 * 1024 - body.len()).min(chunk.len());
        body.extend_from_slice(&chunk[..count]);
        if body.len() == 16 * 1024 {
            break;
        }
    }
    Err(io::Error::other(format!(
        "pipe refused ({status}): {}",
        String::from_utf8_lossy(&body)
    )))
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "pipe cancelled")
}
