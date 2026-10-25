//! A page's synchronization channel in the user's shard.
//!
//! The channel:
//!
//! 1. Registers for changes when it opens, and sends nothing: the page read
//!    what it shows by its routes, and its first message, `open`, names each
//!    value it follows with the version it holds.
//! 2. Follows a value: the product state from a version, whose log of
//!    changes (`backend-page-sync`) says which parts changed since, or a
//!    settings list from the entity tag the page holds, which the channel
//!    compares with the list as it reads it. A version the log no longer
//!    reaches is answered `stale`.
//! 3. Waits for marks, and sends each marked part of a followed value as it
//!    is when the channel takes it, then `synced` with the version the state
//!    is current as of.
//! 4. Sends a heartbeat once its page socket has been silent for the heartbeat
//!    interval.
//!
//! It keeps what the page says it shows, which holds back the session's
//! notifications of that conversation while the channel lives.
//!
//! An account still to choose its password is sent its account alone, when it
//! opens and when the choice lands, since `GET /state` is refused until then;
//! the page reads the state and follows it once the choice landed.
//!
//! A page of the build that took a snapshot opens the channel without `build`:
//! it is sent one `snapshot` message that holds the web build alone, which
//! that build reloads on, and the channel closes.
//!
//! It ends with its session, at shutdown, when the page sends anything else, and
//! when a part cannot be read.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::rc::Rc;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use demi_backend_database::accounts::TokenHash;
use demi_backend_page_sync::{Part, Registration, StateVersion};
use demi_conversation_socket_protocol::{Held, Version};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::sessions::SessionId;
use demi_web_api_protocol::state::{PageMessage, SyncEvent, SyncFollow, SyncFollowed};
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
    /// The message the page of an earlier build waits for was sent.
    Done,
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
            Self::Done => PageEnd::Close(frame(close_code::NORMAL, "superseded")),
        }
    }
}

/// What woke a channel that waits.
enum Woke {
    Marked,
    Stale,
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
        legacy: bool,
    ) {
        // Counted before any wait, so the shard's close waits for this
        // channel; one adopted as the close begins ends at once.
        let _channel = self.sync_channels().token();
        let (mut page, mut from_page) = open_page_socket(socket, self.services().pages, origin);
        let end = if self.is_closing() {
            End::BackendClosing
        } else if legacy {
            self.send_legacy_snapshot(&mut page).await
        } else {
            // What the page shows holds back the session's notifications
            // of it until the channel ends.
            let shows = self.notifications().page(session.id.clone());
            let registration = self
                .services()
                .sync
                .register(self.user(), session.token.clone());
            let follows = Follows::default();
            let mut channel = Channel {
                registration: &registration,
                follows: &follows,
                shard: &self,
                page: &mut page,
                session,
            };
            tokio::select! {
                biased;
                () = self.closed() => End::BackendClosing,
                end = page_messages(&mut from_page, &shows, &registration, &follows) => end,
                end = channel.serve() => match end {
                    Ok(never) => match never {},
                    Err(end) => end,
                },
            }
        };
        page.end(from_page, end.page_end()).await;
    }

    /// Sends the one message a page of the earlier build understands, the
    /// snapshot of the state with the web build alone, which makes that page
    /// reload when it is not the build the backend serves.
    async fn send_legacy_snapshot(&self, page: &mut PageSocket) -> End {
        let text = serde_json::json!({
            "type": "snapshot",
            "state": { "webBuild": self.services().web_build },
        })
        .to_string();
        match page.send(text).await {
            Ok(()) => End::Done,
            Err(PageGone) => End::PageGone,
        }
    }
}

/// What a channel's page follows, as its messages registered it.
#[derive(Default)]
struct Follows {
    /// The followed lists, each with the entity tag the page holds of it.
    lists: RefCell<HashMap<SyncFollowed, String>>,
    /// The epoch of the state's version when it is followed.
    state: RefCell<Option<String>>,
    /// Whether `open` came: it comes once.
    opened: std::cell::Cell<bool>,
    /// The follows that ended as stale, for the channel to tell the page.
    stale: RefCell<Vec<SyncFollowed>>,
    wake: tokio::sync::Notify,
}

