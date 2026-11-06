//! A page's synchronization channel in the user's core.
//!
//! The channel:
//!
//! 1. Registers for changes when it opens, and sends nothing: the page read
//!    what it shows by its routes, and its first message, `open`, names each
//!    value it follows with the version it holds.
//! 2. Follows a value: the product state from a version, whose log of
//!    changes (`core-sync`) says which parts changed since, or a
//!    settings list from the entity tag the page holds, which the channel
//!    compares with the list as it reads it. A version the log no longer
//!    reaches is answered `stale`.
//! 3. Waits for marks, and sends each marked part of a followed value as it
//!    is when the channel takes it, then `synced` with the version the state
//!    is current as of.
//!
//! The core reads its own parts; the caller's page reads the mount's, such
//! as a server's account and sessions (`crate::caller::CallerPage`). The
//! page also hears what the page says it shows and when the user last acted
//! on it, while the channel lives: a server holds back its session's
//! notifications of the conversation the page shows.
//!
//! A page that may see one part alone for now, as an account still to choose
//! its password its account, is sent that part alone, when it opens and when
//! it changes, until the page may see the rest.
//!
//! It ends with its lane, whose connection ends with its caller, as the core
//! closes, when the page sends anything else, and when a part cannot be read.
//!
//! # Apart from the conversation lanes
//!
//! The channel stays apart from the conversation lanes:
//!
//! - A conversation's frames are a log the page applies in order, so the lane's
//!   outbox keeps every frame and closes a page that falls too far behind.
//! - The channel sends current values, which it can merge, so it never closes a
//!   slow page.
//!
//! On one shared lane, a conversation that lagged would close the page's
//! channel and every other conversation with it, and a burst of command output
//! would hold back the sidebar's changes behind it.
//!
//! The channel carries changes alone. The page reads the product state by
//! `GET /state`, then opens the channel and follows the state from the
//! version the read carries; it applies what the channel sends from then on.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::rc::Rc;
use std::sync::Arc;

use axum::extract::ws::close_code;
use demi_core_sync::{Part, Registration, StateVersion};
use demi_conversation_lane_protocol::{Held, Version};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::state::{PageMessage, SyncEvent, SyncFollow, SyncFollowed};

use crate::caller::{Caller, CallerPage};
use crate::state::{is_core_part, read_core_part};
use demi_core::{Lane, LaneClose, LaneEnd, LaneGone, LaneMessage, LaneReader, Core};

/// How a channel ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The backend shuts down.
    BackendClosing,
    /// The page sent a message that is not what it shows.
    UnexpectedMessage,
    /// A part could not be read.
    InternalError,
    /// The page closed the lane, or its connection ended.
    PageGone,
}

impl End {
    /// How the lane ends: with the close the backend sends, or as the page
    /// left it.
    fn lane_end(self) -> LaneEnd {
        let close = |code, reason| LaneClose { code, reason };
        match self {
            Self::BackendClosing => LaneEnd::Close(close(close_code::AWAY, "backend_closing")),
            Self::UnexpectedMessage => {
                LaneEnd::Close(close(close_code::POLICY, "unexpected_message"))
            }
            Self::InternalError => LaneEnd::Fail(close(close_code::ERROR, "internal_error")),
            Self::PageGone => LaneEnd::Gone,
        }
    }
}

/// What woke a channel that waits.
enum Woke {
    Marked,
    Stale,
}

