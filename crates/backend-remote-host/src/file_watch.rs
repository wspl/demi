//! A Host's file watches as the backend shares them (`web-api.md` § File
//! watch, `runner.md` § Watching files): one runner watch per path and depth
//! on a connection, followed by every page that watches the same Host path.
//! The first follower starts it with `fs_watch`, the last one's drop ends it
//! with `fs_unwatch`, and the connection's end ends every follower.

use std::{cell::Cell, collections::HashMap, rc::Rc};

use demi_runner_protocol::wire::Inbound;
use tokio::sync::{broadcast, watch};

use crate::link::Link;

/// How many reports a slow follower may fall behind before it hears that
/// reports were lost.
const BACKLOG: usize = 256;

/// What a follower of a Host's watch hears next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchUpdate {
    /// The Host's watch runs: a change from now on is reported.
    Ready,
    /// Absolute paths on the Host that something changed at, those of them
    /// that came, went or were renamed, and those git ignores in the
    /// watched path's repository.
    Changed {
        paths: Vec<String>,
        entries: Vec<String>,
        ignored: Vec<String>,
    },
    /// The watch lost reports: what it reported no longer tells what
    /// changed. It keeps running.
    Lost,
    /// The Host cannot watch the path, for this reason; nothing more comes.
    Failed(String),
    /// The connection to the Host ended; nothing more comes.
    Ended,
}

/// Where a shared watch is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Starting,
    Ready,
    Failed(String),
    Ended,
}

/// What a watch reports between its states.
#[derive(Debug, Clone)]
enum Report {
    Changed {
        paths: Vec<String>,
        entries: Vec<String>,
        ignored: Vec<String>,
    },
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct WatchKey {
    path: String,
    recursive: bool,
}

/// One runner watch of a connection, with its followers.
pub(crate) struct LinkWatch {
    id: String,
    key: WatchKey,
    followers: Cell<usize>,
    status: watch::Sender<Status>,
    reports: broadcast::Sender<Report>,
}

/// A connection's runner watches, by path and by the id the runner's
/// messages name.
#[derive(Default)]
pub(crate) struct LinkWatches {
    by_key: HashMap<WatchKey, Rc<LinkWatch>>,
    by_id: HashMap<String, Rc<LinkWatch>>,
}

impl LinkWatches {
    /// The watch `id` names, while the runner may still report for it.
    fn get(&self, id: &str) -> Option<&Rc<LinkWatch>> {
        self.by_id.get(id)
    }

    fn forget(&mut self, watch: &Rc<LinkWatch>) {
        if self
            .by_key
            .get(&watch.key)
            .is_some_and(|known| Rc::ptr_eq(known, watch))
        {
            self.by_key.remove(&watch.key);
        }
        self.by_id.remove(&watch.id);
    }

    /// The runner reports for watch `id`. A report of a watch that ended is
    /// one that crossed its `fs_unwatch`.
    pub(crate) fn ready(&self, id: &str) {
        if let Some(watch) = self.get(id) {
            watch.status.send_if_modified(|status| {
                let starting = *status == Status::Starting;
                if starting {
                    *status = Status::Ready;
                }
                starting
            });
        }
    }

    pub(crate) fn changed(&self, id: &str, paths: Vec<String>, entries: Vec<String>, ignored: Vec<String>) {
        if let Some(watch) = self.get(id) {
            // Nobody following now is nobody to tell.
            let _ = watch.reports.send(Report::Changed { paths, entries, ignored });
        }
    }

    pub(crate) fn lost(&self, id: &str) {
        if let Some(watch) = self.get(id) {
            let _ = watch.reports.send(Report::Lost);
        }
    }

    /// The watch reports nothing more: a new follower starts another.
    pub(crate) fn failed(&mut self, id: &str, reason: String) {
        if let Some(watch) = self.get(id).cloned() {
            watch.status.send_replace(Status::Failed(reason));
            self.forget(&watch);
        }
    }