impl Follows {
    fn follows_state(&self) -> bool {
        self.state.borrow().is_some()
    }

    /// Whether any value `part` is a part of is followed.
    fn wants(&self, part: &Part) -> bool {
        match list_of(part) {
            Some(list) => self.lists.borrow().contains_key(&list),
            None => self.follows_state(),
        }
    }

    /// Registers the follow `follow`, and compares what the page holds with
    /// the current value: the parts that changed since go to the channel, or
    /// the follow is stale.
    fn follow(&self, registration: &Registration, follow: SyncFollow) -> Result<(), End> {
        let SyncFollow { value, holds } = follow;
        match (value, holds) {
            (SyncFollowed::State, Held::Version { version: Version { epoch, revision }, .. }) => {
                let held = StateVersion { epoch: epoch.clone(), revision };
                *self.state.borrow_mut() = Some(epoch);
                let stale = registration.follow_since(&held).is_err();
                tracing::info!(
                    event = "follow.answered",
                    value = "state",
                    held_revision = revision,
                    sent = if stale { "stale" } else { "changes" },
                    "a follow of the product state was answered"
                );
                if stale {
                    self.state.borrow_mut().take();
                    self.end_stale(SyncFollowed::State);
                }
            }
            (SyncFollowed::State, Held::Tag { .. }) => return Err(End::UnexpectedMessage),
            (list, Held::Tag { tag }) => {
                tracing::info!(
                    event = "follow.answered",
                    value = list_name(list),
                    held_tag = !tag.is_empty(),
                    "a follow of a list was registered"
                );
                self.lists.borrow_mut().insert(list, tag);
                if let Some(part) = part_of(list) {
                    registration.mark_part(part);
                }
            }
            (_, Held::Version { .. }) => return Err(End::UnexpectedMessage),
        }
        Ok(())
    }

    fn unfollow(&self, value: SyncFollowed) {
        match value {
            SyncFollowed::State => {
                self.state.borrow_mut().take();
            }
            list => {
                self.lists.borrow_mut().remove(&list);
            }
        }
    }

    fn end_stale(&self, value: SyncFollowed) {
        self.stale.borrow_mut().push(value);
        self.wake.notify_one();
    }
}

/// The settings list a part is the change of, if it is one.
fn list_of(part: &Part) -> Option<SyncFollowed> {
    Some(match part {
        Part::Instructions => SyncFollowed::Instructions,
        Part::Subagents => SyncFollowed::Subagents,
        Part::Credentials => SyncFollowed::Credentials,
        Part::PortForwards => SyncFollowed::PortForwards,
        Part::ExposedPorts => SyncFollowed::ExposedPorts,
        Part::Passkeys => SyncFollowed::Passkeys,
        Part::Sessions => SyncFollowed::Sessions,
        Part::Users => SyncFollowed::Users,
        _ => return None,
    })
}

/// The part that reads the settings list `list`.
fn part_of(list: SyncFollowed) -> Option<Part> {
    Some(match list {
        SyncFollowed::State => return None,
        SyncFollowed::Instructions => Part::Instructions,
        SyncFollowed::Subagents => Part::Subagents,
        SyncFollowed::Credentials => Part::Credentials,
        SyncFollowed::PortForwards => Part::PortForwards,
        SyncFollowed::ExposedPorts => Part::ExposedPorts,
        SyncFollowed::Passkeys => Part::Passkeys,
        SyncFollowed::Sessions => Part::Sessions,
        SyncFollowed::Users => Part::Users,
    })
}

/// A channel being served.
struct Channel<'a> {
    /// The channel's place in the registry: the change marks from its
    /// registration on reach it.
    registration: &'a Registration,
    follows: &'a Follows,
    shard: &'a Shard,
    page: &'a mut PageSocket,
    session: ChannelSession,
}

