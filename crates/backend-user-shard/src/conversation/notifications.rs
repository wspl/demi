//! The push notifications of the user's conversations.
//!
//! Three things notify:
//!
//! - a turn of the root that ends, with an answer or an error, once the
//!   conversation no longer runs;
//! - a request that asks the user;
//! - what the agent sends with `demi notify send`, within its limits.
//!
//! Each goes, in the background and in the order they came, to the subscriptions
//! that chose it, but those of a session one of whose pages shows its
//! conversation, as the pages say on their synchronization channels. A page that
//! closes or loses its connection therefore stops holding its conversation's
//! notifications back. The accounts service's sender delivers it.
//!
//! A turn the user stopped notifies nothing, since the user was there.
//!
//! # Telling turns apart
//!
//! A turn's end is told apart from the turns that ended before by the turn that
//! had ended last when the tree started to work, which the shard keeps while it
//! works. As the tree rests, the turn that ended last then is the one notified
//! of, read from the session at once, so a turn that starts after it never
//! stands in for it.
//!
//! - A turn that starts and ends between two status changes still notifies.
//! - Work that ends no turn, such as a compaction or a retry the user stopped,
//!   notifies nothing.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use demi_agent_session::{EndedTurn, TurnOutcome};
use demi_backend_database::StorageError;
use demi_backend_database::push::StoredPushSubscription;
use demi_backend_host_access::root_of;
use demi_shared_types::{BlockId, Timestamp};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::permissions::PermissionRequestId;
use demi_web_api_protocol::push::{ANSWER_START_CHARS, PushAsk, PushMessage};
use demi_web_api_protocol::sessions::SessionId;
use jiff::SignedDuration;

use crate::shard::Shard;

/// How many notifications a conversation's agent sends at most in a minute,
/// and in an hour.
pub(crate) const PER_MINUTE: usize = 3;
pub(crate) const PER_HOUR: usize = 30;

/// What each session's open pages show, by page: the conversation of each
/// that is focused and visible.
type Shown = Rc<RefCell<HashMap<SessionId, HashMap<u64, ConversationId>>>>;

/// What the shard keeps for the notifications of its conversations.
#[derive(Default)]
pub(crate) struct Notifications {
    /// Each conversation whose tree works, with the block that ended its
    /// latest ended turn when the work started.
    working: RefCell<HashMap<ConversationId, Option<BlockId>>>,
    /// When each conversation's agent sent its notifications of the last
    /// hour, oldest first.
    sent: RefCell<HashMap<ConversationId, VecDeque<Timestamp>>>,
    shown: Shown,
    /// The number the next page's synchronization channel is known by.
    next_page: Cell<u64>,
    /// Held while one notification is delivered, so the next goes after it,
    /// to the subscriptions left once its gone ones were removed.
    delivering: tokio::sync::Mutex<()>,
}

/// Why the agent may not send a notification now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum TooMany {
    #[error("too many notifications; at most {PER_MINUTE} a minute")]
    Minute,
    #[error("too many notifications; at most {PER_HOUR} an hour")]
    Hour,
}

/// What one page's synchronization channel shows, held while the channel
/// lives: it forgets what it showed when it ends.
pub(crate) struct PageShows {
    shown: Shown,
    session: SessionId,
    page: u64,
}

impl PageShows {
    /// Notes that the page shows `conversation`, focused and visible, or
    /// none.
    pub(crate) fn set(&self, conversation: Option<ConversationId>) {
        let mut shown = self.shown.borrow_mut();
        let pages = shown.entry(self.session.clone()).or_default();
        match conversation {
            Some(conversation) => {
                pages.insert(self.page, conversation);
            }
            None => {
                pages.remove(&self.page);
            }
        }
        if pages.is_empty() {
            shown.remove(&self.session);
        }
    }
}

impl Drop for PageShows {
    fn drop(&mut self) {
        self.set(None);
    }
}