    /// The connection ended: every follower hears it.
    pub(crate) fn end(&mut self) {
        for (_, watch) in self.by_id.drain() {
            watch.status.send_replace(Status::Ended);
        }
        self.by_key.clear();
    }
}

/// One follower of a Host's watch. Dropping it stops following; the last
/// follower's drop ends the runner's watch.
pub struct HostWatch {
    link: Link,
    watch: Rc<LinkWatch>,
    status: watch::Receiver<Status>,
    reports: broadcast::Receiver<Report>,
    /// The status this follower has been told.
    told: Status,
}

impl Link {
    /// Follows the runner's watch of `path`, and of what lies below it when
    /// `recursive`, starting it unless one runs. A connection that ended
    /// answers a follower that hears `Ended` at once.
    pub(crate) fn watch_files(&self, path: &str, recursive: bool) -> HostWatch {
        let key = WatchKey {
            path: path.to_owned(),
            recursive,
        };
        let known = self.with_watches(|watches| watches.by_key.get(&key).cloned());
        let watch = match known {
            Some(watch) => watch,
            None => {
                let id = uuid::Uuid::new_v4().simple().to_string();
                let watch = Rc::new(LinkWatch {
                    id: id.clone(),
                    key: key.clone(),
                    followers: Cell::new(0),
                    status: watch::Sender::new(if self.is_closed() {
                        Status::Ended
                    } else {
                        Status::Starting
                    }),
                    reports: broadcast::Sender::new(BACKLOG),
                });
                if !self.is_closed() {
                    self.with_watches(|watches| {
                        watches.by_key.insert(key.clone(), watch.clone());
                        watches.by_id.insert(id.clone(), watch.clone());
                    });
                    self.post(&Inbound::FsWatch {
                        id,
                        path: key.path,
                        recursive,
                    });
                }
                watch
            }
        };
        watch.followers.set(watch.followers.get() + 1);
        HostWatch {
            link: self.clone(),
            status: watch.status.subscribe(),
            reports: watch.reports.subscribe(),
            watch,
            told: Status::Starting,
        }
    }
}

impl HostWatch {
    /// What the watch says next. After `Failed` or `Ended` it says the same
    /// again.
    pub async fn next(&mut self) -> WatchUpdate {
        loop {
            let status = self.status.borrow_and_update().clone();
            if status != self.told {
                self.told = status.clone();
                match status {
                    Status::Ready => return WatchUpdate::Ready,
                    Status::Failed(reason) => return WatchUpdate::Failed(reason),
                    Status::Ended => return WatchUpdate::Ended,
                    Status::Starting => {}
                }
            }
            match &self.told {
                Status::Failed(reason) => return WatchUpdate::Failed(reason.clone()),
                Status::Ended => return WatchUpdate::Ended,
                Status::Starting | Status::Ready => {}
            }
            tokio::select! {
                biased;
                changed = self.status.changed() => {
                    if changed.is_err() {
                        return WatchUpdate::Ended;
                    }
                }
                report = self.reports.recv() => match report {
                    Ok(Report::Changed { paths, entries, ignored }) => {
                        return WatchUpdate::Changed { paths, entries, ignored };
                    }
                    Ok(Report::Lost) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        return WatchUpdate::Lost;
                    }
                    Err(broadcast::error::RecvError::Closed) => return WatchUpdate::Ended,
                },
            }
        }
    }
}

impl Drop for HostWatch {
    fn drop(&mut self) {
        let followers = self.watch.followers.get() - 1;
        self.watch.followers.set(followers);
        if followers > 0 {
            return;
        }
        let running = self.link.with_watches(|watches| {
            let running = watches
                .by_id
                .get(&self.watch.id)
                .is_some_and(|known| Rc::ptr_eq(known, &self.watch));
            watches.forget(&self.watch);
            running
        });
        if running {
            self.link.post(&Inbound::FsUnwatch {
                id: self.watch.id.clone(),
            });
        }
    }
}
