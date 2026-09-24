//! A browser environment's tab registry (`browser.md` § One tab registry).
//!
//! One owner task follows Chrome's target events through bounded buffers and
//! keeps the public ID, opener and creation metadata of every top-level page
//! for the browser's generation, closed tabs included. It publishes a
//! snapshot after every change: a command finds its tab there, without a call
//! to Chrome and without waiting for a command on another tab. Opening and
//! closing a tab are requests to the owner, which also decides when the last
//! tab has closed.
//!
//! The owner keeps its bookkeeping in a [`Book`], which knows nothing of
//! Chrome, and does the calls to Chrome itself or in tasks of the
//! environment.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        page::StopLoadingParams,
        target::{
            CloseTargetParams, EventAttachedToTarget, EventTargetCreated, EventTargetDestroyed,
            EventTargetInfoChanged, GetTargetsParams, TargetId, TargetInfo,
        },
    },
    listeners::{EventStream, EventStreamError},
};
use futures_util::StreamExt;
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    BrowserError, BrowserTab, Result,
    environment::BrowserHandle,
    operation::{CONTROL_TIMEOUT, after_cleanup},
    protocol::{BrowserCreatedBy, TabId},
    tab::TabState,
};

/// Target events of one kind the owner has not handled yet. Past this it
/// has lost events and reconciles from Chrome's target list.
const EVENTS: usize = 256;
/// Requests waiting for the owner; a full queue holds back their senders.
const REQUESTS: usize = 64;
/// Targets destroyed lately. Each kind of target event arrives on a buffer
/// of its own, so a page's creation can reach the owner after its
/// destruction; remembering the destruction keeps that page out.
const DESTROYED: usize = 1024;

/// A tab as the registry lists it.
#[derive(Clone)]
pub(super) struct Listed {
    pub tab: BrowserTab,
    pub title: String,
    pub url: String,
}

/// The registry as its owner last published it.
pub(super) struct Snapshot {
    /// The live tabs, in the order the browser created them.
    pub tabs: Vec<Listed>,
    /// The owner is still registering tabs it knows of, such as a popup
    /// being set up, or is reconciling after lost events: a listing waits
    /// until it is done.
    pub registering: bool,
}

impl Snapshot {
    pub fn find(&self, id: &TabId) -> Option<&BrowserTab> {
        self.tabs
            .iter()
            .map(|listed| &listed.tab)
            .find(|tab| tab.id() == id)
    }
}

/// How closing a tab ended.
pub(super) enum Closed {
    /// The tab is gone from the browser and from the registry.
    Tab,
    /// It was the last tab: the environment retires instead, without waiting
    /// for the page to go (`browser.md` § Lifetime).
    Environment,
}

/// The registry's snapshot and the way to its owner.
#[derive(Clone)]
pub(super) struct Tabs {
    requests: mpsc::Sender<Request>,
    snapshot: watch::Receiver<Arc<Snapshot>>,
    emptied: CancellationToken,
}

impl Tabs {
    async fn ask<T>(&self, request: impl FnOnce(oneshot::Sender<Result<T>>) -> Request) -> Result<T> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(request(reply))
            .await
            .map_err(|_| BrowserError::Closed)?;
        // The owner drops its requests when the environment ends.
        answer.await.map_err(|_| BrowserError::Closed)?
    }

    /// Asks for a new blank tab, registered once it can be operated. A
    /// creation whose answer nobody waits for closes its tab again.
    pub async fn create(&self, created_by: BrowserCreatedBy) -> Result<Creation> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Create { created_by, reply })
            .await
            .map_err(|_| BrowserError::Closed)?;
        Ok(Creation { answer })
    }

    /// Closes `target`, answering once the registry no longer lists it.
    pub async fn close(&self, target: TargetId, deadline: Instant) -> Result<Closed> {
        self.ask(|reply| Request::Close {
            target,
            deadline,
            reply,
        })
        .await
    }

    /// Keeps the environment from retiring while a batch of temporary tabs
    /// collects its results.
    pub async fn hold(&self) -> Result<Hold> {
        self.ask(|reply| Request::Hold { reply }).await
    }

    /// Whether the registry has emptied, once its owner has seen every hold
    /// dropped before this call.
    pub async fn settled(&self) -> Result<bool> {
        self.ask(|reply| Request::Settled { reply }).await
    }

    /// The pages the page of `opener` opened that the registry knows of,
    /// whether or not they are registered yet.
    pub async fn opened_by(&self, opener: TargetId) -> Result<Vec<TabId>> {
        self.ask(|reply| Request::OpenedBy { opener, reply }).await
    }

    /// The tabs the page of `opener` opened that the registry knows of, once
    /// each is registered and can be operated, in the order they opened.
    pub async fn popups(&self, opener: TargetId) -> Result<Vec<TabId>> {
        self.ask(|reply| Request::Popups { opener, reply }).await
    }

    /// The latest snapshot, once the owner has registered every tab it
    /// knows of.
    pub async fn listing(&self) -> Result<Arc<Snapshot>> {
        let mut snapshot = self.snapshot.clone();
        let listing = snapshot
            .wait_for(|snapshot| !snapshot.registering)
            .await
            .map_err(|_| BrowserError::Closed)?
            .clone();
        Ok(listing)
    }

    /// The tab with public ID `id`. A tab the latest snapshot lists is found
    /// at once; otherwise the owner may still be registering it.
    pub async fn find(&self, id: &TabId) -> Result<BrowserTab> {
        if let Some(tab) = self.latest().find(id) {
            return Ok(tab.clone());
        }
        self.listing()
            .await?
            .find(id)
            .cloned()
            .ok_or(BrowserError::TabNotFound)
    }

    /// The latest snapshot as it is.
    pub fn latest(&self) -> Arc<Snapshot> {
        self.snapshot.borrow().clone()
    }

    /// Cancelled once the last tab has closed. The registry then admits no
    /// new tab, and the environment's owner retires the environment.
    pub fn emptied(&self) -> &CancellationToken {
        &self.emptied
    }
}