impl Notifications {
    /// Counts a notification the conversation's agent sends at `now`, or
    /// refuses it when the conversation sent its most in the hour or the
    /// minute before, naming the hour's limit when both are reached.
    pub(crate) fn take(&self, conversation: &ConversationId, now: Timestamp) -> Result<(), TooMany> {
        let mut sent = self.sent.borrow_mut();
        let times = sent.entry(conversation.clone()).or_default();
        let age = |time: &Timestamp| now.to_jiff().duration_since(time.to_jiff());
        while times.front().is_some_and(|time| age(time) >= SignedDuration::from_hours(1)) {
            times.pop_front();
        }
        if times.len() >= PER_HOUR {
            return Err(TooMany::Hour);
        }
        if times.iter().filter(|time| age(time) < SignedDuration::from_mins(1)).count() >= PER_MINUTE {
            return Err(TooMany::Minute);
        }
        times.push_back(now);
        Ok(())
    }

    /// What a page of `session` that opens its synchronization channel
    /// shows, nothing until it says.
    pub(crate) fn page(&self, session: SessionId) -> PageShows {
        let page = self.next_page.get();
        self.next_page.set(page + 1);
        PageShows {
            shown: self.shown.clone(),
            session,
            page,
        }
    }

    /// Whether a page of `session` shows `conversation`.
    fn shows(&self, session: &SessionId, conversation: &ConversationId) -> bool {
        self.shown
            .borrow()
            .get(session)
            .is_some_and(|pages| pages.values().any(|shown| shown == conversation))
    }
}

impl Shard {
    pub(crate) fn notifications(&self) -> &Notifications {
        &self.notifications
    }

    /// Follows the conversation's status: as its tree starts to work, notes
    /// the turn that had ended last; as it rests, notifies of the turn that
    /// ended last then, if it is another. Called on each change of the
    /// tree's status.
    pub(crate) fn follow_turn_end(&self, conversation: &ConversationId) {
        if self.is_closing() {
            return;
        }
        let tree = self.agent().tree(&root_of(conversation));
        let mut working = self.notifications.working.borrow_mut();
        match tree {
            Some(tree) if tree.works() => {
                working
                    .entry(conversation.clone())
                    .or_insert_with(|| tree.root().session().ended_turn());
            }
            // A tree no longer loaded did not end a turn as it closed.
            None => {
                working.remove(conversation);
            }
            Some(tree) => {
                let Some(before) = working.remove(conversation) else {
                    return;
                };
                let Some(ended) = tree.root().session().ended_turn_result() else {
                    return;
                };
                if before.as_ref() == Some(&ended.id) || ended.outcome == TurnOutcome::Stopped {
                    return;
                }
                let this = self.this();
                let conversation = conversation.clone();
                self.tasks()
                    .spawn_local(async move { this.notify_turn_end(conversation, ended).await });
            }
        }
    }

    /// Notifies of `ended`, a turn the conversation's root ended with an
    /// answer or an error.
    async fn notify_turn_end(&self, conversation: ConversationId, ended: EndedTurn) {
        let Some(title) = self.conversation_title(&conversation).await else {
            return;
        };
        let turn = ended.id.as_str().to_owned();
        let message = match ended.outcome {
            TurnOutcome::Answered => PushMessage::TurnFinished {
                conversation_id: conversation,
                title,
                turn,
                answer_start: ended
                    .answer
                    .map(|answer| answer.chars().take(ANSWER_START_CHARS).collect()),
            },
            TurnOutcome::Failed => PushMessage::TurnFailed {
                conversation_id: conversation,
                title,
                turn,
            },
            TurnOutcome::Stopped => return,
        };
        self.push(message, None);
    }

    /// Notifies that the conversation asks the user what `ask` says.
    pub(crate) fn notify_asked(&self, conversation: &ConversationId, request: PermissionRequestId, ask: PushAsk) {
        let this = self.this();
        let conversation = conversation.clone();
        self.tasks().spawn_local(async move {
            let Some(title) = this.conversation_title(&conversation).await else {
                return;
            };
            let message = PushMessage::Permission {
                conversation_id: conversation,
                title,
                request,
                ask,
            };
            this.push(message, None);
        });
    }

