//! A page's synchronization channel in the user's shard (`backend.md`
//! § Page synchronization): registered for changes before it reads the
//! product state, it sends that state, then waits for marks and sends each
//! marked part as it is when the channel takes it, and a heartbeat once its
//! page socket has been silent for the heartbeat interval. It ends with its
//! session, at shutdown, when the page sends anything, and when a part cannot
//! be read.

use std::convert::Infallible;
use std::rc::Rc;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use demi_backend_database::accounts::TokenHash;
use demi_backend_page_sync::{Part, Registration};
use demi_plugin_interface::Topic;
use demi_shared_types::Timestamp;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::state::SyncEvent;
use futures_util::StreamExt as _;
use futures_util::stream::SplitStream;

use crate::shard::{PageGone, PageSocket, Shard};
use demi_backend_expose::relay::first_expiry;

/// The close code of a channel whose session ended.
const SESSION_ENDED: u16 = 4002;

/// The session a channel opened with: its token's hash, its user, and when
/// it expires unless a request renews it.
pub struct ChannelSession {
    pub token: TokenHash,
    pub user: UserDto,
    pub expires_at: Timestamp,
}

/// How a channel ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The backend shuts down.
    BackendClosing,
    /// The page sent a message.
    UnexpectedMessage,
    /// A part could not be read.
    InternalError,
    /// The session was signed out, or expired.
    SessionEnded,
    /// The page closed the socket, or it broke.
    PageGone,
}

impl End {
    /// The close frame the backend sends for it; none when the page is gone.
    fn close(self) -> Option<CloseFrame> {
        let (code, reason) = match self {
            Self::BackendClosing => (close_code::AWAY, "backend_closing"),
            Self::UnexpectedMessage => (close_code::POLICY, "unexpected_message"),
            Self::InternalError => (close_code::ERROR, "internal_error"),
            Self::SessionEnded => (SESSION_ENDED, "session_ended"),
            Self::PageGone => return None,
        };
        Some(CloseFrame {
            code,
            reason: Utf8Bytes::from_static(reason),
        })
    }
}

/// What woke a channel that waits.
enum Woke {
    Marked,
    Heartbeat,
    /// The earliest of the user's exposes expired.
    ExposeExpired,
}

impl Shard {
    /// Serves a page's synchronization channel until its session ends, the
    /// page leaves or the shard closes.
    pub async fn serve_sync_channel(self: Rc<Self>, socket: WebSocket, session: ChannelSession) {
        // Counted before any wait, so the shard's close waits for this
        // channel; one adopted as the close begins ends at once.
        let _channel = self.sync_channels().token();
        let (to_page, mut from_page) = socket.split();
        let mut page = PageSocket::new(to_page, self.services().pages);
        let end = if self.is_closing() {
            End::BackendClosing
        } else {
            let mut channel = Channel {
                registration: self
                    .services()
                    .sync
                    .register(self.user(), session.token.clone()),
                shard: &self,
                page: &mut page,
                session,
                expiry: None,
            };
            tokio::select! {
                biased;
                () = self.closed() => End::BackendClosing,
                end = page_ends(&mut from_page) => end,
                end = channel.serve() => match end {
                    Ok(never) => match never {},
                    Err(end) => end,
                },
            }
        };
        if let Some(frame) = end.close() {
            page.close(frame).await;
        }
    }
}

/// A channel being served.
struct Channel<'a> {
    /// Held while the channel lives: the change marks from its
    /// registration on reach it.
    registration: Registration,
    shard: &'a Shard,
    page: &'a mut PageSocket,
    session: ChannelSession,
    /// The earliest expiry of the user's exposes when the page was last
    /// sent a state that follows them; none while no plugin follows them.
    expiry: Option<Timestamp>,
}

