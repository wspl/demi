//! What the backend keeps of a Host's project files
//! (`crates/backend-user-shard/src/conversation/workflows.rs`).
//!
//! For each Host and directory a loaded conversation uses, the backend keeps
//! one search through the conversation's host access, in the form that never
//! wakes a Host. The runner's file watch keeps the search current
//! (`crates/runner-host/src/subscriptions.rs`).
//!
//! An agent's request takes what is held. Only the first search of a directory
//! holds it, for at most [`FIRST_SEARCH_HOLD`], so that a model's first request
//! has its instruction files.
//!
//! A fact a page shows ([`Shown`]) is stored as each search finds it, with
//! when, and the page reads what is stored at once, never waiting on the Host
//! and never following it: a search that finds it changed announces a new
//! revision to the conversations that use it.
//!
//! For example, a conversation in `~/app` asks for its instruction files before
//! its first request:
//!
//! 1. The search starts, and the request waits until it answers or 5 seconds
//!    pass.
//! 2. The search subscribes to each file it read or would read.
//! 3. When the user edits `~/app/AGENTS.md`, the runner says so and the search
//!    runs again in the background, so the next request takes the new text
//!    without asking the Host.
//! 4. A Host that comes online again is searched again.
//! 5. When no loaded conversation uses `~/app` on that Host any longer, its
//!    subscriptions end.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use std::rc::{Rc, Weak};
use std::time::Duration;

use demi_backend_remote_host::{HostEvent, HostSubscriber};
use demi_runner_protocol::files::Subject;
use demi_runner_protocol::wire::SUBSCRIPTION_LEASE;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio::sync::{Notify, watch};
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::access::HostAccessError;
use crate::plugin_files::ReadFilesError;

/// How long the first search of a directory holds the agents' requests that
/// wait for it.
pub const FIRST_SEARCH_HOLD: Duration = Duration::from_secs(5);

/// How often the subscriptions' leases are renewed: twice per lease.
const RENEWAL: Duration = SUBSCRIPTION_LEASE.checked_div(2).expect("two divides a lease");

/// What one search found, and what tells of a change to it.
pub struct Found<T> {
    pub value: T,
    pub watch: Vec<Subject>,
}

/// Why a search found nothing.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The Host is not running, and a search never wakes it.
    #[error("the Host is not running")]
    NotRunning,
    /// The conversation works on another Host than the one the search was
    /// asked of: nothing it found is kept.
    #[error("the conversation works on another Host now")]
    Moved,
    /// The conversation's host access refused the search, or the Host
    /// failed it: a caller that answers for it answers this.
    #[error(transparent)]
    Access(Rc<HostAccessError>),
    #[error("{0}")]
    Failed(String),
}

impl From<ReadFilesError> for SearchError {
    fn from(error: ReadFilesError) -> Self {
        match error {
            ReadFilesError::NotRunning => Self::NotRunning,
            ReadFilesError::Moved => Self::Moved,
            ReadFilesError::Access(error) => Self::Access(Rc::new(error)),
        }
    }
}

/// A search of the directory `K` names on the primary Host of the device,
/// through the conversation's host access, which answers
/// [`SearchError::Moved`] once the conversation works on another.
pub type Search<K, T> =
    Box<dyn Fn(ConversationId, DeviceId, K) -> LocalBoxFuture<'static, Result<Found<T>, SearchError>>>;

/// A holder of subscriptions on the conversation's primary Host, with that
/// Host's device.
pub type Subscribe =
    Box<dyn Fn(ConversationId) -> LocalBoxFuture<'static, Result<(DeviceId, HostSubscriber), SearchError>>>;

/// A kept fact a page shows: each value a search finds is stored, and a
/// stored value that changed is announced.
pub struct Shown<K, T> {
    /// Stores `T`, found on the Host of the device for `K` now; answers
    /// whether it differs from what was stored.
    pub store: Box<dyn Fn(DeviceId, K, &T) -> LocalBoxFuture<'static, Result<bool, String>>>,
    /// Tells the loaded conversations that use it that it changed.
    pub announce: Box<dyn Fn(&BTreeSet<ConversationId>)>,
}

/// One kind of search kept per Host and directory.
pub struct Kept<K, T>(Rc<Inner<K, T>>);

struct Inner<K, T> {
    /// What is searched, for the log.
    what: &'static str,
    search: Search<K, T>,
    subscribe: Subscribe,
    shown: Option<Shown<K, T>>,
    entries: RefCell<HashMap<(DeviceId, K), Entry<T>>>,
}

