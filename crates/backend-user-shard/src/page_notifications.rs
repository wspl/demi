//! Where the user's notifications go: the push subscriptions of the user's
//! browsers, but those of a session one of whose pages shows the
//! conversation, as the pages say on their synchronization channels. A page
//! that closes or loses its connection therefore stops holding its
//! conversation's notifications back. The end of a turn notifies no session
//! one of whose pages is in front of the user, focused and visible: the page
//! shows it. Each notification goes in the background, in the order they
//! came; the accounts service's sender delivers it.
//!
//! A session is in use for [`IN_USE_MINUTES`] after one of its pages said
//! the user acted on it, while that page's channel lives. While another
//! session is in use and its own is not, a subscription quiet while the user
//! is on another device is not notified of the end of a turn: the user is at
//! the other device, which notifies. Who is notified is decided as the
//! notification comes, and nothing is sent to it later. A request that needs
//! the user and what the agent sends reach every subscription that chose
//! them.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use demi_backend_database::push::StoredPushSubscription;
use demi_core_store::StorageError;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::push::{IN_USE_MINUTES, PushMessage};
use demi_web_api_protocol::sessions::SessionId;
use tokio::time::Instant;

use crate::server_starter::ServerSide;
use demi_core::Shard;

/// How long a session stays in use after the last input to one of its pages.
const IN_USE_FOR: Duration = Duration::from_secs(IN_USE_MINUTES * 60);

/// Each session's open pages, by page: what each said on its channel.
type Pages = Rc<RefCell<HashMap<SessionId, HashMap<u64, PageSays>>>>;

/// What one page said on its synchronization channel.
#[derive(Default)]
struct PageSays {
    /// The conversation it shows, focused and visible.
    showing: Option<ConversationId>,
    /// Whether it is focused and visible, in front of the user.
    front: bool,
    /// When the user last acted on it, by the backend's clock.
    used: Option<Instant>,
}

/// The user's open pages, and the order of the notifications delivered.
#[derive(Default)]
pub(crate) struct PageNotifications {
    pages: Pages,
    /// The number the next page's synchronization channel is known by.
    next_page: Cell<u64>,
    /// Held while one notification is delivered, so the next goes after it,
    /// to the subscriptions left once its gone ones were removed.
    delivering: Rc<tokio::sync::Mutex<()>>,
}

/// One page's synchronization channel as the notifications know it, held
/// while the channel lives: what the page shows and when it was used, which
/// the backend forgets when the channel ends.
pub(crate) struct OpenPage {
    pages: Pages,
    session: SessionId,
    page: u64,
}

impl OpenPage {
    /// Notes that the page shows `conversation`, focused and visible, or
    /// none, and whether it is in front of the user.
    pub(crate) fn show(&self, conversation: Option<ConversationId>, front: bool) {
        self.says(|says| {
            says.showing = conversation;
            says.front = front;
        });
    }

    /// Notes that the user acted on the page now.
    pub(crate) fn used(&self) {
        self.says(|says| says.used = Some(Instant::now()));
    }

    fn says(&self, change: impl FnOnce(&mut PageSays)) {
        let mut pages = self.pages.borrow_mut();
        change(
            pages
                .entry(self.session.clone())
                .or_default()
                .entry(self.page)
                .or_default(),
        );
    }
}

impl Drop for OpenPage {
    fn drop(&mut self) {
        let mut pages = self.pages.borrow_mut();
        if let Some(session) = pages.get_mut(&self.session) {
            session.remove(&self.page);
            if session.is_empty() {
                pages.remove(&self.session);
            }
        }
    }
}

/// Where a notification goes now: the subscriptions that chose it, but
/// those it leaves out.
pub(crate) struct Recipients {
    /// The subscriptions it is delivered to.
    pub(crate) now: Vec<StoredPushSubscription>,
    /// How many a page of their session holds back, as it shows the
    /// conversation or, for the end of a turn, is in front of the user.
    pub(crate) shown: usize,
    /// How many stay quiet as the user is on another device.
    elsewhere: usize,
}

impl PageNotifications {
    /// A page of `session` that opens its synchronization channel, which
    /// shows nothing and was not used until it says.
    pub(crate) fn page(&self, session: SessionId) -> OpenPage {
        let page = self.next_page.get();
        self.next_page.set(page + 1);
        OpenPage {
            pages: self.pages.clone(),
            session,
            page,
        }
    }

