//! What the backend and a runner both apply to a page's work on a Host's
//! files, so that the relay's routes and a direct channel answer alike
//! (`crates/runner-protocol/src/direct.rs`): a listing's entries,
//! the paths a delete keeps, how a failed file operation reads to the page,
//! and the page's file watch: its messages, its limits and the
//! subscriptions it holds on the Host for them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use demi_shared_types::{MAX_SAFE_INTEGER, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use typed_path::Utf8TypedPath;

/// One entry of a listing, with its own size and time, as `lstat` reports
/// them. A symbolic link is no directory itself and carries `link`, where
/// it points and what is there. An entry that disappears while the directory is
/// listed is left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
#[garde(allow_unvalidated)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_directory: bool,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub size: u64,
    pub modified_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<crate::wire::SymbolicLink>,
}

impl DirectoryEntry {
    /// A runner's listing entry as the page lists it; none for a time out
    /// of range.
    pub fn from_wire(entry: crate::wire::DirEntry) -> Option<Self> {
        Some(Self {
            name: entry.name,
            is_directory: entry.is_directory,
            size: entry.size,
            modified_at: Timestamp::from_millisecond(entry.mtime.0).ok()?,
            link: entry.link,
        })
    }
}

/// Whether deleting `path` would take with it a directory the Host needs:
/// a filesystem root, or one of `kept` or a directory holding it. Compared
/// without case, since a Host's filesystem may ignore it.
pub fn protected_path<'a>(path: &str, kept: impl IntoIterator<Item = &'a str>) -> bool {
    let top = Utf8TypedPath::derive(&path.to_lowercase()).normalize();
    if top.parent().is_none() {
        return true;
    }
    kept.into_iter().any(|directory| {
        Utf8TypedPath::derive(&directory.to_lowercase())
            .normalize()
            .starts_with(top.as_str())
    })
}

/// What a file operation's failure is to the page, by the errno-style code
/// the Host gave: nothing at the path, no permission, or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsFailure {
    NotFound,
    Forbidden,
    Failed,
}

impl FsFailure {
    pub fn of(code: Option<&str>) -> Self {
        match code {
            Some("ENOENT") => Self::NotFound,
            Some("EACCES" | "EPERM") => Self::Forbidden,
            _ => Self::Failed,
        }
    }
}

/// What a subscription watches, and so when it changes (`crates/runner-host/src/subscriptions.rs`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionKind {
    /// A file: it is created, written, removed or renamed, its metadata
    /// changes, or a folder above it is removed or renamed.
    File,
    /// A folder's names: an entry comes, goes or is renamed, or the folder
    /// or one above it is removed or renamed.
    FolderNames,
    /// A folder's names and its entries' sizes and times: as `folder_names`,
    /// or an entry's content or metadata changes.
    FolderDetails,
    /// A working tree's changes list: a path git does not ignore changes,
    /// or what the repository's git directory says of it.
    Worktree,
    /// The last commit of a working tree's repository: `HEAD`, a ref or
    /// `packed-refs` changes.
    Head,
}

/// What a subscription is of: its kind and its absolute path. Subscriptions
/// of one subject share one on the Host.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Subject {
    pub kind: SubscriptionKind,
    pub path: String,
}

/// The most subscriptions one page's file watch holds, and the most of them
/// outside the conversation's working tree.
pub const MAX_PAGE_SUBSCRIPTIONS: usize = 4096;
pub const MAX_OUTSIDE_SUBSCRIPTIONS: usize = 64;

/// One subscription a page asks for, under a key of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PageSubscription {
    #[garde(length(chars, min = 1, max = 1024))]
    pub key: String,
    #[garde(skip)]
    pub kind: SubscriptionKind,
    #[garde(length(chars, min = 1), custom(demi_command_protocol::without_nul))]
    pub path: String,
}

/// A message the page sends on its file watch, `WS .../fs/watch` or a
/// direct `watch` channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileWatchRequest {
    /// Subscriptions the page holds from now on.
    Subscribe {
        #[garde(length(max = MAX_PAGE_SUBSCRIPTIONS), dive)]
        subscriptions: Vec<PageSubscription>,
    },
    /// Subscriptions the page no longer holds.
    Unsubscribe {
        #[garde(length(max = MAX_PAGE_SUBSCRIPTIONS))]
        keys: Vec<String>,
    },
    /// Every subscription the page still holds, whose leases run again.
    Renew {
        #[garde(length(max = MAX_PAGE_SUBSCRIPTIONS))]
        keys: Vec<String>,
    },
}