/// A directory kept on a Host. Dropping it ends its task, whose
/// subscriptions end with it.
struct Entry<T> {
    state: Rc<State<T>>,
    task: AbortHandle,
}

impl<T> Drop for Entry<T> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct State<T> {
    /// The loaded conversations that use it.
    users: RefCell<BTreeSet<ConversationId>>,
    held: RefCell<Option<T>>,
    /// Why the last search found nothing, while nothing is held: the
    /// conversation's host access refused it or the Host failed it.
    refused: RefCell<Option<Rc<HostAccessError>>>,
    /// How many searches ended, found something or not.
    ended: watch::Sender<u64>,
    /// The Host came online again, on a connection of its own.
    online: Cell<bool>,
    /// Searches again: the Host came online.
    wake: Notify,
    /// When the first search started.
    since: Instant,
}

impl<K, T> Kept<K, T>
where
    K: Clone + Eq + Hash + 'static,
    T: Clone + 'static,
{
    pub fn new(what: &'static str, search: Search<K, T>, subscribe: Subscribe, shown: Option<Shown<K, T>>) -> Self {
        Self(Rc::new(Inner {
            what,
            search,
            subscribe,
            shown,
            entries: RefCell::default(),
        }))
    }

    /// Keeps `key` on `device`, the loaded `conversation`'s primary Host,
    /// current, starting its search if nobody keeps it yet, without waiting
    /// for it: a page reads what [`Shown`] stored.
    pub fn keep(&self, conversation: &ConversationId, device: &DeviceId, key: K) {
        self.use_entry(conversation, device, key);
    }

    /// Why the last search of `key` on `device` was refused or failed, while
    /// nothing is held.
    pub fn refused(&self, device: &DeviceId, key: K) -> Option<Rc<HostAccessError>> {
        let entries = self.0.entries.borrow();
        let state = &entries.get(&(device.clone(), key))?.state;
        if state.held.borrow().is_some() {
            return None;
        }
        state.refused.borrow().clone()
    }

    /// What is held for `key` on `device`, the conversation's primary Host.
    ///
    /// - A loaded conversation starts keeping it if nobody does yet, and keeps
    ///   it until [`Kept::release`].
    /// - A conversation that is not loaded takes what is held, or searches once
    ///   without keeping anything.
    ///
    /// Only a first search holds the call, for at most [`FIRST_SEARCH_HOLD`]
    /// from its start. The answer is none while nothing was found, as on a Host
    /// that is not running, and the refusal when the last search was refused or
    /// failed and nothing is held.
    pub async fn get(
        &self,
        conversation: &ConversationId,
        device: &DeviceId,
        key: K,
        loaded: bool,
    ) -> Result<Option<T>, Rc<HostAccessError>> {
        let state = if loaded {
            Some(self.use_entry(conversation, device, key.clone()))
        } else {
            let entries = self.0.entries.borrow();
            entries.get(&(device.clone(), key.clone())).map(|entry| entry.state.clone())
        };
        let Some(state) = state else {
            let search = (self.0.search)(conversation.clone(), device.clone(), key);
            return match tokio::time::timeout(FIRST_SEARCH_HOLD, search).await {
                Ok(Ok(found)) => Ok(Some(found.value)),
                Ok(Err(SearchError::NotRunning | SearchError::Moved)) | Err(_) => Ok(None),
                Ok(Err(SearchError::Access(error))) => {
                    tracing::warn!(what = self.0.what, %conversation, %error, "a project search failed");
                    Err(error)
                }
                Ok(Err(SearchError::Failed(message))) => {
                    tracing::warn!(what = self.0.what, %conversation, %message, "a project search failed");
                    Ok(None)
                }
            };
        };
        let mut ended = state.ended.subscribe();
        // Past the deadline the call takes what is held, or nothing.
        let _ = tokio::time::timeout_at(state.since + FIRST_SEARCH_HOLD, ended.wait_for(|count| *count > 0)).await;
        match (state.held.borrow().clone(), state.refused.borrow().clone()) {
            (None, Some(refused)) => Err(refused),
            (held, _) => Ok(held),
        }
    }

    /// The Host of `device` came online again: what is kept there is
    /// searched again, on its new connection.
    pub fn online(&self, device: &DeviceId) {
        for ((on, _), entry) in self.0.entries.borrow().iter() {
            if on == device {
                entry.state.online.set(true);
                entry.state.wake.notify_one();
            }
        }
    }

    /// Keeps nothing of the keys `forgotten` takes, on any Host: their
    /// subscriptions end, and the next call searches again.
    pub fn forget(&self, forgotten: impl Fn(&K) -> bool) {
        self.0.entries.borrow_mut().retain(|(_, key), _| !forgotten(key));
    }

    /// The conversation is no longer loaded: what no other loaded
    /// conversation uses is no longer kept, and its subscriptions end.
    pub fn release(&self, conversation: &ConversationId) {
        self.0.entries.borrow_mut().retain(|_, entry| {
            let mut users = entry.state.users.borrow_mut();
            users.remove(conversation);
            !users.is_empty()
        });
    }

    /// The entry the loaded `conversation` uses for `key` on `device`,
    /// started if nobody kept it; the conversation no longer uses what it
    /// used on another Host.
    fn use_entry(&self, conversation: &ConversationId, device: &DeviceId, key: K) -> Rc<State<T>> {
        let mut entries = self.0.entries.borrow_mut();
        entries.retain(|(on, _), entry| {
            if on == device {
                return true;
            }
            let mut users = entry.state.users.borrow_mut();
            users.remove(conversation);
            !users.is_empty()
        });
        let entry = entries.entry((device.clone(), key.clone())).or_insert_with(|| {
            let state = Rc::new(State {
                users: RefCell::default(),
                held: RefCell::default(),
                refused: RefCell::default(),
                ended: watch::Sender::new(0),
                online: Cell::new(false),
                wake: Notify::new(),
                since: Instant::now(),
            });
            let task = tokio::task::spawn_local(keep(Rc::downgrade(&self.0), device.clone(), key, state.clone()));
            Entry {
                state,
                task: task.abort_handle(),
            }
        });
        entry.state.users.borrow_mut().insert(conversation.clone());
        entry.state.clone()
    }
}

