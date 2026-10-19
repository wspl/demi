//! Filesystem and working-tree requests for one backend connection.

use std::{
    future::Future,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker},
};

use crate::{
    files::FileTransfers,
    git::{GitService, MAX_FILES},
    subscriptions::{Report, Subscriptions},
    watches::Watches,
};

pub struct HostServer {
    default_cwd: PathBuf,
    /// The connection's file system watches, which a direct channel's
    /// watches share (`runner.md` § Watching files).
    shared_watches: Watches,
    output: mpsc::Sender<wire::Frame>,
    filesystem: TaskTracker,
    filesystem_capacity: Arc<Semaphore>,
    git: GitService,
    git_capacity: Arc<Semaphore>,
    files: FileTransfers,
    /// The backend's subscriptions, until the connection closes.
    subscriptions: Mutex<Option<Subscriptions>>,
    /// Sends their reports as messages.
    _reports: AbortOnDropHandle<()>,
    cancel: CancellationToken,
}

impl Drop for HostServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.filesystem.close();
    }
}

impl HostServer {
    pub fn new(output: mpsc::Sender<wire::Frame>, default_cwd: PathBuf, pipes: PipeClient) -> Self {
        let cancel = CancellationToken::new();
        // The working tree's changes and the backend's subscriptions share
        // one watch per tree (`runner.md` § Watching files).
        let watches = Watches::default();
        let (subscriptions, reports) = Subscriptions::new(watches.clone(), default_cwd.clone());
        let reporting = AbortOnDropHandle::new(tokio::spawn(report(reports, output.clone(), cancel.clone())));
        Self {
            shared_watches: watches.clone(),
            subscriptions: Mutex::new(Some(subscriptions)),
            _reports: reporting,
            default_cwd,
            files: FileTransfers::new(output.clone(), pipes, cancel.clone()),
            output,
            filesystem: TaskTracker::new(),
            filesystem_capacity: Arc::new(Semaphore::new(32)),
            git: GitService::new(watches, MAX_FILES),
            git_capacity: Arc::new(Semaphore::new(8)),
            cancel,
        }
    }

    /// Filesystem work never blocks the connection's control-message loop.
    /// A file's contents go to the transfers, whose pipes pace them.
    pub fn handle_filesystem(&self, message: Inbound) -> io::Result<()> {
        match message {
            Inbound::FsReadFile { .. } => return self.files.read(message, &self.default_cwd),
            Inbound::FsWriteFile { .. } => return self.files.write(message, &self.default_cwd),
            Inbound::FsDownloadFile { .. } => return self.files.download(message, &self.default_cwd),
            Inbound::FsLook { .. } => return self.files.look(message, &self.default_cwd),
            Inbound::FsReadFiles { .. } => {
                return self.files.read_files(message, &self.default_cwd);
            }
            Inbound::FsWriteDirectory { .. } => {
                return self.files.write_directory(message, &self.default_cwd);
            }
            _ => {}
        }
        if message.fs_request_id().is_none() {
            return Err(io::Error::other("not a filesystem request"));
        }
        let cwd = self.default_cwd.clone();
        let cancel = self.cancel.clone();
        self.admit(&self.filesystem_capacity, "filesystem", async move {
            crate::fs::handle(&message, &cwd, &cancel)
                .await
                .expect("validated filesystem request")
        })
    }

    /// Working-tree work has its own admission: past eight requests in
    /// flight, later ones wait for a slot.
    pub fn handle_git(&self, message: Inbound) -> io::Result<()> {
        if message.git_request_id().is_none() {
            return Err(io::Error::other("not a working-tree request"));
        }
        if let Inbound::GitShow { .. } = message {
            if self.cancel.is_cancelled() {
                return Err(io::Error::other("host connection closed"));
            }
            return self.files.show(
                message,
                &self.default_cwd,
                self.git.clone(),
                self.git_capacity.clone(),
            );
        }
        let cwd = self.default_cwd.clone();
        let cancel = self.cancel.clone();
        let git = self.git.clone();
        self.admit(&self.git_capacity, "working-tree", async move {
            crate::git::handle(&git, &message, &cwd, &cancel)
                .await
                .expect("validated working-tree request")
        })
    }

    /// Runs `reply` once `capacity` has a slot (`runner.md` § Load) and sends
    /// what it answers; a request whose connection closes while it waits or
    /// runs ends without an answer.
    fn admit(
        &self,
        capacity: &Arc<Semaphore>,
        work: &'static str,
        reply: impl Future<Output = Result<wire::Frame, wire::WireError>> + Send + 'static,
    ) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        let capacity = capacity.clone();
        let cancel = self.cancel.clone();
        let output = self.output.clone();
        self.filesystem.spawn(async move {
            let _permit = tokio::select! {
                permit = capacity.acquire_owned() => permit.expect("request capacity is never closed"),
                _ = cancel.cancelled() => return,
            };
            let reply = tokio::select! {
                _ = cancel.cancelled() => return,
                reply = reply => reply,
            };
            match reply {
                Ok(reply) => {
                    tokio::select! {
                        _ = cancel.cancelled() => {},
                        result = output.send(reply) => {
                            if result.is_err() {
                                // The connection owner has closed its receiver.
                                cancel.cancel();
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!("{work} response encoding failed: {error}");
                    cancel.cancel();
                }
            }
        });
        Ok(())
    }

    /// Subscribes, renews or ends a subscription of the backend's
    /// (`runner.md` § Watching files), which reports as messages of its
    /// own until it ends.
    pub fn handle_watch(&self, message: Inbound) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        let held = self.subscriptions.lock().expect("the subscriptions are intact");
        let Some(subscriptions) = held.as_ref() else {
            return Err(io::Error::other("host connection closed"));
        };
        match message {
            Inbound::FsSubscribe { id, kind, path } => subscriptions.subscribe(id, kind, path),
            Inbound::FsUnsubscribe { id } => subscriptions.unsubscribe(id),
            Inbound::FsRenew { ids } => subscriptions.renew(ids),
            _ => return Err(io::Error::other("not a subscription request")),
        }
        Ok(())
    }

    /// The connection's file system watches, which a direct channel's
    /// subscriptions follow too.
    pub fn watches(&self) -> &Watches {
        &self.shared_watches
    }

    /// The connection's working-tree service.
    pub fn git(&self) -> &GitService {
        &self.git
    }

    pub async fn close(&self) {
        self.cancel.cancel();
        let ended = self.subscriptions.lock().expect("the subscriptions are intact").take();
        drop(ended);
        self.filesystem.close();
        tokio::join!(
            self.filesystem.wait(),
            self.files.close(),
            self.shared_watches.close(),
        );
    }
}

/// Sends the backend's subscriptions' reports as messages, until the
/// connection closes.
async fn report(mut reports: mpsc::Receiver<Report>, output: mpsc::Sender<wire::Frame>, cancel: CancellationToken) {
    loop {
        let report = tokio::select! {
            () = cancel.cancelled() => return,
            report = reports.recv() => match report {
                Some(report) => report,
                None => return,
            },
        };
        let message = match report {
            Report::Live { id, since } => Outbound::FsLive {
                id,
                since,
                epoch: crate::numbers::epoch().to_owned(),
            },
            Report::Failed { id, reason } => Outbound::FsFailed { id, reason },
            Report::Changed(changes) => Outbound::FsChanged { changes },
        };
        let frame = match wire::encode(&message) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!("a subscription's report could not be encoded: {error}");
                continue;
            }
        };
        tokio::select! {
            () = cancel.cancelled() => return,
            sent = output.send(frame) => if sent.is_err() {
                return;
            },
        }
    }
}
