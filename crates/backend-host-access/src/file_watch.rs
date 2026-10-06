//! The file watch (`web-api.md` § File watch, `sessions-and-targets.md`
//! § Host operations): the Host's reports of what changed in a
//! conversation's files, carried to a page that shows them. It is admitted
//! and ended as a user stream is, never wakes a stopped Cloud, and is no
//! activity. The shard follows the Host's watch of the conversation's
//! working tree and, non-recursively, of each path outside it the page
//! names; pages that watch the same Host paths share the Host's watches
//! (`RemoteHost::watch`). The edge carries the messages, and holds the lease.
//!
//! The page learns which of its paths are covered from the order of the
//! states: the first `live` covers the working tree; each `paths` message is
//! answered by one `live` once every watch it names runs; a `lost` is
//! followed by a `live` of its own once the watches run again. A Host that
//! cannot watch a path says `unavailable` once, and the watch says no state
//! after it.

use std::collections::HashMap;

use demi_backend_remote_host::{HostWatch, RemoteHost, WatchUpdate};
use demi_host_interface::HostErrorKind;
use demi_web_api_protocol::files::{FileWatchMessage, FileWatchState};
use demi_web_api_protocol::ids::ConversationId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::HostShard;
use crate::access::{Attention, HostAccessError, Refusal};
use crate::lease::{Lease, Released};
use crate::transfer::OpenTransfer;

/// The page's `paths` messages that may wait for the shard.
const FROM_PAGE: usize = 8;
/// The messages for the page that may wait for the edge.
const TO_PAGE: usize = 64;

/// An open file watch, as the edge carries it. `Send`.
pub struct FileWatchChannel {
    /// What the page hears, in order; it ends when the Host does, after an
    /// `offline`.
    pub to_page: mpsc::Receiver<FileWatchMessage>,
    /// The paths of each `paths` message of the page.
    pub from_page: mpsc::Sender<Vec<String>>,
    pub lease: Lease,
}

/// What opening a file watch answers.
pub enum OpenedWatch {
    Watching(FileWatchChannel),
    /// The Host is out of reach or a stopped Cloud, which the page hears as
    /// `offline`.
    Offline,
}

impl dyn HostShard + '_ {
    /// Opens a page's file watch of the conversation's primary Host:
    /// admitted as a user stream is, without waking a stopped Cloud and
    /// without activity, and open until the edge drops its lease, the Host's
    /// connection ends, or a transition ends it.
    pub async fn open_file_watch(
        &self,
        id: &ConversationId,
        cancel: &CancellationToken,
    ) -> Result<OpenedWatch, HostAccessError> {
        let access = match self.admit_stream(id, Attention::Looks, cancel).await {
            Ok(access) => access,
            Err(HostAccessError::Refused(Refusal::Stopped)) => return Ok(OpenedWatch::Offline),
            Err(HostAccessError::Host(error)) if matches!(error.kind, HostErrorKind::Offline) => {
                return Ok(OpenedWatch::Offline);
            }
            Err(error) => return Err(error),
        };
        let host = access.host.host.clone();
        let Ok(tree) = host.watch(&access.host.root, true) else {
            return Ok(OpenedWatch::Offline);
        };
        let (to_page, page_hears) = mpsc::channel(TO_PAGE);
        let (page_says, from_page) = mpsc::channel(FROM_PAGE);
        let (lease, released) = Lease::new(access.open.ended.clone());
        let relay = Relay {
            host,
            to_page,
            updates: mpsc::unbounded_channel(),
            followed: HashMap::new(),
            tree: Coverage::Starting,
            initial: false,
            rescan: false,
            answers: 0,
            unavailable: false,
        };
        self.tasks()
            .spawn_local(relay.run(tree, from_page, access.open, released));
        Ok(OpenedWatch::Watching(FileWatchChannel {
            to_page: page_hears,
            from_page: page_says,
            lease,
        }))
    }
}

/// Where one of the Host's watches is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Coverage {
    Starting,
    Running,
    Failed(String),
}

/// Which watch an update comes from: the working tree's, or a path's.
type Source = Option<String>;

/// A followed path outside the working tree: its watch's task, and where
/// the watch is.
struct Followed {
    _task: AbortOnDropHandle<()>,
    coverage: Coverage,
}

struct Relay {
    host: RemoteHost,
    to_page: mpsc::Sender<FileWatchMessage>,
    updates: (
        mpsc::UnboundedSender<(Source, WatchUpdate)>,
        mpsc::UnboundedReceiver<(Source, WatchUpdate)>,
    ),
    followed: HashMap<String, Followed>,
    tree: Coverage,
    /// The first `live` went out.
    initial: bool,
    /// A `lost` went out whose `live` is owed.
    rescan: bool,
    /// `paths` messages taken whose `live` is owed.
    answers: usize,
    /// The page heard that the Host cannot watch; no state follows.
    unavailable: bool,
}