/// Serves `caller`'s page's synchronization channel on its lane, on `core`,
/// until the page leaves or the core closes.
pub(crate) async fn serve(core: Rc<Core>, mut page: Lane, mut from_page: LaneReader, caller: Arc<dyn Caller>) {
    // Counted before any wait, so the core's close waits for this channel;
    // one adopted as the close begins ends at once.
    let _channel = core.channels().token();
    let end = if core.is_closing() {
        End::BackendClosing
    } else {
        // What the page shows and when it was used, which the caller's page
        // hears until the channel ends.
        let caller_page = RefCell::new(caller.page(&core));
        let registration = core.services().sync.watch(core.user());
        let follows = Follows::default();
        let mut channel = Channel {
            registration: &registration,
            follows: &follows,
            core: &core,
            page: &mut page,
            caller_page: &caller_page,
        };
        tokio::select! {
            biased;
            () = core.closed() => End::BackendClosing,
            end = page_messages(&mut from_page, &caller_page, &registration, &follows) => end,
            end = channel.serve() => match end {
                Ok(never) => match never {},
                Err(end) => end,
            },
        }
    };
    page.end(from_page, end.lane_end());
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
        Part::Passkeys => SyncFollowed::Passkeys,
        Part::Sessions => SyncFollowed::Sessions,
        Part::Users => SyncFollowed::Users,
        Part::CloudIsolation => SyncFollowed::CloudIsolation,
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
        SyncFollowed::Passkeys => Part::Passkeys,
        SyncFollowed::Sessions => Part::Sessions,
        SyncFollowed::Users => Part::Users,
        SyncFollowed::CloudIsolation => Part::CloudIsolation,
    })
}

/// A channel being served.
struct Channel<'a> {
    /// The channel's place in the registry: the change marks from its
    /// registration on reach it.
    registration: &'a Registration,
    follows: &'a Follows,
    core: &'a Core,
    page: &'a mut Lane,
    caller_page: &'a RefCell<Box<dyn CallerPage>>,
}