/// A tab creation the owner accepted.
pub(super) struct Creation {
    answer: oneshot::Receiver<Result<BrowserTab>>,
}

impl Creation {
    /// The created tab, once registered; waiting again after an interrupted
    /// wait takes up where it left off.
    pub async fn tab(&mut self) -> Result<BrowserTab> {
        // The owner drops its requests when the environment ends.
        (&mut self.answer).await.map_err(|_| BrowserError::Closed)?
    }
}

/// A batch of temporary tabs in progress; dropping it lets the environment
/// retire once it has no tab.
pub(super) struct Hold {
    holds: watch::Sender<usize>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.holds.send_modify(|holds| *holds -= 1);
    }
}

enum Request {
    Create {
        created_by: BrowserCreatedBy,
        reply: oneshot::Sender<Result<BrowserTab>>,
    },
    Close {
        target: TargetId,
        deadline: Instant,
        reply: oneshot::Sender<Result<Closed>>,
    },
    Hold {
        reply: oneshot::Sender<Result<Hold>>,
    },
    Settled {
        reply: oneshot::Sender<Result<bool>>,
    },
    /// The pages `opener` opened that the registry knows of, registered or
    /// not; answered at once.
    OpenedBy {
        opener: TargetId,
        reply: oneshot::Sender<Result<Vec<TabId>>>,
    },
    /// The tabs `opener` opened, once each is registered.
    Popups {
        opener: TargetId,
        reply: oneshot::Sender<Result<Vec<TabId>>>,
    },
    /// A creation's page, or why Chrome made none.
    Opened {
        page: Result<Page>,
        created_by: BrowserCreatedBy,
        reply: oneshot::Sender<Result<BrowserTab>>,
    },
    /// The tab set up for `target`, or why it could not be; `reply` is its
    /// creator's.
    Ready {
        target: TargetId,
        tab: Result<BrowserTab>,
        reply: Option<oneshot::Sender<Result<BrowserTab>>>,
    },
    /// Chrome did not close `target`.
    NotClosed { target: TargetId, error: BrowserError },
}

/// A top-level page as an event or Chrome's target list shows it.
struct Sighting<'a> {
    target: &'a TargetId,
    title: &'a str,
    url: &'a str,
    opener: Option<&'a TargetId>,
}

impl<'a> Sighting<'a> {
    /// The sighting of a top-level page; none for any other target.
    fn of(info: &'a TargetInfo) -> Option<Self> {
        (info.r#type == "page").then_some(Self {
            target: &info.target_id,
            title: &info.title,
            url: &info.url,
            opener: info.opener_id.as_ref(),
        })
    }
}

/// What the registry knows of the browser's top-level pages, apart from
/// Chrome: their public IDs, openers and stages, the creations in flight,
/// and whether the last tab has closed. `T` is the tab a live page has.
struct Book<T> {
    entries: HashMap<TargetId, Entry<T>>,
    /// Each top-level page's public ID and creation order, kept for the
    /// generation, closed tabs included.
    public_ids: HashMap<TargetId, (TabId, usize)>,
    openers: HashMap<TargetId, TargetId>,
    destroyed: VecDeque<TargetId>,
    /// Creations requested and not yet registered or failed.
    creating: usize,
    /// The last tab has closed: no new tab is admitted.
    sealed: bool,
    reconciling: bool,
}

struct Entry<T> {
    title: String,
    url: String,
    stage: Stage<T>,
}

enum Stage<T> {
    /// Known from an event. A popup is set up once Chrome attached it; a
    /// page without an opener is one of the registry's own creations, set
    /// up for its creator.
    Pending,
    /// Its tab is being set up.
    Setup,
    Live { tab: T, closing: bool },
    /// Chrome shows the page, but its tab could not be set up.
    Unusable,
}