impl Channel<'_> {
    /// Sends every part that changes, until the channel ends.
    async fn serve(&mut self) -> Result<Infallible, End> {
        // An account still to choose its password sees the account alone, until
        // it is chosen, on this page or another.
        let mut choosing = self.session.user.chooses_password;
        if choosing {
            let user = self.session.user.clone();
            self.send(&SyncEvent::User { user }).await?;
        }
        loop {
            let woke = tokio::select! {
                () = self.registration.marked() => Woke::Marked,
                () = self.follows.wake.notified() => Woke::Stale,
                () = self.page.silent() => Woke::Heartbeat,
            };
            self.check_session().await?;
            match woke {
                Woke::Stale => {
                    let stale = std::mem::take(&mut *self.follows.stale.borrow_mut());
                    for value in stale {
                        self.send(&SyncEvent::Stale { value }).await?;
                    }
                }
                Woke::Marked => {
                    // The parts go in their order, each read only after the change
                    // that marked it.
                    //
                    // A part marked while another was read goes first when it
                    // comes before it, and the part read is read again after it,
                    // since the read may show a later change than what the page has
                    // of the part before. A login's completion, read after the
                    // provider publication that marked the providers first, thereby
                    // never reaches the page before its account.
                    let mut pending = BTreeSet::new();
                    let mut current = None;
                    loop {
                        let marked = self.registration.take();
                        if marked.session_ended {
                            return Err(End::SessionEnded);
                        }
                        pending.extend(marked.parts);
                        current = current.max(marked.revision);
                        let Some(part) = pending.pop_first() else {
                            break;
                        };
                        if choosing && part != Part::User {
                            continue;
                        }
                        if !choosing && !self.follows.wants(&part) {
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
                        current = current.max(marked.revision);
                        if pending.first().is_some_and(|first| *first < part) {
                            pending.insert(part);
                            continue;
                        }
                        self.send_read(event, choosing).await?;
                        if choosing && !self.session.user.chooses_password {
                            // The choice landed: the page reads the rest.
                            choosing = false;
                        }
                    }
                    // The parts are sent: the state is current as of the
                    // latest change they stand for.
                    let epoch = self.follows.state.borrow().clone();
                    if let (Some(revision), Some(epoch)) = (current, epoch) {
                        self.send(&SyncEvent::Synced { version: Version { epoch, revision } }).await?;
                    }
                }
                Woke::Heartbeat => self.send(&SyncEvent::Heartbeat).await?,
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
                    event = "page_session.read_failed",
                    user = %self.session.user.id,
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
            Some(event) => self.send_read(event, false).await,
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
                let (name, conversation, plugin) = part_name(part);
                tracing::error!(
                    event = "state_part.read_failed",
                    user = %self.session.user.id,
                    part = name,
                    conversation = conversation.map(tracing::field::display),
                    plugin,
                    error = &error as &dyn std::error::Error,
                    "a part of a page's state could not be read"
                );
                End::InternalError
            })
    }

    /// Sends `event`, a part just read, to the followers of the value it is
    /// of: a list only when what the page holds of it differs.
    async fn send_read(&mut self, event: SyncEvent, choosing: bool) -> Result<(), End> {
        // An account still to choose its password follows nothing, and is sent
        // its account.
        let event = match self.followed(event, choosing) {
            Some(event) => event,
            None => return Ok(()),
        };
        let mut promoted = false;
        if let SyncEvent::User { user } = &event {
            // An account made an admin starts to see the accounts and the
            // server's updates.
            promoted = self.session.user.role == Role::User && user.role != Role::User;
            self.session.user = user.clone();
        }
        self.send(&event).await?;
        if promoted {
            Box::pin(self.send_part(&Part::Users)).await?;
            Box::pin(self.send_part(&Part::ServerUpgrade)).await?;
        }
        Ok(())
    }

    /// The message `event` becomes for what the page follows, none when the
    /// page needs nothing of it. A list the page holds at this tag is not
    /// sent again.
    fn followed(&self, event: SyncEvent, choosing: bool) -> Option<SyncEvent> {
        let (list, tag) = match &event {
            SyncEvent::Instructions { etag, .. } => (SyncFollowed::Instructions, etag),
            SyncEvent::Subagents { etag, .. } => (SyncFollowed::Subagents, etag),
            SyncEvent::Credentials { etag, .. } => (SyncFollowed::Credentials, etag),
            SyncEvent::PortForwards { etag, .. } => (SyncFollowed::PortForwards, etag),
            SyncEvent::ExposedPorts { etag, .. } => (SyncFollowed::ExposedPorts, etag),
            SyncEvent::Passkeys { etag, .. } => (SyncFollowed::Passkeys, etag),
            SyncEvent::Sessions { etag, .. } => (SyncFollowed::Sessions, etag),
            SyncEvent::Users { etag, .. } => (SyncFollowed::Users, etag),
            // A part of the state: followed with the state.
            SyncEvent::User { .. } if choosing => return Some(event),
            _ => return self.follows.follows_state().then_some(event),
        };
        let mut lists = self.follows.lists.borrow_mut();
        let held = lists.get_mut(&list)?;
        // A list whose route sends no tag has none to compare: it is sent.
        if !tag.is_empty() && held == tag {
            return None;
        }
        held.clone_from(tag);
        Some(event)
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

/// Reads the page's side of the socket until it ends: what it follows, which
/// `follows` and the registration keep, and what it shows, which `shows`
/// keeps; any other message ends the channel.
async fn page_messages(
    from_page: &mut PageStream,
    shows: &PageShows,
    registration: &Registration,
    follows: &Follows,
) -> End {
    loop {
        match from_page.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<PageMessage>(&text) {
                Ok(PageMessage::Showing { showing }) => {
                    shows.set(showing);
                }
                Ok(PageMessage::Open { follows: opened }) => {
                    if follows.opened.replace(true) {
                        return End::UnexpectedMessage;
                    }
                    for follow in opened {
                        if let Err(end) = follows.follow(registration, follow) {
                            return end;
                        }
                    }
                }
                Ok(PageMessage::Follow(follow)) => {
                    if let Err(end) = follows.follow(registration, follow) {
                        return end;
                    }
                }
                Ok(PageMessage::Unfollow { value }) => follows.unfollow(value),
                Err(_) => return End::UnexpectedMessage,
            },
            Some(Ok(Message::Binary(_))) => return End::UnexpectedMessage,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Close(_)) | Err(_)) | None => return End::PageGone,
        }
    }
}

