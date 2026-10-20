//! Connection-owned filesystem flush work: a `sync` from the backend flushes
//! a managed guest's writable filesystems, or every filesystem where none is
//! named, before the guest is saved.

use demi_runner_protocol::wire;
use std::{io, path::PathBuf, sync::Arc};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// A connection's flushes of the filesystems mounted at `mounts`.
pub struct Volumes {
    mounts: Vec<PathBuf>,
    syncs: TaskTracker,
    sync_capacity: Arc<Semaphore>,
    stop: CancellationToken,
    output: mpsc::Sender<wire::Frame>,
}

impl Volumes {
    pub fn new(
        mounts: Vec<PathBuf>,
        output: mpsc::Sender<wire::Frame>,
        stop: CancellationToken,
    ) -> Self {
        Self {
            mounts,
            output,
            stop,
            syncs: TaskTracker::new(),
            sync_capacity: Arc::new(Semaphore::new(4)),
        }
    }

    pub fn sync(&self, id: String) -> io::Result<()> {
        let capacity = self.sync_capacity.clone();
        let mounts = self.mounts.clone();
        let output = self.output.clone();
        let stop = self.stop.clone();
        self.syncs.spawn(async move {
            // Past the capacity a sync waits for a slot (`crates/runner-host/src/lib.rs`).
            let _permit = tokio::select! {
                permit = capacity.acquire_owned() => permit.expect("sync capacity is never closed"),
                _ = stop.cancelled() => return,
            };
            let result = tokio::task::spawn_blocking(move || sync_filesystems(&mounts))
                .await
                .map_err(io::Error::other)
                .and_then(|result| result);
            let message = wire::encode(&wire::Outbound::SyncDone {
                id,
                error: result.err().map(|error| error.to_string()),
            });
            send(&output, message, &stop).await;
        });
        Ok(())
    }

    pub async fn close(&mut self) {
        self.stop.cancel();
        self.syncs.close();
        self.syncs.wait().await;
    }
}

impl Drop for Volumes {
    fn drop(&mut self) {
        self.stop.cancel();
        self.syncs.close();
    }
}

async fn send(
    output: &mpsc::Sender<wire::Frame>,
    message: Result<wire::Frame, wire::WireError>,
    stop: &CancellationToken,
) {
    match message {
        Ok(message) => {
            tokio::select! {
                _ = stop.cancelled() => {},
                result = output.send(message) => {
                    if result.is_err() { stop.cancel(); }
                }
            }
        }
        Err(error) => {
            tracing::warn!("volume response encoding: {error}");
            stop.cancel();
        }
    }
}

/// Flushes the managed filesystems, or every filesystem where one cannot be
/// named.
fn sync_filesystems(mounts: &[PathBuf]) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if !mounts.is_empty() {
        for mount in mounts {
            rustix::fs::syncfs(std::fs::File::open(mount)?)?;
        }
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    let _ = mounts;
    #[cfg(unix)]
    {
        rustix::fs::sync();
        Ok(())
    }
    #[cfg(windows)]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "whole-filesystem sync is unavailable for a directory runner on Windows",
    ))
}
