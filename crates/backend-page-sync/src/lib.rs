//! The registry of each user's open synchronization channels, a shared service.
//!
//! Whoever commits a change that a page shows marks the changed part in the
//! registry, after the commit and from any thread. Some changes come from
//! outside the user's shard: a shared instance's provider entries serve every
//! user, so a change of one marks the channels of every user.
//!
//! A mark does two things:
//!
//! - It adds the part to the set of each of the user's channels.
//! - It wakes the channel's task, which reads the part in the shard when it can
//!   send it.
//!
//! A channel holds at most one mark per part however far its page falls behind,
//! so it never queues. A part of the backend that follows a user's changes as a
//! page does, such as the search index, watches them the same way.
//!
//! A channel is registered under the hash of the session it opened with, a
//! record of `backend-database`, so a sign-out ends that session's channels.
//!
//! # The product state's version
//!
//! Each user has one version of their product state, `{ epoch, revision }`,
//! and a log of the parts the last [`RECENT_CHANGES`] changes marked.
//! A mark adds one to the revision and logs the parts it marked, in one step
//! with the marks of the user's channels, so a channel that follows the state
//! from a revision learns which parts changed since, or that the log no longer
//! reaches it.
//!
//! The log is in this registry because a mark comes from any thread. A user's
//! entry is made by the first read of the version, and freed with the user's
//! shard ([`SyncRegistry::forget`]); a later entry has another epoch, which
//! no version of the earlier one matches. The log holds at most
//! [`RECENT_CHANGES`] sets of part names for each user, a few hundred bytes
//! each at most, so some hundred kibibytes in the worst case.
//!
//! The registry reads and builds no product state: `backend-user-shard` does.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use demi_backend_database::accounts::TokenHash;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use tokio::sync::Notify;

/// A part of the product state that a page shows. A channel sends the parts
/// that changed in this order, so a new conversation's summary goes before
/// the order that places it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// The conversation's sidebar entry and its metadata: a channel sends the
    /// entry, and the conversation's sockets its metadata.
    Conversation(ConversationId),
    /// The conversation's metadata or its draft alone, which no sidebar
    /// entry shows: only the conversation's sockets hear of it.
    Metadata(ConversationId),
    /// The order of every conversation.
    ConversationOrder,
    Preferences,
    User,
    Workspaces,
    Devices,
    Providers,
    /// The user's sign-ins; after the entries, so that a completed login's
    /// account is listed when the page learns it completed.
    Logins,
    Cloud,
    /// The Subagents switch, the limits and the subagent profiles.
    Subagents,
    /// The personal instructions.
    Instructions,
    /// The user's credentials, without their values.
    Credentials,
    /// The user's port forwards with what their runners announced.
    PortForwards,
    /// The user's exposed ports.
    ExposedPorts,
    /// The user's passkeys.
    Passkeys,
    /// The user's sessions, the signed-in browsers.
    Sessions,
    /// Every account, which only the channels of the master and admins send.
    Users,
    /// Settings › Updates, which only the channels of the master and
    /// admins send.
    ServerUpgrade,
    /// Whether every user uses the master's providers, which every channel
    /// sends.
    Instance,
    /// The backend stops for an update of the server: every page shows the
    /// update's screen.
    Update,
    /// The plugin list: whether the user has each plugin on.
    Plugins,
    /// A plugin's state, by the plugin's id.
    Plugin(String),
    /// The catalog of the entries' models, which the providers and the
    /// preferences decide too: a channel sends it after them, last, since
    /// a source's first read may take a while.
    Models,
}

/// How many changes of the product state a user's log keeps the parts of.
///
/// A cache: a page that falls behind it reads again, so the log's memory is
/// bounded and nothing is lost.
pub const RECENT_CHANGES: usize = 1000;

/// The version of a user's product state: the log's epoch, and how many
/// changes it counted since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateVersion {
    pub epoch: String,
    pub revision: u64,
}

/// Why a channel cannot follow the state from the version a page holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stale;

/// A user's product state's version and the parts its latest changes marked.
struct StateLog {
    epoch: String,
    revision: u64,
    /// The parts the change at each revision marked, the latest last; the
    /// last entry is the change at `revision`.
    recent: VecDeque<Vec<Part>>,
}

impl StateLog {
    fn new() -> Self {
        static MADE: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        Self {
            epoch: format!("{nanos:x}-{}", MADE.fetch_add(1, Ordering::Relaxed)),
            revision: 0,
            recent: VecDeque::new(),
        }
    }

    fn version(&self) -> StateVersion {
        StateVersion {
            epoch: self.epoch.clone(),
            revision: self.revision,
        }
    }

    /// Counts one change that marked `parts`.
    fn record(&mut self, parts: Vec<Part>) {
        self.revision += 1;
        self.recent.push_back(parts);
        if self.recent.len() > RECENT_CHANGES {
            self.recent.pop_front();
        }
    }