/// A followed list as the protocol names it.
fn list_name(list: SyncFollowed) -> String {
    serde_json::to_value(list)
        .ok()
        .and_then(|name| name.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// A part of the product state by name, with the conversation or the plugin
/// it is of.
fn part_name(part: &Part) -> (&'static str, Option<&ConversationId>, Option<&str>) {
    match part {
        Part::Conversation(id) => ("conversation", Some(id), None),
        Part::Metadata(id) => ("metadata", Some(id), None),
        Part::ConversationOrder => ("conversation_order", None, None),
        Part::Preferences => ("preferences", None, None),
        Part::User => ("user", None, None),
        Part::Workspaces => ("workspaces", None, None),
        Part::Devices => ("devices", None, None),
        Part::Providers => ("providers", None, None),
        Part::Logins => ("logins", None, None),
        Part::Cloud => ("cloud", None, None),
        Part::Subagents => ("subagents", None, None),
        Part::Instructions => ("instructions", None, None),
        Part::Credentials => ("credentials", None, None),
        Part::PortForwards => ("port_forwards", None, None),
        Part::ExposedPorts => ("exposed_ports", None, None),
        Part::Passkeys => ("passkeys", None, None),
        Part::Sessions => ("sessions", None, None),
        Part::Users => ("users", None, None),
        Part::ServerUpgrade => ("server_upgrade", None, None),
        Part::Update => ("update", None, None),
        Part::Plugins => ("plugins", None, None),
        Part::Plugin(id) => ("plugin", None, Some(id.as_str())),
        Part::Models => ("models", None, None),
    }
}