impl<T: Clone> Book<T> {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            public_ids: HashMap::new(),
            openers: HashMap::new(),
            destroyed: VecDeque::new(),
            creating: 0,
            sealed: false,
            reconciling: false,
        }
    }

    /// Records a page; true when the listing changed.
    fn found(&mut self, seen: &Sighting<'_>) -> bool {
        if self.was_destroyed(seen.target) {
            return false;
        }
        self.public_id(seen.target);
        // Chrome may leave the opener out of later target information once
        // the opener closed; the first sighting keeps it.
        if let Some(opener) = seen.opener {
            self.public_id(opener);
            self.openers
                .entry(seen.target.clone())
                .or_insert_with(|| opener.clone());
        }
        let entry = self
            .entries
            .entry(seen.target.clone())
            .or_insert_with(|| Entry {
                title: String::new(),
                url: String::new(),
                stage: Stage::Pending,
            });
        let changed = entry.title != seen.title || entry.url != seen.url;
        seen.title.clone_into(&mut entry.title);
        seen.url.clone_into(&mut entry.url);
        changed && matches!(entry.stage, Stage::Live { .. })
    }

    fn is_pending(&self, target: &TargetId) -> bool {
        self.entries
            .get(target)
            .is_some_and(|entry| matches!(entry.stage, Stage::Pending))
    }

    fn opener(&self, target: &TargetId) -> Option<&TargetId> {
        self.openers.get(target)
    }

    /// Records that `opener` opened `target`; the opener's public ID.
    fn opened_by(&mut self, target: &TargetId, opener: TargetId) -> TabId {
        let id = self.public_id(&opener);
        self.openers.insert(target.clone(), opener);
        id
    }

    /// Admits a creation, unless the last tab has closed.
    fn admit(&mut self) -> bool {
        if self.sealed {
            return false;
        }
        self.creating += 1;
        true
    }

    /// A creation ended without a page.
    fn failed(&mut self) {
        self.creating -= 1;
    }

    /// The public IDs of every page `opener` opened, closed or being
    /// registered ones included.
    fn opened(&self, opener: &TargetId) -> Vec<TabId> {
        self.openers
            .iter()
            .filter(|(_, known)| *known == opener)
            .filter_map(|(target, _)| self.public_ids.get(target))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The live tabs `opener` opened, in the order they opened; none while
    /// one of the pages it opened is still being registered.
    fn popups(&self, opener: &TargetId) -> Option<Vec<TabId>> {
        let mut popups = Vec::new();
        for (target, entry) in &self.entries {
            if self.openers.get(target) != Some(opener) {
                continue;
            }
            match entry.stage {
                Stage::Pending | Stage::Setup => return None,
                Stage::Live { .. } => {
                    popups.push(self.public_ids.get(target).expect("a live page has a public ID"))
                }
                Stage::Unusable => {}
            }
        }
        popups.sort_by_key(|(_, order)| *order);
        Some(popups.into_iter().map(|(id, _)| id.clone()).collect())
    }

    /// The page's tab is being set up; its public ID.
    fn set_up(&mut self, target: &TargetId) -> TabId {
        let id = self.public_id(target);
        self.entries
            .entry(target.clone())
            .or_insert_with(|| Entry {
                title: String::new(),
                url: String::new(),
                stage: Stage::Pending,
            })
            .stage = Stage::Setup;
        id
    }

    /// A page's set-up tab, or none when it could not be set up; why it is
    /// refused, if it is. `created` says whether a creation asked for it.
    fn ready(&mut self, target: &TargetId, created: bool, tab: Option<T>) -> Option<BrowserError> {
        if created {
            self.creating -= 1;
        }
        let refused = if self.sealed {
            Some(BrowserError::Closed)
        } else if self.was_destroyed(target) {
            Some(BrowserError::TabNotFound)
        } else {
            None
        };
        let stage = match (tab, &refused) {
            (Some(tab), None) => Stage::Live {
                tab,
                closing: false,
            },
            _ => Stage::Unusable,
        };
        if let Some(entry) = self.entries.get_mut(target) {
            entry.stage = stage;
        }
        refused
    }

    /// Whether `target` is a live tab nobody is closing.
    fn closable(&self, target: &TargetId) -> bool {
        matches!(
            self.entries.get(target),
            Some(Entry {
                stage: Stage::Live { closing: false, .. },
                ..
            })
        )
    }

    /// Marks `target` as closing; its tab, if nobody was closing it.
    fn start_closing(&mut self, target: &TargetId) -> Option<T> {
        match self.entries.get_mut(target) {
            Some(Entry {
                stage: Stage::Live { tab, closing },
                ..
            }) if !*closing => {
                *closing = true;
                Some(tab.clone())
            }
            _ => None,
        }
    }

    /// Chrome did not close `target`, which may be closed again.
    fn not_closed(&mut self, target: &TargetId) {
        if let Some(Entry {
            stage: Stage::Live { closing, .. },
            ..
        }) = self.entries.get_mut(target)
        {
            *closing = false;
        }
    }

    /// Chrome destroyed `target`: its tab, if it was live, and whether the
    /// listing changed.
    fn gone(&mut self, target: &TargetId) -> (Option<T>, bool) {
        if self.destroyed.len() == DESTROYED {
            self.destroyed.pop_front();
        }
        self.destroyed.push_back(target.clone());
        match self.entries.remove(target).map(|entry| entry.stage) {
            Some(Stage::Live { tab, .. }) => (Some(tab), true),
            // A tab being set up no longer holds up a listing.
            Some(Stage::Setup) => (None, true),
            Some(Stage::Pending | Stage::Unusable) | None => (None, false),
        }
    }

    fn was_destroyed(&self, target: &TargetId) -> bool {
        self.destroyed.contains(target)
    }

    /// Whether an entry keeps the environment from being empty.
    fn counts(&self, target: &TargetId, entry: &Entry<T>) -> bool {
        match entry.stage {
            Stage::Setup | Stage::Live { .. } => true,
            // A popup Chrome has not attached yet; a creation's page is
            // counted by its creation.
            Stage::Pending => self.openers.contains_key(target),
            Stage::Unusable => false,
        }
    }

    /// Whether `target` is the one tab left, with none being created and no
    /// batch held.
    fn only(&self, target: &TargetId, holds: usize) -> bool {
        self.creating == 0
            && holds == 0
            && self
                .entries
                .iter()
                .all(|(id, entry)| id == target || !self.counts(id, entry))
    }

    /// Seals the registry if no tab remains, none is being created and no
    /// batch is held; true if it sealed now.
    fn settle(&mut self, holds: usize) -> bool {
        let empty = self.creating == 0
            && holds == 0
            && !self
                .entries
                .iter()
                .any(|(target, entry)| self.counts(target, entry));
        if self.sealed || !empty {
            return false;
        }
        self.sealed = true;
        true
    }

    /// The live tabs in creation order, with their titles and URLs, and
    /// whether the owner is still registering tabs.
    fn listing(&self) -> (Vec<(T, String, String)>, bool) {
        let mut tabs: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(target, entry)| match &entry.stage {
                Stage::Live { tab, .. } => Some((
                    self.public_ids
                        .get(target)
                        .map_or(usize::MAX, |(_, order)| *order),
                    (tab.clone(), entry.title.clone(), entry.url.clone()),
                )),
                Stage::Pending | Stage::Setup | Stage::Unusable => None,
            })
            .collect();
        tabs.sort_by_key(|(order, _)| *order);
        let registering = self.reconciling
            || self
                .entries
                .values()
                .any(|entry| matches!(entry.stage, Stage::Setup));
        (tabs.into_iter().map(|(_, tab)| tab).collect(), registering)
    }

    /// The pages the registry knows of that Chrome's target list lacks.
    fn vanished(&self, present: &HashSet<TargetId>) -> Vec<TargetId> {
        self.entries
            .keys()
            .filter(|target| !present.contains(*target))
            .cloned()
            .collect()
    }

    /// The pages waiting for Chrome to attach them.
    fn pending(&self) -> Vec<TargetId> {
        self.entries
            .iter()
            .filter(|(_, entry)| matches!(entry.stage, Stage::Pending))
            .map(|(target, _)| target.clone())
            .collect()
    }

    /// The public ID of `target`, kept for the browser's generation.
    fn public_id(&mut self, target: &TargetId) -> TabId {
        if let Some((id, _)) = self.public_ids.get(target) {
            return id.clone();
        }
        let order = self.public_ids.len();
        // The system's random source does not fail on a working system.
        let id = TabId::from_random(
            super::handles::random().expect("the system's random source works"),
        );
        self.public_ids.insert(target.clone(), (id.clone(), order));
        id
    }
}