/// Keeps `key` on `device` current: searches, holds the subscriptions the
/// search names, and searches again when one changes or the Host comes
/// online again. It ends with its entry.
async fn keep<K, T>(inner: Weak<Inner<K, T>>, device: DeviceId, key: K, state: Rc<State<T>>)
where
    K: Clone + Eq + Hash + 'static,
    T: Clone + 'static,
{
    let Some(what) = inner.upgrade().map(|inner| inner.what) else {
        return;
    };
    let mut subscriber: Option<HostSubscriber> = None;
    let mut watched: BTreeSet<Subject> = BTreeSet::new();
    // What the last search newly subscribed to and the Host does not watch
    // yet: a change before it does is told by nothing, so once none is
    // pending and one of them went live, the search runs once more. One
    // that failed alone does not search again, which would fail it again.
    let mut pending: BTreeSet<Subject> = BTreeSet::new();
    let mut went_live = false;
    let mut renewal = tokio::time::interval_at(Instant::now() + RENEWAL, RENEWAL);
    loop {
        if state.online.replace(false) {
            // What was held lived on the connection that ended.
            subscriber = None;
            watched.clear();
            pending.clear();
        }
        let Some(conversation) = state.users.borrow().first().cloned() else {
            return;
        };
        if subscriber.is_none() {
            let Some(subscribing) = inner.upgrade().map(|inner| (inner.subscribe)(conversation.clone())) else {
                return;
            };
            match subscribing.await {
                Ok((on, holder)) if on == device => subscriber = Some(holder),
                Ok(_) => {
                    // It works on another Host now; another user is searched for.
                    state.users.borrow_mut().remove(&conversation);
                    if state.users.borrow().is_empty()
                        && let Some(inner) = inner.upgrade()
                    {
                        // Ends this task.
                        inner.entries.borrow_mut().remove(&(device.clone(), key.clone()));
                    }
                    continue;
                }
                Err(SearchError::NotRunning) => {
                    state.refused.replace(None);
                }
                Err(SearchError::Moved) => {}
                // Nothing is searched until the Host comes online again:
                // while nothing is held, a caller answers the refusal.
                Err(SearchError::Access(error)) => {
                    tracing::warn!(what, %device, %error, "a project search could not subscribe; it waits for the Host to come online again");
                    state.refused.replace(Some(error));
                }
                Err(SearchError::Failed(message)) => {
                    tracing::warn!(what, %device, %message, "a project search could not subscribe; it waits for the Host to come online again");
                }
            }
        }
        if let Some(holder) = &subscriber {
            let Some(searching) = inner
                .upgrade()
                .map(|inner| (inner.search)(conversation.clone(), device.clone(), key.clone()))
            else {
                return;
            };
            match searching.await {
                Ok(found) => {
                    let wanted: BTreeSet<Subject> = found.watch.into_iter().collect();
                    for subject in watched.difference(&wanted) {
                        holder.release(subject);
                    }
                    pending = wanted.difference(&watched).cloned().collect();
                    went_live = false;
                    for subject in &pending {
                        holder.hold(subject.clone());
                    }
                    watched = wanted;
                    if let Some(shown) = inner.upgrade().as_deref().and_then(|inner| inner.shown.as_ref()) {
                        match (shown.store)(device.clone(), key.clone(), &found.value).await {
                            Ok(true) => (shown.announce)(&state.users.borrow()),
                            Ok(false) => {}
                            // The page keeps showing what was stored before;
                            // the next search stores again.
                            Err(error) => tracing::warn!(what, %device, %error, "a project search's result was not stored"),
                        }
                    }
                    *state.held.borrow_mut() = Some(found.value);
                    state.refused.replace(None);
                }
                // The Host stopped meanwhile: its coming online searches again.
                Err(SearchError::NotRunning) => {
                    state.refused.replace(None);
                }
                // The conversation moved, which releases what it used here.
                Err(SearchError::Moved) => {}
                Err(SearchError::Access(error)) => {
                    tracing::warn!(what, %device, %error, "a project search failed; the last result is kept");
                    state.refused.replace(Some(error));
                }
                Err(SearchError::Failed(message)) => {
                    tracing::warn!(what, %device, %message, "a project search failed; the last result is kept");
                }
            }
        }
        state.ended.send_modify(|count| *count += 1);
        // Until a subscription says something changed, or the Host comes
        // online again.
        loop {
            let heard = tokio::select! {
                () = state.wake.notified() => break,
                event = next_event(&mut subscriber) => event,
                _ = renewal.tick() => {
                    if let Some(holder) = &subscriber {
                        holder.renew(&watched.iter().cloned().collect::<Vec<_>>());
                    }
                    continue;
                }
            };
            match heard {
                HostEvent::Changed(_) => break,
                HostEvent::Live { subject, .. } => {
                    if pending.remove(&subject) {
                        went_live = true;
                        if pending.is_empty() {
                            break;
                        }
                    }
                }
                HostEvent::Failed { subject, reason } => {
                    tracing::warn!(what, %device, path = %subject.path, %reason, "a project file's subscription failed; a change to it goes unnoticed");
                    watched.remove(&subject);
                    if pending.remove(&subject) && pending.is_empty() && went_live {
                        break;
                    }
                }
                // The Host went: its coming online searches again.
                HostEvent::Ended => {
                    subscriber = None;
                    watched.clear();
                    pending.clear();
                }
            }
        }
    }
}