/// Why the relay ended.
enum End {
    /// The edge let go, or a transition ended the watch.
    Released,
    /// The Host's connection ended: the page hears `offline`.
    Offline,
}

impl Relay {
    async fn run(
        mut self,
        tree: HostWatch,
        mut from_page: mpsc::Receiver<Vec<String>>,
        open: OpenTransfer,
        released: Released,
    ) {
        let _tree = self.follow(None, tree);
        tokio::pin!(released);
        let end = loop {
            tokio::select! {
                () = &mut released => break End::Released,
                () = open.ended.cancelled() => break End::Released,
                paths = from_page.recv() => match paths {
                    Some(paths) => self.paths(paths),
                    None => break End::Released,
                },
                Some((source, update)) = self.updates.1.recv() => {
                    match self.update(source, update).await {
                        Some(end) => break end,
                        None => {}
                    }
                }
            }
            if !self.settle().await {
                break End::Released;
            }
        };
        if let End::Offline = end {
            let offline = FileWatchMessage::State {
                state: FileWatchState::Offline,
                reason: None,
            };
            // A page that went meanwhile hears nothing.
            let _ = self.to_page.send(offline).await;
        }
        // The watches end with the relay, then its registration with the
        // conversation's transfers.
        drop(self);
        drop(open);
    }

    /// Runs a task that hands the relay `watch`'s updates until it is
    /// dropped.
    fn follow(&self, source: Source, mut watch: HostWatch) -> AbortOnDropHandle<()> {
        let updates = self.updates.0.clone();
        AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            loop {
                let update = watch.next().await;
                let last = matches!(update, WatchUpdate::Failed(_) | WatchUpdate::Ended);
                if updates.send((source.clone(), update)).is_err() || last {
                    return;
                }
            }
        }))
    }

    /// Follows the paths of a `paths` message, and only them.
    fn paths(&mut self, paths: Vec<String>) {
        self.followed.retain(|path, _| paths.contains(path));
        for path in paths {
            if self.followed.contains_key(&path) {
                continue;
            }
            let coverage = match self.host.watch(&path, false) {
                Ok(watch) => Followed {
                    _task: self.follow(Some(path.clone()), watch),
                    coverage: Coverage::Starting,
                },
                // The connection ended: the tree's watch says so.
                Err(_) => continue,
            };
            self.followed.insert(path, coverage);
        }
        self.answers += 1;
    }

    /// Takes one watch's update; the end it brings, if any.
    async fn update(&mut self, source: Source, update: WatchUpdate) -> Option<End> {
        let coverage = match &source {
            None => &mut self.tree,
            Some(path) => match self.followed.get_mut(path) {
                Some(followed) => &mut followed.coverage,
                // A path the page no longer names.
                None => return None,
            },
        };
        match update {
            WatchUpdate::Ready => *coverage = Coverage::Running,
            WatchUpdate::Failed(reason) => *coverage = Coverage::Failed(reason),
            WatchUpdate::Ended => return Some(End::Offline),
            WatchUpdate::Changed(paths) => {
                if !self.send(FileWatchMessage::Changed { paths }).await {
                    return Some(End::Released);
                }
            }
            WatchUpdate::Lost => {
                // Before the first `live` nothing was covered.
                if self.initial && !self.unavailable {
                    self.rescan = true;
                    let lost = FileWatchMessage::State {
                        state: FileWatchState::Lost,
                        reason: None,
                    };
                    if !self.send(lost).await {
                        return Some(End::Released);
                    }
                }
            }
        }
        None
    }

    /// Sends the states the watches now owe the page; false once the edge
    /// is gone.
    async fn settle(&mut self) -> bool {
        if self.unavailable {
            return true;
        }
        let coverages = std::iter::once(&self.tree)
            .chain(self.followed.values().map(|followed| &followed.coverage));
        let mut starting = false;
        let mut failed = None;
        for coverage in coverages {
            match coverage {
                Coverage::Starting => starting = true,
                Coverage::Failed(reason) => failed = failed.or(Some(reason.clone())),
                Coverage::Running => {}
            }
        }
        if let Some(reason) = failed {
            self.unavailable = true;
            return self
                .send(FileWatchMessage::State {
                    state: FileWatchState::Unavailable,
                    reason: Some(reason),
                })
                .await;
        }
        if starting {
            return true;
        }
        let owed = usize::from(!self.initial) + usize::from(self.rescan) + self.answers;
        self.initial = true;
        self.rescan = false;
        self.answers = 0;
        for _ in 0..owed {
            let live = FileWatchMessage::State {
                state: FileWatchState::Live,
                reason: None,
            };
            if !self.send(live).await {
                return false;
            }
        }
        true
    }

    async fn send(&self, message: FileWatchMessage) -> bool {
        self.to_page.send(message).await.is_ok()
    }
}