/// Why a page's subscription was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefusedReason {
    /// The page holds [`MAX_PAGE_SUBSCRIPTIONS`] subscriptions, or
    /// [`MAX_OUTSIDE_SUBSCRIPTIONS`] outside the conversation's working
    /// tree.
    Limit,
}

/// A Host's number as the page reads it, which stays within JavaScript's
/// safe range: the schema of a subscription's `since` and of a change's
/// `at`.
pub struct HostNumber;

impl JsonSchema for HostNumber {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "HostNumber".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "integer", "minimum": 0, "maximum": MAX_SAFE_INTEGER })
    }
}

/// A message a page's file watch carries to the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileWatchMessage {
    /// The Host watches the subscription: a change at a number above
    /// `since` is reported, numbers of `epoch`.
    Live {
        key: String,
        #[schemars(with = "HostNumber")]
        since: u64,
        epoch: String,
    },
    /// Subscriptions whose subjects changed, each once, as `[key, at]`, `at`
    /// the number of the latest event that changed it.
    Changed {
        #[schemars(with = "Vec<(String, HostNumber)>")]
        changes: Vec<(String, u64)>,
    },
    /// The Host cannot watch the subscription, for this reason; it is over.
    Unavailable { key: String, reason: String },
    /// The subscription is past the page's limits; it is not held.
    Refused { key: String, reason: RefusedReason },
    /// A `renew` arrived; the leases of `keys` run again from it.
    Renewed { keys: Vec<String> },
    /// The Host is out of reach or a stopped Cloud: no subscription is live
    /// until the next `live`.
    Offline,
    /// Nothing else was sent for 30 seconds: the watch is quiet, not dead.
    Heartbeat,
    /// The backend's answer to the page's `{ ping: id }` on its socket, sent
    /// at once; a direct channel's watch, whose far end is the runner, carries
    /// none.
    Pong { id: u32 },
}

impl FileWatchMessage {
    /// The message as JSON texts of at most `limit` bytes each, as a page's
    /// socket or channel carries them: a `changed` naming more keys than
    /// one carries goes in as many as it takes.
    pub fn into_texts(self, limit: usize) -> Vec<String> {
        let text = serde_json::to_string(&self).expect("a watch message serializes");
        let FileWatchMessage::Changed { changes } = self else {
            return vec![text];
        };
        if text.len() <= limit || changes.len() < 2 {
            return vec![text];
        }
        let (first, second) = changes.split_at(changes.len() / 2);
        [first, second]
            .into_iter()
            .flat_map(|half| FileWatchMessage::Changed { changes: half.to_vec() }.into_texts(limit))
            .collect()
    }
}

/// What the page's subscriptions ask of the Host's, and tell the page, as
/// [`PageSubscriptions`] works them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A message for the page.
    Page(FileWatchMessage),
    /// The page holds a subject the Host must watch from now on.
    Hold(Subject),
    /// Nothing the page holds needs the subject any more.
    Release(Subject),
    /// The subjects whose leases the page renewed.
    Renew(Vec<Subject>),
}

/// Where the Host's subscription of a subject is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostState {
    Starting,
    Live { since: u64, epoch: String },
}

/// One subject the page holds, under its keys, or for the subscriptions
/// inside the working tree it anchors.
#[derive(Debug)]
struct Held {
    host: HostState,
    keys: BTreeSet<String>,
    /// The working tree's own subscription, held while anything the page
    /// holds lies inside it.
    anchor: bool,
}

/// One key the page holds: its subject, and when the page's lease of it
/// ends, as the backend counts it.
#[derive(Debug)]
struct Key {
    subject: Subject,
    lease: Instant,
}

/// The subscriptions of one page's file watch, as the backend's relay and a
/// runner's direct `watch` both keep them: the page's keys and their leases, the
/// limits, and the subjects they hold on the Host, each once however many keys hold it.
/// A subject inside the conversation's working tree also holds the working tree's own
/// subscription, so that the Host watches it through the tree's one watch
/// (`crates/runner-host/src/subscriptions.rs`); the page hears of that one only under a
/// key of its own.
///
/// It holds no subscription itself: its owner holds and releases the
/// Host's as the [`Effect`]s say, and hands it their reports.
#[derive(Debug)]
pub struct PageSubscriptions {
    root: Option<String>,
    keys: BTreeMap<String, Key>,
    subjects: HashMap<Subject, Held>,
    /// Keys held outside the working tree.
    outside: usize,
    /// Keys held inside it, which the anchor serves.
    inside: usize,
}

