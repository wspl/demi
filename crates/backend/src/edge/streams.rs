//! User streams at the edge (`web-api.md` § User streams): the upgrade of a
//! declared stream, and the relay of its bytes between the page's socket
//! and the Host's pipes, taken from the Host only as fast as the page takes
//! them. `POST /activity` records one user operation on the conversation's
//! Host.

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use demi_web_api::error::ErrorCode;
use futures_util::{SinkExt as _, StreamExt as _};
use url::Url;

use super::AppState;
use super::conversations::owned;
use super::error::ApiError;
use super::gate::AuthUser;
use crate::conversation::stream::UserStream;

/// `WS /conversations/:id/streams/:name`: everything that can refuse the
/// stream answers before the upgrade, the Host's opening of it included.
pub(super) async fn open(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    // The stream operates a browser signed in to the user's sites.
    if !from_product(&headers, state.site.public_url.as_ref()) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            ErrorCode::ForbiddenOrigin,
            "A user stream opens only from the product",
        ));
    }
    let Some(binding) = state.services.user_streams.get(&name).cloned() else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, ErrorCode::UnknownStream, "No stream has that name"));
    };
    let record = owned(&state.services, &user.id, &id).await?;
    let upgrade = upgrade.map_err(|_| {
        ApiError::new(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::UpgradeRequired,
            "A user stream is a WebSocket",
        )
    })?;
    let stream = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move { shard.open_user_stream(&record.id, &binding, &cancel).await })
        .await??;
    // An upgrade that never completes drops the stream, which ends it.
    Ok(upgrade.on_upgrade(move |socket| relay(socket, stream)))
}

/// `POST /conversations/:id/activity`: one user operation, admitted on the
/// conversation's Host and ended at once, which restarts its idle window
/// (`resource-lifecycle.md` § Activity).
pub(super) async fn activity(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let record = owned(&state.services, &user.id, &id).await?;
    state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move { shard.with_host(&record.id, None, &cancel, async |_| ()).await })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether the upgrade comes from a page of the product: the public URL's
/// origin, or the host the request was sent to.
fn from_product(headers: &HeaderMap, public_url: Option<&Url>) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|origin| origin.to_str().ok()) else {
        return false;
    };
    if public_url.is_some_and(|url| url.origin().ascii_serialization() == origin) {
        return true;
    }
    let Ok(origin) = Url::parse(origin) else {
        return false;
    };
    let Some(host) = origin.host_str() else {
        return false;
    };
    let authority = match origin.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    headers.get(HOST).is_some_and(|sent_to| sent_to.as_bytes() == authority.as_bytes())
}

/// How a stream ended, which the page's socket closes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The invocation completed.
    Completed,
    /// The page sent a text message.
    BinaryOnly,
    /// The Host became unreachable, or a pipe to it failed.
    HostUnreachable,
    /// A transition ended the stream.
    Changed,
    /// The page closed its socket, or it failed.
    PageClosed,
}

impl End {
    /// The close code and reason, when the page is still there to hear
    /// them.
    fn close(self) -> Option<(u16, &'static str)> {
        match self {
            Self::Completed => Some((1000, "completed")),
            Self::BinaryOnly => Some((1003, "binary_only")),
            Self::HostUnreachable => Some((1011, "host_unreachable")),
            Self::Changed => Some((4000, "conversation_changed")),
            Self::PageClosed => None,
        }
    }
}

/// Relays an open stream: the page's binary messages go to the Host in
/// order, each after the Host took the one before, and the Host's bytes go
/// to the page, each after the page's socket took the one before. Message
/// boundaries mean nothing. Whichever side ends first ends the stream;
/// dropping the lease then ends it in the shard.
async fn relay(socket: WebSocket, stream: UserStream) {
    let UserStream {
        mut to_host,
        mut from_host,
        lease,
    } = stream;
    let (mut to_page, mut from_page) = socket.split();
    let end = {
        let deliver = async {
            loop {
                match from_host.next().await {
                    Some(Ok(bytes)) => {
                        if to_page.send(Message::Binary(bytes)).await.is_err() {
                            return End::PageClosed;
                        }
                    }
                    Some(Err(_)) => return End::HostUnreachable,
                    None => return End::Completed,
                }
            }
        };
        let forward = async {
            loop {
                match from_page.next().await {
                    Some(Ok(Message::Binary(bytes))) => {
                        if to_host.write(bytes).await.is_err() {
                            return End::HostUnreachable;
                        }
                    }
                    Some(Ok(Message::Text(_))) => return End::BinaryOnly,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return End::PageClosed,
                }
            }
        };
        // A transition fails the stream's pipes once it ended the lease, so
        // the lease's end is seen first.
        tokio::select! {
            biased;
            () = lease.ended() => End::Changed,
            end = deliver => end,
            end = forward => end,
        }
    };
    if let Some((code, reason)) = end.close() {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        // A page that went meanwhile hears nothing, which is what closing
        // tells it.
        let _ = to_page.send(Message::Close(Some(frame))).await;
    }
    drop(lease);
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn a_stream_opens_from_the_product_s_public_origin_or_the_host_asked() {
        let headers = |origin: Option<&str>, host: &str| {
            let mut headers = HeaderMap::new();
            if let Some(origin) = origin {
                headers.insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
            }
            headers.insert(HOST, HeaderValue::from_str(host).unwrap());
            headers
        };
        let public: Url = "https://demi.example.com/".parse().unwrap();
        assert!(from_product(&headers(Some("https://demi.example.com"), "10.0.0.2:3271"), Some(&public)));
        assert!(from_product(&headers(Some("http://127.0.0.1:3271"), "127.0.0.1:3271"), None));
        assert!(from_product(&headers(Some("https://demi.example.com"), "demi.example.com"), None));
        for (origin, host) in [
            (Some("https://elsewhere.example"), "127.0.0.1:3271"),
            (Some("http://127.0.0.1:9999"), "127.0.0.1:3271"),
            (Some("null"), "127.0.0.1:3271"),
            (None, "127.0.0.1:3271"),
        ] {
            assert!(!from_product(&headers(origin, host), Some(&public)), "{origin:?}");
        }
    }
}
