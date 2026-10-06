//! The file watch (`web-api.md` § File watch, `sessions-and-targets.md`
//! § Host operations): the Host's reports of what changed in a
//! conversation's files, carried to a page that shows them. It is admitted
//! and ended as a user stream is, never wakes a stopped Cloud, and is no
//! activity. The shard follows the Host's watch of the conversation's
//! working tree and, non-recursively, of each path outside it the page
//! names; pages that watch the same Host paths share the Host's watches
//! (`RemoteHost::watch`). The edge carries the messages, and holds the lease.
//!
//! The order of the states the page hears is `PageWatch`'s, which a direct
//! channel's watch on the runner keeps too.

use std::collections::HashMap;

use demi_backend_remote_host::{HostWatch, RemoteHost, WatchUpdate};
use demi_host_interface::HostErrorKind;
use demi_runner_protocol::files::{PageWatch, WatchReport};
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
            watch: PageWatch::new(),
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

/// Which watch an update comes from: the working tree's, or a path's.
type Source = Option<String>;

struct Relay {
    host: RemoteHost,
    to_page: mpsc::Sender<FileWatchMessage>,
    updates: (
        mpsc::UnboundedSender<(Source, WatchUpdate)>,
        mpsc::UnboundedReceiver<(Source, WatchUpdate)>,
    ),
    /// The task following each path's watch outside the working tree.
    followed: HashMap<String, AbortOnDropHandle<()>>,
    /// The watches' states and what the page is owed of them.
    watch: PageWatch,
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
            let said = tokio::select! {
                () = &mut released => break End::Released,
                () = open.ended.cancelled() => break End::Released,
                paths = from_page.recv() => match paths {
                    Some(paths) => {
                        self.paths(paths);
                        Vec::new()
                    }
                    None => break End::Released,
                },
                Some((source, update)) = self.updates.1.recv() => {
                    let report = match update {
                        WatchUpdate::Ready => WatchReport::Ready,
                        WatchUpdate::Changed(paths) => WatchReport::Changed(paths),
                        WatchUpdate::Lost => WatchReport::Lost,
                        WatchUpdate::Failed(reason) => WatchReport::Failed(reason),
                        WatchUpdate::Ended => break End::Offline,
                    };
                    self.watch.report(source.as_deref(), report)
                }
            };
            let owed = self.watch.settle();
            if !self.send_all(said.into_iter().chain(owed)).await {
                break End::Released;
            }
        };
        // A Host that cannot watch said so, and says no other state.
        if matches!(end, End::Offline) && !self.watch.unavailable() {
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
        let changed = self.watch.paths(paths);
        for path in changed.stop {
            self.followed.remove(&path);
        }
        for path in changed.start {
            match self.host.watch(&path, false) {
                Ok(watch) => {
                    let task = self.follow(Some(path.clone()), watch);
                    self.followed.insert(path, task);
                }
                // The connection ended: the tree's watch says so.
                Err(_) => self.watch.forget(&path),
            }
        }
    }

    /// Sends `messages` in order; false once the edge is gone.
    async fn send_all(&self, messages: impl IntoIterator<Item = FileWatchMessage>) -> bool {
        for message in messages {
            if self.to_page.send(message).await.is_err() {
                return false;
            }
        }
        true
    }
}