    /// The conversation's title, null while it has none; none when the
    /// conversation is gone or cannot be read.
    async fn conversation_title(&self, conversation: &ConversationId) -> Option<Option<String>> {
        match self.services().control.conversation(conversation.clone()).await {
            Ok(record) => record.map(|record| record.title),
            Err(error) => {
                tracing::warn!(%conversation, "a conversation was not read for its notification: {error}");
                None
            }
        }
    }

    /// The subscriptions `message` goes to now: those that chose it, but
    /// those of a session a page of which shows its conversation.
    pub(crate) async fn recipients(&self, message: &PushMessage) -> Result<Vec<StoredPushSubscription>, StorageError> {
        let subscriptions = self.services().push.subscriptions(self.user().clone()).await?;
        Ok(subscriptions
            .into_iter()
            .filter(|subscription| {
                message.chosen_by(&subscription.events)
                    && !self.notifications.shows(&subscription.session, message.conversation())
            })
            .collect())
    }

    /// Delivers `message` in the background, after the notifications before
    /// it: to `recipients`, or else to the subscriptions it goes to then. A
    /// failure is logged.
    pub(crate) fn push(&self, message: PushMessage, recipients: Option<Vec<StoredPushSubscription>>) {
        let this = self.this();
        self.tasks().spawn_local(async move {
            let _turn = this.notifications.delivering.lock().await;
            let recipients = match recipients {
                Some(recipients) => recipients,
                None => match this.recipients(&message).await {
                    Ok(recipients) => recipients,
                    Err(error) => {
                        tracing::warn!(user = %this.user(), "the push subscriptions were not read: {error}");
                        return;
                    }
                },
            };
            if recipients.is_empty() {
                return;
            }
            let services = this.services();
            let Some(contact) = services.public_url.get() else {
                // The backend serves only once it listens, which sets it.
                tracing::error!("a notification came before the backend listened");
                return;
            };
            let removed = services
                .push
                .deliver(this.user(), &message, &recipients, contact.as_str())
                .await;
            // Each session says whether its browser has notifications on.
            if removed {
                this.mark(demi_backend_page_sync::Part::Sessions);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cost: a few counts in memory; under a millisecond.

    #[test]
    fn a_conversation_sends_at_most_three_notifications_a_minute_and_thirty_an_hour() {
        let notifications = Notifications::default();
        let conversation = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap();
        let other = ConversationId::try_from("1b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap();
        let start = Timestamp::from_millisecond(1_800_000_000_000).unwrap();
        let at = |seconds: i64| Timestamp::truncate(start.to_jiff() + SignedDuration::from_secs(seconds));
        for second in 0..3 {
            assert_eq!(notifications.take(&conversation, at(second)), Ok(()));
        }
        assert_eq!(notifications.take(&conversation, at(59)), Err(TooMany::Minute));
        // Each conversation counts its own.
        assert_eq!(notifications.take(&other, at(59)), Ok(()));
        // A minute after the first, one more may go; a refusal counted none.
        assert_eq!(notifications.take(&conversation, at(60)), Ok(()));
        // Three a minute reach thirty in the tenth minute; the hour then
        // refuses until the first of them is an hour old.
        for minute in 2..11 {
            for second in 0..3 {
                let sent = notifications.take(&conversation, at(minute * 60 + second));
                let expected = if minute == 10 && second == 2 { Err(TooMany::Hour) } else { Ok(()) };
                assert_eq!(sent, expected, "minute {minute}, second {second}");
            }
        }
        assert_eq!(notifications.take(&conversation, at(3599)), Err(TooMany::Hour));
        assert_eq!(notifications.take(&conversation, at(3600)), Ok(()));
        assert_eq!(TooMany::Minute.to_string(), "too many notifications; at most 3 a minute");
        assert_eq!(TooMany::Hour.to_string(), "too many notifications; at most 30 an hour");
    }
}