/// What setting up a tab needs.
#[derive(Clone)]
struct Context {
    browser: BrowserHandle,
    tabs: Tabs,
    /// The environment's end.
    ended: CancellationToken,
    tasks: TaskTracker,
    failure: watch::Sender<Option<String>>,
    /// Tells the live view that what it shows changed.
    changes: watch::Sender<u64>,
}

impl Context {
    async fn set_up(&self, page: Page, id: TabId, created_by: BrowserCreatedBy) -> Result<BrowserTab> {
        let ended = self.ended.child_token();
        let tab = async {
            let state = TabState::observe(
                &page,
                &self.browser,
                ended.clone(),
                &self.tasks,
                self.failure.clone(),
                self.changes.clone(),
            )
            .await?;
            let tab = BrowserTab::new(
                page,
                self.browser.clone(),
                self.tabs.clone(),
                ended.clone(),
                state,
                id,
                created_by,
            );
            // Every page starts at the unwatched viewport; its window must hold it.
            tab.set_viewport(super::viewport::UNWATCHED).await?;
            Ok(tab)
        }
        .await;
        if tab.is_err() {
            // The observers started for the page end with it.
            ended.cancel();
        }
        tab
    }

    /// Sends the owner what a task of its found. The owner is gone only
    /// once the environment ended, when nobody needs it.
    async fn tell(&self, request: Request) {
        let _ended = self.tabs.requests.send(request).await;
    }
}

struct Events {
    created: EventStream<EventTargetCreated>,
    attached: EventStream<EventAttachedToTarget>,
    changed: EventStream<EventTargetInfoChanged>,
    destroyed: EventStream<EventTargetDestroyed>,
}

