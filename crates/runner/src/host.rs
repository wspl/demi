//! Filesystem, working-tree and network requests for one backend connection.

use std::{future::Future, io, path::PathBuf, sync::Arc};

use crate::connection::wire::{self as wire, Inbound};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{files::FileTransfers, git::GitService, net::NetStreams, pipes::PipeClient};

pub struct HostServer {
    default_cwd: PathBuf,
    output: mpsc::Sender<wire::Frame>,
    filesystem: TaskTracker,
    filesystem_capacity: Arc<Semaphore>,
    git: GitService,
    git_capacity: Arc<Semaphore>,
    net: NetStreams,
    files: FileTransfers,
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
        Self {
            default_cwd,
            net: NetStreams::new(output.clone(), pipes.clone(), cancel.clone()),
            files: FileTransfers::new(output.clone(), pipes, cancel.clone()),
            output,
            filesystem: TaskTracker::new(),
            filesystem_capacity: Arc::new(Semaphore::new(32)),
            git: GitService::default(),
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

    /// One network stream request (`runner.md` § Network streams): the
    /// tracker owns the socket until the connection closes or both pipes end.
    pub fn handle_net(&self, message: Inbound) -> io::Result<()> {
        self.net.handle_open(message)
    }

    pub async fn close(&self) {
        self.cancel.cancel();
        self.filesystem.close();
        tokio::join!(self.filesystem.wait(), self.net.close(), self.files.close());
    }
}