    /// Whether a page of `session` shows `conversation`, or is in front of
    /// the user while `front` counts.
    fn shows(&self, session: &SessionId, conversation: &ConversationId, front: bool) -> bool {
        self.pages.borrow().get(session).is_some_and(|pages| {
            pages
                .values()
                .any(|says| says.showing.as_ref() == Some(conversation) || (front && says.front))
        })
    }

    /// The sessions in use now.
    pub(crate) fn in_use(&self) -> Vec<SessionId> {
        let now = Instant::now();
        self.pages
            .borrow()
            .iter()
            .filter(|(_, pages)| {
                pages
                    .values()
                    .filter_map(|says| says.used)
                    .any(|used| used + IN_USE_FOR > now)
            })
            .map(|(session, _)| session.clone())
            .collect()
    }
}

impl ServerSide {
    /// Where `message` goes now: to the subscriptions that chose it, but
    /// those of a session a page of which shows its conversation, and,
    /// for the end of a turn, those quiet while the user is on another
    /// device, when another session is in use and theirs is not.
    ///
    /// `in_use` are the sessions in use when the notification came, which
    /// decide it however long it waits for the ones before.
    pub(crate) async fn recipients(&self, user: &UserId, message: &PushMessage, in_use: &[SessionId]) -> Result<Recipients, StorageError> {
        let subscriptions = self.services.push.subscriptions(user.clone()).await?;
        let mut recipients = Recipients {
            now: Vec::new(),
            shown: 0,
            elsewhere: 0,
        };
        for subscription in subscriptions {
            if !message.chosen_by(&subscription.events) {
                continue;
            }
            if self
                .pages
                .shows(&subscription.session, message.conversation(), message.can_stay_quiet())
            {
                recipients.shown += 1;
            } else if message.can_stay_quiet()
                && subscription.quiet_while_elsewhere
                && !in_use.is_empty()
                && !in_use.contains(&subscription.session)
            {
                recipients.elsewhere += 1;
            } else {
                recipients.now.push(subscription);
            }
        }
        Ok(recipients)
    }

    /// Delivers `message` to `recipients` in the background, after the
    /// notifications before it. A failure is logged.
    pub(crate) fn deliver(shard: &Rc<Shard>, message: PushMessage, recipients: Recipients) {
        let delivering = Self::of(shard).pages.delivering.clone();
        let this = shard.clone();
        shard.tasks().spawn_local(async move {
            let _turn = delivering.lock().await;
            let started = std::time::Instant::now();
            let kind = match &message {
                PushMessage::TurnFinished { .. } => "turn_finished",
                PushMessage::TurnFailed { .. } => "turn_failed",
                PushMessage::Permission { .. } => "ask",
                PushMessage::Agent { .. } => "agent",
            };
            let Recipients { now: recipients, shown: suppressed, elsewhere } = recipients;
            if recipients.is_empty() {
                // No browser chose its kind, or each that did had a page
                // showing its conversation, as `suppressed_by_page` counts,
                // or stays quiet as the user is on another device, as
                // `quiet_elsewhere` counts.
                tracing::info!(
                    event = "notification.unsent",
                    user = %this.user(),
                    conversation = %message.conversation(),
                    kind,
                    suppressed_by_page = suppressed,
                    quiet_elsewhere = elsewhere,
                    "a notification went to no browser"
                );
                return;
            }
            let services = &Self::of(&this).services;
            let Some(contact) = services.public_url.get() else {
                // The backend serves only once it listens, which sets it.
                tracing::error!(event = "notification.too_early", conversation = %message.conversation(), "a notification came before the backend listened");
                return;
            };
            let removed = services
                .push
                .deliver(this.user(), &message, &recipients, contact.as_str())
                .await;
            // A push service that refused one is logged by the sender.
            tracing::info!(
                event = "notification.sent",
                user = %this.user(),
                conversation = %message.conversation(),
                kind,
                recipients = recipients.len(),
                suppressed_by_page = suppressed,
                quiet_elsewhere = elsewhere,
                removed_gone = removed,
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "a notification was sent to the push services of its browsers"
            );
            // Each session says whether its browser has notifications on.
            if removed {
                this.mark(demi_core_sync::Part::Sessions);
            }
        });
    }
}