/// Starts the registry of the environment `browser` connects to. It
/// subscribes before it returns, so it sees every tab created after.
pub(super) async fn start(
    browser: BrowserHandle,
    ended: CancellationToken,
    tasks: &TaskTracker,
    failure: watch::Sender<Option<String>>,
    changes: watch::Sender<u64>,
) -> Result<Tabs> {
    let events = {
        let call = browser.call()?;
        Events {
            created: call.event_listener_with_capacity(EVENTS).await?,
            attached: call.event_listener_with_capacity(EVENTS).await?,
            changed: call.event_listener_with_capacity(EVENTS).await?,
            destroyed: call.event_listener_with_capacity(EVENTS).await?,
        }
    };
    let (requests, receiver) = mpsc::channel(REQUESTS);
    let (publish, snapshot) = watch::channel(Arc::new(Snapshot {
        tabs: Vec::new(),
        registering: false,
    }));
    let tabs = Tabs {
        requests,
        snapshot,
        emptied: CancellationToken::new(),
    };
    let owner = Owner {
        context: Context {
            browser,
            tabs: tabs.clone(),
            ended,
            tasks: tasks.clone(),
            failure,
            changes,
        },
        book: Book::new(),
        closing: HashMap::new(),
        popups: Vec::new(),
        holds: watch::channel(0).0,
        publish,
    };
    tasks.spawn(owner.run(receiver, events));
    Ok(tabs)
}

struct Owner {
    context: Context,
    book: Book<BrowserTab>,
    /// The close requests waiting for their tab to go.
    closing: HashMap<TargetId, oneshot::Sender<Result<Closed>>>,
    /// The popup queries waiting for a popup to be registered.
    popups: Vec<(TargetId, oneshot::Sender<Result<Vec<TabId>>>)>,
    /// Batches of temporary tabs in progress; their holds count down.
    holds: watch::Sender<usize>,
    publish: watch::Sender<Arc<Snapshot>>,
}

impl Owner {
    async fn run(mut self, mut requests: mpsc::Receiver<Request>, mut events: Events) {
        let ended = self.context.ended.clone();
        let mut holds = self.holds.subscribe();
        loop {
            // Earlier kinds of a target's events go first, so its creation
            // is usually handled before its destruction; the book's
            // destroyed list covers the rest.
            tokio::select! {
                biased;
                _ = ended.cancelled() => break,
                event = events.created.next() => match event {
                    Some(Ok(event)) => self.found(&event.target_info),
                    Some(Err(lost)) => self.lost(lost).await,
                    None => break,
                },
                event = events.attached.next() => match event {
                    Some(Ok(event)) => {
                        self.found(&event.target_info);
                        self.adopt(&event.target_info.target_id).await;
                    }
                    Some(Err(lost)) => self.lost(lost).await,
                    None => break,
                },
                event = events.changed.next() => match event {
                    Some(Ok(event)) => self.found(&event.target_info),
                    Some(Err(lost)) => self.lost(lost).await,
                    None => break,
                },
                event = events.destroyed.next() => match event {
                    Some(Ok(event)) => self.gone(&event.target_id),
                    Some(Err(lost)) => self.lost(lost).await,
                    None => break,
                },
                changed = holds.changed() => match changed {
                    Ok(()) => self.settle(),
                    Err(_) => break,
                },
                request = requests.recv() => match request {
                    Some(request) => self.request(request).await,
                    None => break,
                },
            }
            self.answer_popups();
        }
    }

    /// Answers each popup query whose popups are all registered now.
    fn answer_popups(&mut self) {
        for (opener, reply) in std::mem::take(&mut self.popups) {
            // A command that left needs no answer.
            if reply.is_closed() {
                continue;
            }
            match self.book.popups(&opener) {
                Some(popups) => {
                    let _left = reply.send(Ok(popups));
                }
                None => self.popups.push((opener, reply)),
            }
        }
    }

    async fn request(&mut self, request: Request) {
        match request {
            Request::Create { created_by, reply } => {
                if !self.book.admit() {
                    let _gone = reply.send(Err(BrowserError::Closed));
                    return;
                }
                let context = self.context.clone();
                self.context.tasks.spawn(async move {
                    let page = tokio::select! {
                        _ = context.ended.cancelled() => Err(BrowserError::Closed),
                        page = async { Ok(context.browser.call()?.new_page("about:blank").await?) } => page,
                    };
                    context
                        .tell(Request::Opened {
                            page,
                            created_by,
                            reply,
                        })
                        .await;
                });
            }
            Request::Opened {
                page,
                created_by,
                reply,
            } => match page {
                Ok(page) => {
                    let target = page.target_id().clone();
                    self.set_up(target, page, created_by, Some(reply));
                }
                Err(error) => {
                    self.book.failed();
                    self.settle();
                    let _gone = reply.send(Err(error));
                }
            },
            Request::Ready { target, tab, reply } => self.ready(target, tab, reply),
            Request::Close {
                target,
                deadline,
                reply,
            } => self.close(target, deadline, reply).await,
            Request::NotClosed { target, error } => {
                self.book.not_closed(&target);
                if let Some(closing) = self.closing.remove(&target) {
                    let _gone = closing.send(Err(error));
                }
            }
            Request::Hold { reply } => {
                if self.book.sealed {
                    let _gone = reply.send(Err(BrowserError::Closed));
                    return;
                }
                self.holds.send_modify(|holds| *holds += 1);
                let _gone = reply.send(Ok(Hold {
                    holds: self.holds.clone(),
                }));
            }
            Request::Settled { reply } => {
                self.settle();
                let _gone = reply.send(Ok(self.book.sealed));
            }
            Request::OpenedBy { opener, reply } => {
                let _left = reply.send(Ok(self.book.opened(&opener)));
            }
            // Answered once no page the opener opened is being registered.
            Request::Popups { opener, reply } => self.popups.push((opener, reply)),
        }
    }