impl PageSubscriptions {
    /// The subscriptions of a page whose conversation works in `root`;
    /// none for a device's folder dialog, all of whose subscriptions are
    /// outside a working tree.
    pub fn new(root: Option<String>) -> Self {
        Self {
            root,
            keys: BTreeMap::new(),
            subjects: HashMap::new(),
            outside: 0,
            inside: 0,
        }
    }

    fn inside(&self, path: &str) -> bool {
        self.root.as_deref().is_some_and(|root| {
            let root = root.trim_end_matches('/');
            path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
        })
    }

    fn anchor(&self) -> Option<Subject> {
        self.root.as_ref().map(|root| Subject {
            kind: SubscriptionKind::Worktree,
            path: root.clone(),
        })
    }

    /// The page subscribes, its leases running from `now`.
    pub fn subscribe(&mut self, subscriptions: Vec<PageSubscription>, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        for PageSubscription { key, kind, path } in subscriptions {
            let subject = Subject { kind, path };
            if let Some(held) = self.keys.get_mut(&key) {
                if held.subject == subject {
                    held.lease = now + crate::wire::SUBSCRIPTION_LEASE;
                    if let Some(Held {
                        host: HostState::Live { since, epoch },
                        ..
                    }) = self.subjects.get(&subject)
                    {
                        effects.push(Effect::Page(FileWatchMessage::Live {
                            key,
                            since: *since,
                            epoch: epoch.clone(),
                        }));
                    }
                    continue;
                }
                self.drop_key(&key, &mut effects);
            }
            let inside = self.inside(&subject.path);
            if self.keys.len() >= MAX_PAGE_SUBSCRIPTIONS
                || (!inside && self.outside >= MAX_OUTSIDE_SUBSCRIPTIONS)
            {
                effects.push(Effect::Page(FileWatchMessage::Refused {
                    key,
                    reason: RefusedReason::Limit,
                }));
                continue;
            }
            if inside {
                self.inside += 1;
                if self.inside == 1 {
                    self.hold_anchor(&mut effects);
                }
            } else {
                self.outside += 1;
            }
            let held = self.subjects.entry(subject.clone()).or_insert_with(|| {
                effects.push(Effect::Hold(subject.clone()));
                Held {
                    host: HostState::Starting,
                    keys: BTreeSet::new(),
                    anchor: false,
                }
            });
            held.keys.insert(key.clone());
            if let HostState::Live { since, epoch } = &held.host {
                effects.push(Effect::Page(FileWatchMessage::Live {
                    key: key.clone(),
                    since: *since,
                    epoch: epoch.clone(),
                }));
            }
            self.keys.insert(
                key,
                Key {
                    subject,
                    lease: now + crate::wire::SUBSCRIPTION_LEASE,
                },
            );
        }
        effects
    }

    fn hold_anchor(&mut self, effects: &mut Vec<Effect>) {
        let Some(anchor) = self.anchor() else {
            return;
        };
        let held = self.subjects.entry(anchor.clone()).or_insert_with(|| {
            effects.push(Effect::Hold(anchor));
            Held {
                host: HostState::Starting,
                keys: BTreeSet::new(),
                anchor: false,
            }
        });
        held.anchor = true;
    }

    /// The page no longer holds `keys`.
    pub fn unsubscribe(&mut self, keys: &[String]) -> Vec<Effect> {
        let mut effects = Vec::new();
        for key in keys {
            self.drop_key(key, &mut effects);
        }
        effects
    }

    /// Lets `key` go, and its subject once nothing holds it.
    fn drop_key(&mut self, key: &str, effects: &mut Vec<Effect>) {
        let Some(Key { subject, .. }) = self.keys.remove(key) else {
            return;
        };
        if self.inside(&subject.path) {
            self.inside -= 1;
            if self.inside == 0
                && let Some(anchor) = self.anchor()
                && let Some(held) = self.subjects.get_mut(&anchor)
            {
                held.anchor = false;
                self.release_unheld(&anchor, effects);
            }
        } else {
            self.outside -= 1;
        }
        if let Some(held) = self.subjects.get_mut(&subject) {
            held.keys.remove(key);
            self.release_unheld(&subject, effects);
        }
    }