impl Channel<'_> {
    /// Sends the product state, then every part that changes, until the
    /// channel ends.
    async fn serve(&mut self) -> Result<Infallible, End> {
        let state = self
            .shard
            .product_state(self.session.user.clone())
            .await
            .map_err(|error| {
                tracing::error!(
                    error = &error as &dyn std::error::Error,
                    "a page's product state could not be read"
                );
                End::InternalError
            })?;
        #[cfg(feature = "testing")]
        self.shard
            .services()
            .syncs
            .pass(super::SyncStep::Snapshot)
            .await;
        self.send(&SyncEvent::Snapshot {
            state: Box::new(state),
        })
        .await?;
        self.read_expiry().await?;
        loop {
            let woke = {
                let clock = &*self.shard.services().clock;
                tokio::select! {
                    () = self.registration.marked() => Woke::Marked,
                    () = self.page.silent() => Woke::Heartbeat,
                    () = first_expiry(clock, self.expiry) => Woke::ExposeExpired,
                }
            };
            self.check_session().await?;
            match woke {
                Woke::Marked => {
                    #[cfg(feature = "testing")]
                    self.shard
                        .services()
                        .syncs
                        .pass(super::SyncStep::Changes)
                        .await;
                    let marked = self.registration.take();
                    if marked.session_ended {
                        return Err(End::SessionEnded);
                    }
                    for part in &marked.parts {
                        self.send_part(part).await?;
                    }
                }
                Woke::Heartbeat => self.send(&SyncEvent::Heartbeat).await?,
                Woke::ExposeExpired => {
                    for part in self.expose_followers() {
                        self.send_part(&part).await?;
                    }
                }
            }
        }
    }

    /// Ends the channel once its session expired. Past the expiry it knew,
    /// it looks at the session again, since a request may have renewed it.
    async fn check_session(&mut self) -> Result<(), End> {
        let services = self.shard.services();
        if services.clock.now() < self.session.expires_at {
            return Ok(());
        }
        match services.sessions.check(&self.session.token).await {
            Ok(Some(session)) => {
                self.session.expires_at = session.expires_at;
                Ok(())
            }
            Ok(None) => Err(End::SessionEnded),
            Err(error) => {
                tracing::error!(
                    error = &error as &dyn std::error::Error,
                    "a page's session could not be read"
                );
                Err(End::InternalError)
            }
        }
    }

    /// Reads `part` now and sends it.
    async fn send_part(&mut self, part: &Part) -> Result<(), End> {
        let read = self
            .shard
            .read_part(part, &self.session.user)
            .await
            .map_err(|error| {
                tracing::error!(
                    ?part,
                    error = &error as &dyn std::error::Error,
                    "a part of a page's state could not be read"
                );
                End::InternalError
            })?;
        let Some(event) = read else {
            return Ok(());
        };
        if let SyncEvent::User { user } = &event {
            self.session.user = user.clone();
        }
        self.send(&event).await?;
        if self.expose_followers().contains(part) {
            self.read_expiry().await?;
        }
        Ok(())
    }

    /// The parts of the plugins whose state follows the user's exposes.
    fn expose_followers(&self) -> Vec<Part> {
        self.shard
            .services()
            .plugins
            .followers(Topic::Exposes)
            .map(|plugin| Part::Plugin(plugin.as_str().to_owned()))
            .collect()
    }

    /// Reads when the earliest of the user's exposes expires, so the
    /// states that follow them are sent again then.
    async fn read_expiry(&mut self) -> Result<(), End> {
        if self.expose_followers().is_empty() {
            return Ok(());
        }
        let exposes = self
            .shard
            .expose_shard()
            .list_exposes()
            .await
            .map_err(|error| {
                tracing::error!(
                    error = &error as &dyn std::error::Error,
                    "a page's exposes could not be read"
                );
                End::InternalError
            })?;
        self.expiry = exposes.iter().map(|expose| expose.record.expires_at).min();
        Ok(())
    }

    async fn send(&mut self, event: &SyncEvent) -> Result<(), End> {
        // The messages' types serialize their fields as JSON strings,
        // numbers, arrays and objects with string keys, which serde_json
        // never refuses.
        let text =
            serde_json::to_string(event).expect("a synchronization message serializes to JSON");
        self.page.send(text).await.map_err(|PageGone| End::PageGone)
    }
}

/// Reads the page's side of the socket until it ends: the page sends
/// nothing, so any message ends the channel.
async fn page_ends(from_page: &mut SplitStream<WebSocket>) -> End {
    loop {
        match from_page.next().await {
            Some(Ok(Message::Text(_) | Message::Binary(_))) => return End::UnexpectedMessage,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Close(_)) | Err(_)) | None => return End::PageGone,
        }
    }
}