    fn found(&mut self, info: &TargetInfo) {
        if let Some(seen) = Sighting::of(info)
            && self.book.found(&seen)
        {
            self.publish();
        }
    }

    /// Sets up the popup `target` once Chrome attached it.
    async fn adopt(&mut self, target: &TargetId) {
        if !self.book.is_pending(target) {
            return;
        }
        let page = match self.context.browser.call() {
            Ok(call) => call.get_page(target.clone()).await.ok(),
            Err(_) => None,
        };
        // Not attached yet: its attachment brings it back here.
        let Some(page) = page else { return };
        let opener = match self.book.opener(target) {
            Some(opener) => Some(opener.clone()),
            None => page.opener_id().clone(),
        };
        // One of the registry's own creations, which its creator sets up.
        let Some(opener) = opener else { return };
        let created_by = BrowserCreatedBy::Page {
            opener: self.book.opened_by(target, opener),
        };
        self.set_up(target.clone(), page, created_by, None);
    }

    fn set_up(
        &mut self,
        target: TargetId,
        page: Page,
        created_by: BrowserCreatedBy,
        reply: Option<oneshot::Sender<Result<BrowserTab>>>,
    ) {
        let id = self.book.set_up(&target);
        self.publish();
        let context = self.context.clone();
        self.context.tasks.spawn(async move {
            let tab = tokio::select! {
                _ = context.ended.cancelled() => Err(BrowserError::Closed),
                tab = context.set_up(page, id, created_by) => tab,
            };
            context.tell(Request::Ready { target, tab, reply }).await;
        });
    }

    fn ready(
        &mut self,
        target: TargetId,
        tab: Result<BrowserTab>,
        reply: Option<oneshot::Sender<Result<BrowserTab>>>,
    ) {
        let refused = self
            .book
            .ready(&target, reply.is_some(), tab.as_ref().ok().cloned());
        let answer = match (tab, refused) {
            (Ok(tab), None) => Ok(tab),
            (Ok(tab), Some(refused)) => {
                tab.ended.cancel();
                Err(refused)
            }
            (Err(error), _) => Err(error),
        };
        // Published before the creator hears of it, so its next lookup finds it.
        self.publish();
        self.settle();
        match (answer, reply) {
            (Ok(tab), Some(reply)) => {
                // Its creator left before it was ready: a tab nobody asked
                // for does not stay.
                if reply.send(Ok(tab)).is_err()
                    && let Some(tab) = self.book.start_closing(&target)
                {
                    self.start_close(target, tab, Instant::now() + CONTROL_TIMEOUT, None);
                }
            }
            (Ok(_), None) => {}
            (Err(error), Some(reply)) => {
                let _gone = reply.send(Err(error));
                // A blank page its creator cannot use goes again.
                let browser = self.context.browser.clone();
                self.context.tasks.spawn(async move {
                    let closed = async {
                        browser
                            .call()?
                            .execute(CloseTargetParams::new(target))
                            .await?;
                        Ok::<_, BrowserError>(())
                    };
                    let closed = tokio::time::timeout(CONTROL_TIMEOUT, closed)
                        .await
                        .map_err(|_| BrowserError::Timeout)
                        .and_then(std::convert::identity);
                    if let Err(error) = closed {
                        tracing::warn!("could not close a browser tab that failed to set up: {error}");
                    }
                });
            }
            (Err(error), None) => {
                tracing::warn!("a page's new browser tab could not be set up: {error}");
            }
        }
    }

    async fn close(
        &mut self,
        target: TargetId,
        deadline: Instant,
        reply: oneshot::Sender<Result<Closed>>,
    ) {
        if self.book.sealed {
            let _gone = reply.send(Err(BrowserError::Closed));
            return;
        }
        if self.book.closable(&target) && self.book.only(&target, *self.holds.borrow()) {
            // Chrome may have a tab the owner has not heard of yet, such as
            // a popup this one just opened.
            self.reconcile().await;
            if self.book.closable(&target) && self.book.only(&target, *self.holds.borrow()) {
                self.book.sealed = true;
                self.context.tabs.emptied.cancel();
                let _gone = reply.send(Ok(Closed::Environment));
                return;
            }
        }
        let Some(tab) = self.book.start_closing(&target) else {
            let _gone = reply.send(Err(BrowserError::TabNotFound));
            return;
        };
        self.start_close(target, tab, deadline, Some(reply));
    }