/// What the Host says next of what `subscriber` holds; nothing without one.
async fn next_event(subscriber: &mut Option<HostSubscriber>) -> HostEvent {
    match subscriber {
        Some(subscriber) => subscriber.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use demi_backend_remote_host::testing::{FixtureOptions, RunnerFixture};
    use demi_plugin_interface::{HostFile, HostRead};
    use demi_runner_protocol::files::SubscriptionKind;
    use tokio::sync::mpsc;

    use super::*;
    use crate::plugin_files::look_files;

    /// A [`Shown`] that stores in memory and sends each value it announces
    /// on `announced`, with the conversations it tells.
    fn shown(announced: mpsc::UnboundedSender<(String, BTreeSet<ConversationId>)>) -> Shown<String, String> {
        let stored: Rc<RefCell<Option<String>>> = Rc::default();
        let last = stored.clone();
        Shown {
            store: Box::new(move |_, _, value: &String| {
                let changed = stored.borrow().as_ref() != Some(value);
                *stored.borrow_mut() = Some(value.clone());
                Box::pin(async move { Ok(changed) })
            }),
            announce: Box::new(move |users| {
                let value = last.borrow().clone().expect("stored before it is announced");
                announced.send((value, users.clone())).unwrap();
            }),
        }
    }

    /// Subscriptions on `fixture`'s Host, as the device `device`.
    fn subscribe(fixture: &RunnerFixture, device: &DeviceId) -> Subscribe {
        let (host, device) = (fixture.host(), device.clone());
        Box::new(move |_| {
            let (host, device) = (host.clone(), device.clone());
            Box::pin(async move {
                let subscriber = host.subscriber().map_err(|error| SearchError::Failed(error.to_string()))?;
                Ok((device, subscriber))
            })
        })
    }

    // About a second: a real runner process reads the file and watches it.
    //
    // Planted defect it catches: a release that leaves the subscriptions
    // held, a search per call, or an edit the runner reports that is never
    // stored and announced to the conversations that use it.
    #[tokio::test(flavor = "local")]
    async fn a_directory_is_searched_once_kept_by_the_watch_and_released_with_its_last_conversation() {
        let fixture = RunnerFixture::start(FixtureOptions::default()).await;
        let path = fixture.home_dir().join("AGENTS.md");
        std::fs::write(&path, "one").unwrap();
        let path = path.to_str().unwrap().to_owned();
        let subject = Subject {
            kind: SubscriptionKind::File,
            path: path.clone(),
            suffix: None,
        };
        let searches = Rc::new(Cell::new(0));
        let host = fixture.host();
        let search: Search<String, String> = {
            let (host, searches, subject) = (host.clone(), searches.clone(), subject.clone());
            Box::new(move |_, _, path: String| {
                let (host, searches, subject) = (host.clone(), searches.clone(), subject.clone());
                Box::pin(async move {
                    searches.set(searches.get() + 1);
                    let (found, _) = look_files(&host, &[HostRead::new(path, 64)])
                        .await
                        .map_err(|error| SearchError::Failed(error.to_string()))?;
                    let HostFile::File { bytes, .. } = &found[0] else {
                        panic!("{:?}", found[0]);
                    };
                    Ok(Found {
                        value: String::from_utf8_lossy(bytes.as_bytes()).into_owned(),
                        watch: vec![subject],
                    })
                })
            })
        };
        let device = DeviceId::try_from("device-1").unwrap();
        let (announced, mut announcements) = mpsc::unbounded_channel();
        let kept = Kept::new("a test's file", search, subscribe(&fixture, &device), Some(shown(announced)));
        let first = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap();
        let second = ConversationId::try_from("1c7e8f4f-9a4b-4d2f-8e3c-8b2d3f4a5b6c").unwrap();

        assert_eq!(kept.get(&first, &device, path.clone(), true).await.unwrap().as_deref(), Some("one"));
        assert_eq!(kept.get(&second, &device, path.clone(), true).await.unwrap().as_deref(), Some("one"));
        assert_eq!(announcements.recv().await.unwrap().0, "one");
        let link = fixture.link().await;
        assert_eq!(link.subscribed(), [subject.clone()]);

        // The edit is searched, whether the runner reports it or it came
        // before the Host watched the file, and announced to both.
        std::fs::write(&path, "two").unwrap();
        let (value, users) = announcements.recv().await.unwrap();
        assert_eq!((value.as_str(), users), ("two", BTreeSet::from([first.clone(), second.clone()])));
        // Taking what is held asks the Host nothing.
        let searched = searches.get();
        for conversation in [&first, &second, &first] {
            let held = kept.get(conversation, &device, path.clone(), true).await.unwrap();
            assert_eq!(held.as_deref(), Some("two"));
        }
        assert_eq!(searches.get(), searched);

        // Held while one conversation still uses it, released with the last.
        kept.release(&first);
        tokio::task::yield_now().await;
        assert_eq!(link.subscribed(), [subject]);
        kept.release(&second);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !link.subscribed().is_empty() {
            assert!(Instant::now() < deadline, "the subscription was never released");
            tokio::task::yield_now().await;
        }
        fixture.stop().await;
    }

    // Instant: the search is held until the test lets it go.
    //
    // Planted defect it catches: a page's read that waits for a directory's
    // first search, as one held it five seconds, or a search that finds what
    // was stored announcing it again.
    #[tokio::test(flavor = "local")]
    async fn keeping_for_a_page_waits_for_no_search_and_only_a_change_is_announced() {
        let fixture = RunnerFixture::start(FixtureOptions::default()).await;
        let gate = Rc::new(Notify::new());
        let search: Search<String, String> = {
            let gate = gate.clone();
            Box::new(move |_, _, _| {
                let gate = gate.clone();
                Box::pin(async move {
                    gate.notified().await;
                    Ok(Found {
                        value: "same".to_owned(),
                        watch: Vec::new(),
                    })
                })
            })
        };
        let device = DeviceId::try_from("device-1").unwrap();
        let (announced, mut announcements) = mpsc::unbounded_channel();
        let kept = Kept::new("the workflows", search, subscribe(&fixture, &device), Some(shown(announced)));
        let conversation = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap();

        // The search waits for the gate; keeping for the page returns at once.
        kept.keep(&conversation, &device, "/p".to_owned());
        gate.notify_one();
        assert_eq!(announcements.recv().await.unwrap().0, "same");
        // The Host comes online again: it is searched again and finds what
        // was stored, which nobody is told of.
        kept.online(&device);
        gate.notify_one();
        let mut ended = kept.0.entries.borrow()[&(device.clone(), "/p".to_owned())].state.ended.subscribe();
        ended.wait_for(|count| *count >= 2).await.unwrap();
        assert!(announcements.try_recv().is_err());

        drop(kept);
        fixture.stop().await;
    }
}
