//! A page's synchronization channel in the user's shard (`backend.md`
//! § Page synchronization): registered for changes before it reads the
//! product state, it sends that state, then waits for marks and sends each
//! marked part as it is when the channel takes it, and a heartbeat once its
//! page socket has been silent for the heartbeat interval. It keeps what the
//! page says it shows, which holds back the session's notifications of that
//! conversation while the channel lives. It ends with its session, at
//! shutdown, when the page sends anything else, and when a part cannot be
//! read.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::rc::Rc;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use demi_backend_database::accounts::TokenHash;
use demi_backend_page_sync::{Part, Registration};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::sessions::SessionId;
use demi_web_api_protocol::state::{PageShowing, SyncEvent};
use futures_util::StreamExt as _;

use crate::conversation::notifications::PageShows;
use crate::shard::page_socket::{PageSocketOrigin, open_page_socket};
use crate::shard::{PageEnd, PageGone, PageSocket, PageStream, Shard};

/// The close code of a channel whose session ended.
const SESSION_ENDED: u16 = 4002;

/// The session a page's channel or conversation socket opened with: its
/// token's hash, its user, and when it expires unless a request renews it.
pub struct ChannelSession {
    pub id: SessionId,
    pub token: TokenHash,
    pub user: UserDto,
    pub expires_at: Timestamp,
}

/// How a channel ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The backend shuts down.
    BackendClosing,
    /// The page sent a message that is not what it shows.
    UnexpectedMessage,
    /// A part could not be read.
    InternalError,
    /// The session was signed out, or expired.
    SessionEnded,
    /// The page closed the socket, or it broke.
    PageGone,
}

impl End {
    /// How the socket ends: with the close frame the backend sends, or as
    /// the page left it.
    fn page_end(self) -> PageEnd {
        let frame = |code, reason| CloseFrame {
            code,
            reason: Utf8Bytes::from_static(reason),
        };
        match self {
            Self::BackendClosing => PageEnd::Close(frame(close_code::AWAY, "backend_closing")),
            Self::UnexpectedMessage => {
                PageEnd::Close(frame(close_code::POLICY, "unexpected_message"))
            }
            Self::InternalError => PageEnd::Fail(frame(close_code::ERROR, "internal_error")),
            Self::SessionEnded => PageEnd::Close(frame(SESSION_ENDED, "session_ended")),
            Self::PageGone => PageEnd::Gone,
        }
    }
}

/// What woke a channel that waits.
enum Woke {
    Marked,
    Heartbeat,
}

