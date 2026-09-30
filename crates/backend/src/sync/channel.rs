//! A page's synchronization channel in the user's shard (`backend.md`
//! § Browser synchronization): registered for changes before it reads the
//! product state, it sends that state, then waits for marks and sends each
//! marked part as it is when the channel takes it, and a heartbeat once its
//! page socket has been silent for the heartbeat interval. It ends with its
//! session, at shutdown, when the page sends anything, and when a part cannot
//! be read.

use std::convert::Infallible;
use std::rc::Rc;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use demi_backend_storage::accounts::TokenHash;
use demi_backend_sync::{Part, Registration};
use demi_core::Timestamp;
use demi_web_api::auth::UserDto;
use demi_web_api::exposes::ExposeDto;
use demi_web_api::state::SyncEvent;
use futures_util::StreamExt as _;
use futures_util::stream::SplitStream;

use crate::expose::first_expiry;
use crate::shard::{PageGone, PageSocket, Shard};

/// The close code of a channel whose session ended.
const SESSION_ENDED: u16 = 4002;

/// The session a channel opened with: its token's hash, its user, and when
/// it expires unless a request renews it.
pub(crate) struct ChannelSession {
    pub(crate) token: TokenHash,
    pub(crate) user: UserDto,
    pub(crate) expires_at: Timestamp,
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
    /// An expose the page shows expired.
    ExposeExpired,
}

impl Shard {
    /// Serves a page's synchronization channel until its session ends, the
    /// page leaves or the shard closes.
    pub(crate) async fn serve_sync_channel(self: Rc<Self>, socket: WebSocket, session: ChannelSession) {
        // Counted before any wait, so the shard's close waits for this
        // channel; one adopted as the close begins ends at once.
        let _channel = self.sync_channels().token();
        let (to_page, mut from_page) = socket.split();
        let mut page = PageSocket::new(to_page, self.services().pages);
        let end = if self.is_closing() {
            End::BackendClosing
        } else {
            let mut channel = Channel {
                registration: self.services().sync.register(self.user(), session.token.clone()),
                shard: &self,
                page: &mut page,
                session,
                exposes: Vec::new(),
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
    /// The exposes the page was last sent, for their expiry.
    exposes: Vec<ExposeDto>,
}

impl Channel<'_> {
    /// Sends the product state, then every part that changes, until the
    /// channel ends.
    async fn serve(&mut self) -> Result<Infallible, End> {
        let state = self.shard.product_state(self.session.user.clone()).await.map_err(|error| {
            tracing::error!(error = &error as &dyn std::error::Error, "a page's product state could not be read");
            End::InternalError
        })?;
        #[cfg(feature = "testing")]
        self.shard.services().syncs.pass(super::SyncStep::Snapshot).await;
        self.exposes.clone_from(&state.exposes);
        self.send(&SyncEvent::Snapshot { state: Box::new(state) }).await?;
        loop {
            let woke = {
                let clock = &*self.shard.services().clock;
                tokio::select! {
                    () = self.registration.marked() => Woke::Marked,
                    () = self.page.silent() => Woke::Heartbeat,
                    () = first_expiry(clock, &self.exposes) => Woke::ExposeExpired,
                }
            };
            self.check_session().await?;
            match woke {
                Woke::Marked => {
                    #[cfg(feature = "testing")]
                    self.shard.services().syncs.pass(super::SyncStep::Changes).await;
                    let marked = self.registration.take();
                    if marked.session_ended {
                        return Err(End::SessionEnded);
                    }
                    for part in &marked.parts {
                        self.send_part(part).await?;
                    }
                }
                Woke::Heartbeat => self.send(&SyncEvent::Heartbeat).await?,
                Woke::ExposeExpired => self.send_part(&Part::Exposes).await?,
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
                tracing::error!(error = &error as &dyn std::error::Error, "a page's session could not be read");
                Err(End::InternalError)
            }
        }
    }

    /// Reads `part` now and sends it.
    async fn send_part(&mut self, part: &Part) -> Result<(), End> {
        let read = self.shard.read_part(part, &self.session.user).await.map_err(|error| {
            tracing::error!(?part, error = &error as &dyn std::error::Error, "a part of a page's state could not be read");
            End::InternalError
        })?;
        let Some(event) = read else {
            return Ok(());
        };
        match &event {
            SyncEvent::User { user } => self.session.user = user.clone(),
            SyncEvent::Exposes { exposes } => self.exposes.clone_from(exposes),
            _ => {}
        }
        self.send(&event).await
    }

    async fn send(&mut self, event: &SyncEvent) -> Result<(), End> {
        // The messages' types serialize their fields as JSON strings,
        // numbers, arrays and objects with string keys, which serde_json
        // never refuses.
        let text = serde_json::to_string(event).expect("a synchronization message serializes to JSON");
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