    /// The parts the changes after `held` marked, or `Stale` when the log
    /// does not reach `held`.
    fn since(&self, held: &StateVersion) -> Result<BTreeSet<Part>, Stale> {
        if held.epoch != self.epoch || held.revision > self.revision {
            return Err(Stale);
        }
        let missed = usize::try_from(self.revision - held.revision).map_err(|_| Stale)?;
        if missed > self.recent.len() {
            return Err(Stale);
        }
        Ok(self.recent.iter().skip(self.recent.len() - missed).flatten().cloned().collect())
    }
}

/// What the registry keeps of one user.
struct UserEntry {
    log: StateLog,
    channels: Vec<Arc<Channel>>,
}

impl UserEntry {
    fn new() -> Self {
        Self {
            log: StateLog::new(),
            channels: Vec::new(),
        }
    }
}

/// Every user's open channels and product state log. Cloning it shares it.
#[derive(Clone, Default)]
pub struct SyncRegistry(Arc<Mutex<HashMap<UserId, UserEntry>>>);

/// One open channel as the registry marks it.
struct Channel {
    /// The session a page connection's watch follows the end of; none for
    /// any other watch, which no sign-out ends.
    session: Option<TokenHash>,
    /// The parts it follows.
    follows: Follows,
    marked: Mutex<Marked>,
    /// Woken by each mark; a mark while the task is busy leaves a permit, so
    /// none is missed.
    wake: Notify,
}

/// The parts a channel follows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Follows {
    Every,
    Conversation(ConversationId),
    /// No part: a page connection's watch of its session alone.
    Nothing,
}

/// What was marked on a channel since its task last took it.
#[derive(Debug, Default)]
pub struct Marked {
    pub parts: BTreeSet<Part>,
    /// The revision of the latest change of the product state marked here:
    /// the state is current as of it once the parts are sent.
    pub revision: Option<u64>,
    /// The session a page connection's watch follows was signed out.
    pub session_ended: bool,
}

impl Marked {
    fn is_empty(&self) -> bool {
        self.parts.is_empty() && self.revision.is_none() && !self.session_ended
    }
}

impl Channel {
    fn lock(&self) -> MutexGuard<'_, Marked> {
        // A section only inserts into the set or takes it, so a poisoned
        // one is whole.
        self.marked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Marks `parts`, the change at `revision`, on the channel if it follows
    /// them.
    fn mark(&self, parts: &[Part], revision: u64) {
        let mut marked = self.lock();
        let mut any = false;
        for part in parts {
            let follows = match &self.follows {
                Follows::Every => !matches!(part, Part::Metadata(_)),
                Follows::Conversation(id) => {
                    matches!(part, Part::Conversation(changed) | Part::Metadata(changed) if changed == id)
                }
                Follows::Nothing => false,
            };
            if follows {
                marked.parts.insert(part.clone());
                any = true;
            }
        }
        if any {
            if self.follows == Follows::Every {
                marked.revision = Some(revision);
            }
            drop(marked);
            self.wake.notify_one();
        }
    }
}

impl SyncRegistry {
    fn lock(&self) -> MutexGuard<'_, HashMap<UserId, UserEntry>> {
        // A section only adds, removes or visits channels, so a poisoned map
        // is whole.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a watch of `user`'s changes, for a page's synchronization
    /// channel or a part of the backend that follows them as a page does,
    /// such as the search index, until the registration is dropped.
    pub fn watch(&self, user: &UserId) -> Registration {
        self.add(user, None, Follows::Every)
    }

    /// Registers a watch of `user`'s conversation `conversation` for a lane of
    /// it: the watch's `marked` returns once the conversation's part is
    /// marked, and for nothing else.
    pub fn watch_conversation(&self, user: &UserId, conversation: ConversationId) -> Registration {
        self.add(user, None, Follows::Conversation(conversation))
    }

    /// Registers a watch of the session `session` of `user` for a page
    /// connection opened with it: the watch's `marked` returns once the
    /// session is signed out, and for nothing else.
    pub fn watch_session(&self, user: &UserId, session: TokenHash) -> Registration {
        self.add(user, Some(session), Follows::Nothing)
    }

    fn add(&self, user: &UserId, session: Option<TokenHash>, follows: Follows) -> Registration {
        let channel = Arc::new(Channel {
            session,
            follows,
            marked: Mutex::default(),
            wake: Notify::new(),
        });
        self.lock()
            .entry(user.clone())
            .or_insert_with(UserEntry::new)
            .channels
            .push(channel.clone());
        Registration {
            registry: self.clone(),
            user: user.clone(),
            channel,
        }
    }

    /// The marks of `user`'s changes, for a part of the user's shard that
    /// holds no shard.
    pub fn of(&self, user: &UserId) -> UserMarks {
        UserMarks {
            registry: self.clone(),
            user: user.clone(),
        }
    }

    /// The version of `user`'s product state now, which a read of the state
    /// takes before it reads, so a change made while it reads is a change
    /// after the version.
    pub fn version(&self, user: &UserId) -> StateVersion {
        self.lock().entry(user.clone()).or_insert_with(UserEntry::new).log.version()
    }

    /// Frees what the registry keeps of `user` once nothing follows them: the
    /// shard of `user` closed.
    pub fn forget(&self, user: &UserId) {
        let mut users = self.lock();
        if users.get(user).is_some_and(|entry| entry.channels.is_empty()) {
            users.remove(user);
        }
    }