impl Shard {
    /// Serves a page's synchronization channel, which `origin` opened, until
    /// its session ends, the page leaves or the shard closes.
    pub async fn serve_sync_channel(
        self: Rc<Self>,
        socket: WebSocket,
        session: ChannelSession,
        origin: PageSocketOrigin,
    ) {
        // Counted before any wait, so the shard's close waits for this
        // channel; one adopted as the close begins ends at once.
        let _channel = self.sync_channels().token();
        let (mut page, mut from_page) = open_page_socket(socket, self.services().pages, origin);
        let end = if self.is_closing() {
            End::BackendClosing
        } else {
            // What the page shows holds back the session's notifications
            // of it until the channel ends.
            let shows = self.notifications().page(session.id.clone());
            let mut channel = Channel {
                registration: self
                    .services()
                    .sync
                    .register(self.user(), session.token.clone()),
                shard: &self,
                page: &mut page,
                session,
            };
            tokio::select! {
                biased;
                () = self.closed() => End::BackendClosing,
                end = page_messages(&self, &mut from_page, &shows) => end,
                end = channel.serve() => match end {
                    Ok(never) => match never {},
                    Err(end) => end,
                },
            }
        };
        page.end(from_page, end.page_end()).await;
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
}

impl Channel<'_> {
    /// Sends the product state, then every part that changes, until the
    /// channel ends.
    async fn serve(&mut self) -> Result<Infallible, End> {
        // An account still to choose its password sees the account alone, until
        // it is chosen, on this page or another (`web-api-settings.md` § Account API).
        let mut choosing = self.session.user.chooses_password;
        if choosing {
            let user = self.session.user.clone();
            self.send(&SyncEvent::User { user }).await?;
        } else {
            self.send_snapshot().await?;
        }
        loop {
            let woke = tokio::select! {
                () = self.registration.marked() => Woke::Marked,
                () = self.page.silent() => Woke::Heartbeat,
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
                    // The parts go in their order, each read only after the
                    // change that marked it (`web-api-sync.md` § Page
                    // synchronization): a part marked while another was read
                    // goes first when it comes before it, and the part read
                    // is read again after it, since the read may show a
                    // later change than what the page has of the part
                    // before. A login's completion, read after the provider
                    // publication that marked the providers first, thereby
                    // never reaches the page before its account.
                    let mut pending = BTreeSet::new();
                    loop {
                        let marked = self.registration.take();
                        if marked.session_ended {
                            return Err(End::SessionEnded);
                        }
                        pending.extend(marked.parts);
                        let Some(part) = pending.pop_first() else {
                            break;
                        };
                        if choosing && part != Part::User {
                            continue;
                        }
                        let Some(event) = self.read_part(&part).await? else {
                            continue;
                        };
                        let marked = self.registration.take();
                        if marked.session_ended {
                            return Err(End::SessionEnded);
                        }
                        pending.extend(marked.parts);
                        if pending.first().is_some_and(|first| *first < part) {
                            pending.insert(part);
                            continue;
                        }
                        self.send_read(event).await?;
                        if choosing && !self.session.user.chooses_password {
                            // The choice landed: the rest of the state follows.
                            choosing = false;
                            self.send_snapshot().await?;
                        }
                    }
                }
                Woke::Heartbeat => self.send(&SyncEvent::Heartbeat).await?,
            }
        }
    }

    /// Reads the whole product state now and sends it.
    async fn send_snapshot(&mut self) -> Result<(), End> {
        let state = self
            .shard
            .product_state(self.session.user.clone(), &self.session.token)
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
        .await
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
        match self.read_part(part).await? {
            Some(event) => self.send_read(event).await,
            None => Ok(()),
        }
    }

    /// Reads `part` now, as the message that carries it; none when the
    /// page is sent nothing of it.
    async fn read_part(&mut self, part: &Part) -> Result<Option<SyncEvent>, End> {
        self.shard
            .read_part(part, &self.session.user, &self.session.token)
            .await
            .map_err(|error| {
                tracing::error!(
                    ?part,
                    error = &error as &dyn std::error::Error,
                    "a part of a page's state could not be read"
                );
                End::InternalError
            })
    }

    /// Sends `event`, a part just read.
    async fn send_read(&mut self, event: SyncEvent) -> Result<(), End> {
        let mut promoted = false;
        if let SyncEvent::User { user } = &event {
            // An account made an admin starts to see the accounts.
            promoted = self.session.user.role == Role::User && user.role != Role::User;
            self.session.user = user.clone();
        }
        self.send(&event).await?;
        if promoted {
            Box::pin(self.send_part(&Part::Users)).await?;
        }
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

/// Reads the page's side of the socket until it ends: the page sends only
/// what it shows, which `shows` keeps, and any other message ends the
/// channel.
#[cfg_attr(not(feature = "testing"), expect(unused_variables, reason = "the shard holds a test's channels"))]
async fn page_messages(shard: &Shard, from_page: &mut PageStream, shows: &PageShows) -> End {
    loop {
        match from_page.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<PageShowing>(&text) {
                Ok(PageShowing { showing }) => {
                    shows.set(showing);
                    #[cfg(feature = "testing")]
                    shard.services().syncs.pass(super::SyncStep::Showing).await;
                }
                Err(_) => return End::UnexpectedMessage,
            },
            Some(Ok(Message::Binary(_))) => return End::UnexpectedMessage,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Close(_)) | Err(_)) | None => return End::PageGone,
        }
    }
}