    /// Ends the tab's commands and asks Chrome to close it; the tab leaves
    /// the registry once Chrome destroyed it. The book already marks it
    /// closing.
    fn start_close(
        &mut self,
        target: TargetId,
        tab: BrowserTab,
        deadline: Instant,
        reply: Option<oneshot::Sender<Result<Closed>>>,
    ) {
        if let Some(reply) = reply {
            self.closing.insert(target.clone(), reply);
        }
        tab.ended.cancel();
        let context = self.context.clone();
        self.context.tasks.spawn(async move {
            let closed = tokio::select! {
                _ = context.ended.cancelled() => Ok(()),
                closed = close_target(&context.browser, &tab, deadline) => closed,
            };
            if let Err(error) = closed {
                context.tell(Request::NotClosed { target, error }).await;
            }
        });
    }

    fn gone(&mut self, target: &TargetId) {
        let (tab, changed) = self.book.gone(target);
        // The tab's debugging connections end with it.
        if let Some(tab) = tab {
            tab.ended.cancel();
        }
        if changed {
            self.publish();
        }
        self.settle();
        if let Some(closing) = self.closing.remove(target) {
            let _gone = closing.send(Ok(Closed::Tab));
        }
    }

    /// A target event buffer overflowed: the events it lost cost one
    /// reconciliation (`browser.md` § One tab registry).
    async fn lost(&mut self, lost: EventStreamError) {
        tracing::info!("the browser tab registry reconciles: {lost}");
        self.reconcile().await;
    }

    /// Rebuilds the registry from Chrome's target list, before the next
    /// listing.
    async fn reconcile(&mut self) {
        self.book.reconciling = true;
        self.publish();
        let targets = async {
            Ok::<_, BrowserError>(
                self.context
                    .browser
                    .call()?
                    .execute(GetTargetsParams::default())
                    .await?
                    .result
                    .target_infos,
            )
        }
        .await;
        match targets {
            Ok(targets) => {
                let mut present = HashSet::new();
                for seen in targets.iter().filter_map(Sighting::of) {
                    present.insert(seen.target.clone());
                    self.book.found(&seen);
                }
                for target in self.book.vanished(&present) {
                    self.gone(&target);
                }
                for target in self.book.pending() {
                    self.adopt(&target).await;
                }
            }
            // Only an ending browser has no target list; its end follows.
            Err(error) => tracing::debug!("the browser tab registry could not reconcile: {error}"),
        }
        self.book.reconciling = false;
        self.publish();
    }

    /// Seals the registry once it has no tab left, none being created and no
    /// batch held; the environment's owner then retires it.
    fn settle(&mut self) {
        if self.book.settle(*self.holds.borrow()) {
            self.context.tabs.emptied.cancel();
        }
    }

    fn publish(&mut self) {
        let (tabs, registering) = self.book.listing();
        let tabs = tabs
            .into_iter()
            .map(|(tab, title, url)| Listed { tab, title, url })
            .collect();
        self.publish
            .send_replace(Arc::new(Snapshot { tabs, registering }));
        self.context.changes.send_modify(|revision| *revision += 1);
    }
}