    /// Marks `part` changed on each open channel of `user`.
    pub fn mark(&self, user: &UserId, part: Part) {
        Self::mark_entry(self.lock().get_mut(user), part);
    }

    /// Marks `part` changed on every user's open channels, for a change
    /// every user sees, such as a shared instance's provider entry.
    pub fn mark_everyone(&self, part: &Part) {
        for entry in self.lock().values_mut() {
            Self::mark_entry(Some(entry), part.clone());
        }
    }

    /// Counts the change that marked `part` in the log of `entry`, and marks
    /// its channels, in one step. A user nothing reads or follows has no
    /// entry, and nothing to mark.
    fn mark_entry(entry: Option<&mut UserEntry>, part: Part) {
        let Some(entry) = entry else {
            return;
        };
        let mut parts = vec![part];
        // The catalog lists the entries' models, and Automatic reads the
        // preferences: a change of either is one of the catalog too.
        if matches!(parts[0], Part::Providers | Part::Preferences) {
            parts.push(Part::Models);
        }
        // A conversation's metadata or draft alone is no change of the state.
        if !matches!(parts[0], Part::Metadata(_)) {
            entry.log.record(parts.clone());
        }
        for channel in &entry.channels {
            channel.mark(&parts, entry.log.revision);
        }
    }

    /// Ends the page connections of `user` that opened with the session
    /// `session`, which was signed out.
    pub fn end_session(&self, user: &UserId, session: &TokenHash) {
        if let Some(entry) = self.lock().get(user) {
            for channel in entry
                .channels
                .iter()
                .filter(|channel| channel.session.as_ref() == Some(session))
            {
                channel.lock().session_ended = true;
                channel.wake.notify_one();
            }
        }
    }
}

/// The marks of one user's changes. Cloning it is cheap.
#[derive(Clone)]
pub struct UserMarks {
    registry: SyncRegistry,
    user: UserId,
}

impl UserMarks {
    /// Marks `part` changed on the user's open channels.
    pub fn mark(&self, part: Part) {
        self.registry.mark(&self.user, part);
    }
}

/// The user's pages as the plugin host tells them of its changes
/// (`crates/plugin-host/src/store.rs`).
impl demi_plugin_host::store::PluginChanges for UserMarks {
    fn plugins_changed(&self) {
        self.mark(Part::Plugins);
    }

    fn plugin_changed(&self, plugin: String) {
        self.mark(Part::Plugin(plugin));
    }

    fn conversation_changed(&self, conversation: ConversationId) {
        self.mark(Part::Conversation(conversation));
    }

    fn metadata_changed(&self, conversation: ConversationId) {
        self.mark(Part::Metadata(conversation));
    }
}

/// A channel's place in the registry, which dropping it gives up.
pub struct Registration {
    registry: SyncRegistry,
    user: UserId,
    channel: Arc<Channel>,
}

impl Registration {
    /// Waits until something is marked on the channel, and takes nothing.
    pub async fn marked(&self) {
        loop {
            if !self.channel.lock().is_empty() {
                return;
            }
            self.channel.wake.notified().await;
        }
    }

    /// Marks, in one step with the log, the parts that changed after `held`,
    /// and the revision they bring the state to, which a page that follows
    /// the state from `held` has not seen; `Stale` when the log does not
    /// reach `held`.
    pub fn follow_since(&self, held: &StateVersion) -> Result<(), Stale> {
        let users = self.registry.lock();
        let Some(entry) = users.get(&self.user) else {
            return Err(Stale);
        };
        let parts = entry.log.since(held)?;
        if entry.log.revision > held.revision {
            let mut marked = self.channel.lock();
            marked.parts.extend(parts);
            marked.revision = Some(entry.log.revision);
            drop(marked);
            self.channel.wake.notify_one();
        }
        Ok(())
    }

    /// Marks `part` changed on this channel alone, for a list the page
    /// follows that is to be read and compared.
    pub fn mark_part(&self, part: Part) {
        self.channel.lock().parts.insert(part);
        self.channel.wake.notify_one();
    }

    /// Takes what was marked since the last take.
    pub fn take(&self) -> Marked {
        std::mem::take(&mut *self.channel.lock())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut users = self.registry.lock();
        if let Some(entry) = users.get_mut(&self.user) {
            entry.channels.retain(|channel| !Arc::ptr_eq(channel, &self.channel));
        }
    }
}

/// Counts, for each conversation, how many times one of its parts changed since
/// its shard started, such as its permission requests.
///
/// The conversation's metadata carries the count. A conversation without an
/// entry has changed none. The counts live on the user's shard, which nothing
/// else reaches.
#[derive(Default)]
pub struct Revisions(RefCell<HashMap<ConversationId, u64>>);

impl Revisions {
    pub fn of(&self, conversation: &ConversationId) -> u64 {
        self.0.borrow().get(conversation).copied().unwrap_or(0)
    }

    /// Counts a change of `conversation`'s part.
    pub fn raise(&self, conversation: &ConversationId) {
        *self.0.borrow_mut().entry(conversation.clone()).or_default() += 1;
    }
}