    fn release_unheld(&mut self, subject: &Subject, effects: &mut Vec<Effect>) {
        if self
            .subjects
            .get(subject)
            .is_some_and(|held| held.keys.is_empty() && !held.anchor)
        {
            self.subjects.remove(subject);
            effects.push(Effect::Release(subject.clone()));
        }
    }

    /// The page renews `keys` at `now`: those it holds are renewed, which
    /// the page hears, with their subjects on the Host, the working tree's
    /// own among them while anything inside it is.
    pub fn renew(&mut self, keys: &[String], now: Instant) -> Vec<Effect> {
        let mut renewed = Vec::new();
        let mut subjects = BTreeSet::new();
        for key in keys {
            if let Some(held) = self.keys.get_mut(key) {
                held.lease = now + crate::wire::SUBSCRIPTION_LEASE;
                renewed.push(key.clone());
                subjects.insert(held.subject.clone());
            }
        }
        if self.inside > 0
            && let Some(anchor) = self.anchor()
            && self.subjects.contains_key(&anchor)
        {
            subjects.insert(anchor);
        }
        let mut effects = vec![Effect::Page(FileWatchMessage::Renewed { keys: renewed })];
        if !subjects.is_empty() {
            effects.push(Effect::Renew(subjects.into_iter().collect()));
        }
        effects
    }

    /// When the earliest lease the page holds ends.
    pub fn next_expiry(&self) -> Option<Instant> {
        self.keys.values().map(|key| key.lease).min()
    }

    /// Lets go of the keys whose leases ended by `now`: the page stopped
    /// renewing them, and believes them over already.
    pub fn expire(&mut self, now: Instant) -> Vec<Effect> {
        let ended: Vec<String> = self
            .keys
            .iter()
            .filter(|(_, key)| key.lease <= now)
            .map(|(key, _)| key.clone())
            .collect();
        self.unsubscribe(&ended)
    }

    /// The Host watches `subject` from `since`.
    pub fn live(&mut self, subject: &Subject, since: u64, epoch: &str) -> Vec<Effect> {
        let Some(held) = self.subjects.get_mut(subject) else {
            return Vec::new();
        };
        held.host = HostState::Live {
            since,
            epoch: epoch.to_owned(),
        };
        held.keys
            .iter()
            .map(|key| {
                Effect::Page(FileWatchMessage::Live {
                    key: key.clone(),
                    since,
                    epoch: epoch.to_owned(),
                })
            })
            .collect()
    }

    /// The Host cannot watch `subject`: each key of it hears so and is
    /// over. When the working tree's own subscription fails, what lies
    /// inside it is watched alone, as the Host decides, and the tree's is
    /// held again once nothing is inside it and something comes.
    pub fn failed(&mut self, subject: &Subject, reason: &str) -> Vec<Effect> {
        let Some(held) = self.subjects.remove(subject) else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        for key in &held.keys {
            effects.push(Effect::Page(FileWatchMessage::Unavailable {
                key: key.clone(),
                reason: reason.to_owned(),
            }));
            if self.keys.remove(key).is_some() {
                if self.inside(&subject.path) {
                    self.inside -= 1;
                } else {
                    self.outside -= 1;
                }
            }
        }
        if self.inside == 0
            && let Some(anchor) = self.anchor()
            && let Some(held) = self.subjects.get_mut(&anchor)
        {
            held.anchor = false;
            self.release_unheld(&anchor, &mut effects);
        }
        effects
    }

    /// Subjects that changed, each with the number of its latest change:
    /// one message naming each key of them once.
    pub fn changed(&self, changes: &[(Subject, u64)]) -> Vec<Effect> {
        let mut by_key: BTreeMap<&str, u64> = BTreeMap::new();
        for (subject, at) in changes {
            let Some(held) = self.subjects.get(subject) else {
                continue;
            };
            for key in &held.keys {
                let latest = by_key.entry(key).or_insert(*at);
                *latest = (*latest).max(*at);
            }
        }
        if by_key.is_empty() {
            return Vec::new();
        }
        let changes = by_key
            .into_iter()
            .map(|(key, at)| (key.to_owned(), at))
            .collect();
        vec![Effect::Page(FileWatchMessage::Changed { changes })]
    }

    /// Every subject the page holds on the Host.
    pub fn subjects(&self) -> impl Iterator<Item = &Subject> {
        self.subjects.keys()
    }
}