impl Channel<'_> {
    /// The one part the page may see for now, if it may see one alone.
    fn only(&self) -> Option<Part> {
        self.caller_page.borrow().only()
    }

    /// Sends every part that changes, until the channel ends.
    async fn serve(&mut self) -> Result<Infallible, End> {
        // A page that may see one part alone, as an account still to choose
        // its password its account, is sent it as the channel opens.
        if let Some(part) = self.only() {
            self.send_part(&part).await?;
        }
        loop {
            let woke = tokio::select! {
                () = self.registration.marked() => Woke::Marked,
                () = self.follows.wake.notified() => Woke::Stale,
            };
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
                        pending.extend(marked.parts);
                        current = current.max(marked.revision);
                        let Some(part) = pending.pop_first() else {
                            break;
                        };
                        let only = self.only();
                        if only.as_ref().is_some_and(|only| *only != part) {
                            continue;
                        }
                        if only.is_none() && !self.follows.wants(&part) {
                            continue;
                        }
                        let Some(event) = self.read_part(&part).await? else {
                            continue;
                        };
                        let marked = self.registration.take();
                        pending.extend(marked.parts);
                        current = current.max(marked.revision);
                        if pending.first().is_some_and(|first| *first < part) {
                            pending.insert(part);
                            continue;
                        }
                        self.send_read(event, only.is_some()).await?;
                    }
                    // The parts are sent: the state is current as of the
                    // latest change they stand for.
                    let epoch = self.follows.state.borrow().clone();
                    if let (Some(revision), Some(epoch)) = (current, epoch) {
                        self.send(&SyncEvent::Synced { version: Version { epoch, revision } }).await?;
                    }
                }
            }
        }
    }

    /// Reads `part` now and sends it.
    async fn send_part(&mut self, part: &Part) -> Result<(), End> {
        let only = self.only().is_some();
        match self.read_part(part).await? {
            Some(event) => self.send_read(event, only).await,
            None => Ok(()),
        }
    }

    /// Reads `part` now, as the message that carries it; none when the
    /// page is sent nothing of it. The core reads its own parts, the
    /// caller's page the mount's.
    async fn read_part(&mut self, part: &Part) -> Result<Option<SyncEvent>, End> {
        let read = if is_core_part(part) {
            read_core_part(self.core, part).await
        } else {
            let caller_page = self.caller_page.borrow();
            caller_page.read(self.core, part).await
        };
        read.map_err(|error| {
            let (name, conversation, plugin) = part_name(part);
            tracing::error!(
                event = "state_part.read_failed",
                user = %self.core.user(),
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
    /// of: a list only when what the page holds of it differs. The caller's
    /// page learns it, and names the parts it may see from now on, which go
    /// after it.
    async fn send_read(&mut self, event: SyncEvent, only: bool) -> Result<(), End> {
        // A page that may see one part alone follows nothing, and is sent
        // that part.
        let event = match self.followed(event, only) {
            Some(event) => event,
            None => return Ok(()),
        };
        let after = self.caller_page.borrow_mut().sent(&event);
        self.send(&event).await?;
        for part in after {
            Box::pin(self.send_part(&part)).await?;
        }
        Ok(())
    }

    /// The message `event` becomes for what the page follows, none when the
    /// page needs nothing of it. A list the page holds at this tag is not
    /// sent again.
    fn followed(&self, event: SyncEvent, only: bool) -> Option<SyncEvent> {
        let (list, tag) = match &event {
            SyncEvent::Instructions { etag, .. } => (SyncFollowed::Instructions, etag),
            SyncEvent::Subagents { etag, .. } => (SyncFollowed::Subagents, etag),
            SyncEvent::Credentials { etag, .. } => (SyncFollowed::Credentials, etag),
            SyncEvent::PortForwards { etag, .. } => (SyncFollowed::PortForwards, etag),
            SyncEvent::Passkeys { etag, .. } => (SyncFollowed::Passkeys, etag),
            SyncEvent::Sessions { etag, .. } => (SyncFollowed::Sessions, etag),
            SyncEvent::Users { etag, .. } => (SyncFollowed::Users, etag),
            // The one part a page may see alone, sent as it is.
            _ if only => return Some(event),
            // A part of the state: followed with the state.
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
        self.page.send(text).await.map_err(|LaneGone| End::PageGone)
    }
}

/// Reads the page's side of the lane until it ends: what it follows, which
/// `follows` and the registration keep, and what it shows and when it was
/// used, which the caller's page hears; any other message ends the channel.
async fn page_messages(
    from_page: &mut LaneReader,
    caller_page: &RefCell<Box<dyn CallerPage>>,
    registration: &Registration,
    follows: &Follows,
) -> End {
    loop {
        match from_page.next().await {
            Some(LaneMessage::Text(text)) => match serde_json::from_str::<PageMessage>(&text) {
                Ok(PageMessage::Showing { showing, front }) => caller_page.borrow().show(showing, front),
                Ok(PageMessage::Used) => caller_page.borrow().used(),
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
            Some(LaneMessage::Binary(_)) => return End::UnexpectedMessage,
            None => return End::PageGone,
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
        Part::Instance => ("instance", None, None),
        Part::User => ("user", None, None),
        Part::Workspaces => ("workspaces", None, None),
        Part::Projects => ("projects", None, None),
        Part::Devices => ("devices", None, None),
        Part::Providers => ("providers", None, None),
        Part::Logins => ("logins", None, None),
        Part::Cloud => ("cloud", None, None),
        Part::Subagents => ("subagents", None, None),
        Part::Instructions => ("instructions", None, None),
        Part::Credentials => ("credentials", None, None),
        Part::PortForwards => ("port_forwards", None, None),
        Part::Passkeys => ("passkeys", None, None),
        Part::Sessions => ("sessions", None, None),
        Part::Users => ("users", None, None),
        Part::CloudIsolation => ("cloud_isolation", None, None),
        Part::ServerUpgrade => ("server_upgrade", None, None),
        Part::Update => ("update", None, None),
        Part::Plugins => ("plugins", None, None),
        Part::Plugin(id) => ("plugin", None, Some(id.as_str())),
        Part::Models => ("models", None, None),
    }
}