/// Stops the tab's loading and asks Chrome to destroy it, without running its
/// `beforeunload` hooks.
async fn close_target(browser: &BrowserHandle, tab: &BrowserTab, deadline: Instant) -> Result<()> {
    // A debugger paused in the page lets go before the page closes.
    let cleanup = tab.state.debug.release().await;
    let closed = tokio::time::timeout_at(deadline, async {
        loop {
            match tab.page.execute(StopLoadingParams {}).await {
                Ok(_) => break,
                // A navigation briefly replaces the active renderer. Wait for
                // its session before stopping the load and closing it.
                Err(chromiumoxide::error::CdpError::Chrome(error))
                    if error.message == "Not attached to an active page" =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => return Err(BrowserError::from(error)),
            }
        }
        browser
            .call()?
            .execute(CloseTargetParams::new(tab.page.target_id().clone()))
            .await?;
        Ok(())
    })
    .await
    .map_err(|_| BrowserError::Timeout)
    .and_then(std::convert::identity);
    after_cleanup(closed, cleanup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str) -> TargetId {
        TargetId::new(name)
    }

    fn page<'a>(target: &'a TargetId, opener: Option<&'a TargetId>) -> Sighting<'a> {
        Sighting {
            target,
            title: "",
            url: "about:blank",
            opener,
        }
    }

    /// A book with `names` as live tabs, as the registry's own creations.
    fn live(names: &[&'static str]) -> Book<&'static str> {
        let mut book = Book::new();
        for name in names {
            assert!(book.admit());
            book.set_up(&target(name));
            assert!(book.ready(&target(name), true, Some(*name)).is_none());
        }
        book
    }

    fn listed(book: &Book<&'static str>) -> Vec<&'static str> {
        book.listing().0.into_iter().map(|(tab, _, _)| tab).collect()
    }

    #[test]
    fn an_openers_popups_are_answered_once_registered() {
        let mut book = live(&["a"]);
        let opener = target("a");
        assert_eq!(book.popups(&opener), Some(Vec::new()));
        let popup = target("popup");
        assert!(!book.found(&page(&popup, Some(&opener))));
        // Known, but not yet operable: the answer waits, and the page counts
        // as opened already.
        assert_eq!(book.popups(&opener), None);
        assert_eq!(book.opened(&opener), vec![book.public_id(&popup)]);
        book.set_up(&popup);
        assert_eq!(book.popups(&opener), None);
        assert!(book.ready(&popup, false, Some("popup")).is_none());
        let id = book.public_id(&popup);
        assert_eq!(book.popups(&opener), Some(vec![id.clone()]));
        // A popup that could not be set up is not one the opener can name.
        let broken = target("broken");
        book.found(&page(&broken, Some(&opener)));
        book.set_up(&broken);
        assert!(book.ready(&broken, false, None).is_none());
        assert_eq!(book.popups(&opener), Some(vec![id]));
    }

    #[test]
    fn a_page_destroyed_before_its_creation_is_handled_stays_out() {
        let mut book = live(&["a"]);
        let popup = target("popup");
        let (gone, changed) = book.gone(&popup);
        assert!(gone.is_none() && !changed);
        // Its creation and attachment arrive late, on their own buffers.
        assert!(!book.found(&page(&popup, Some(&target("a")))));
        assert!(!book.is_pending(&popup));
        assert!(matches!(
            book.ready(&popup, false, Some("popup")),
            Some(BrowserError::TabNotFound)
        ));
        assert_eq!(listed(&book), ["a"]);
    }

    #[test]
    fn the_last_tab_waits_for_creations_batches_and_popups() {
        let a = target("a");
        let mut book = live(&["a"]);
        assert!(book.only(&a, 0));
        assert!(!book.only(&a, 1), "a batch of temporary tabs holds the environment");
        assert!(book.admit());
        assert!(!book.only(&a, 0), "a creation in flight holds it");
        book.failed();
        assert!(book.only(&a, 0));
        // A page without an opener that no creation asked for does not count.
        book.found(&page(&target("stray"), None));
        assert!(book.only(&a, 0));
        // A popup Chrome has not attached yet does.
        book.found(&page(&target("popup"), Some(&a)));
        assert!(!book.only(&a, 0));
    }

    #[test]
    fn the_registry_seals_once_when_its_last_tab_goes() {
        let a = target("a");
        let mut book = live(&["a"]);
        assert!(!book.settle(0));
        assert_eq!(book.gone(&a).0, Some("a"));
        assert!(!book.settle(1), "a held batch keeps it open");
        assert!(book.settle(0));
        assert!(!book.settle(0), "it seals only once");
        assert!(!book.admit(), "a sealed registry creates no tab");
    }

    #[test]
    fn a_tab_readied_after_sealing_is_refused() {
        let mut book = live(&["a"]);
        assert!(book.admit());
        book.set_up(&target("late"));
        book.gone(&target("a"));
        book.sealed = true;
        assert!(matches!(
            book.ready(&target("late"), true, Some("late")),
            Some(BrowserError::Closed)
        ));
        assert!(listed(&book).is_empty());
        assert_eq!(book.creating, 0);
    }

    #[test]
    fn a_listing_waits_for_tabs_being_set_up_and_lists_in_creation_order() {
        let mut book = live(&["first"]);
        let popup = target("popup");
        book.found(&page(&popup, Some(&target("first"))));
        book.set_up(&popup);
        let (tabs, registering) = book.listing();
        assert!(registering);
        assert_eq!(tabs.len(), 1);
        book.ready(&popup, false, Some("popup"));
        assert!(!book.listing().1);
        assert_eq!(listed(&book), ["first", "popup"]);
    }

    #[test]
    fn an_opener_keeps_its_id_after_it_closes() {
        let mut book = live(&["opener"]);
        let opener = target("opener");
        let popup = target("popup");
        let id = book.public_id(&opener);
        book.found(&page(&popup, Some(&opener)));
        book.gone(&opener);
        // Chrome may drop the opener from later target information.
        book.found(&page(&popup, None));
        assert_eq!(book.opener(&popup), Some(&opener));
        assert_eq!(book.opened_by(&popup, opener.clone()), id);
    }

    #[test]
    fn reconciling_finds_the_pages_events_missed_and_drops_the_vanished() {
        let mut book = live(&["a", "b"]);
        let popup = target("popup");
        let present: HashSet<_> = [target("a"), popup.clone()].into();
        book.found(&page(&popup, Some(&target("a"))));
        assert_eq!(book.vanished(&present), [target("b")]);
        assert_eq!(book.pending(), [popup]);
    }

    #[test]
    fn a_closing_tab_is_closed_once_until_chrome_refuses() {
        let a = target("a");
        let mut book = live(&["a", "b"]);
        assert_eq!(book.start_closing(&a), Some("a"));
        assert!(!book.closable(&a));
        assert_eq!(book.start_closing(&a), None);
        book.not_closed(&a);
        assert!(book.closable(&a));
    }
}
