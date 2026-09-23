//! Managed volume growth and connection-owned filesystem flush work.

use crate::connection::wire::{self, VolumeName};
use std::{collections::HashMap, io, path::PathBuf, sync::Arc};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone)]
pub struct ManagedVolume {
    pub name: VolumeName,
    pub mount: PathBuf,
}

enum Pending {
    Checking,
    Requested(String),
}

/// A connection's managed volumes (`managed-hosts.md` § Volume growth): at
/// most one growth request per volume is outstanding. The connection owns
/// this and drives the capacity checks it starts through `checked`.
pub struct Volumes {
    blocks: Vec<ManagedVolume>,
    pending: HashMap<VolumeName, Pending>,
    /// Capacity checks, each ending with its volume and the size to grow it
    /// to, if it runs short.
    checks: JoinSet<(VolumeName, Option<u64>)>,
    syncs: TaskTracker,
    sync_capacity: Arc<Semaphore>,
    stop: CancellationToken,
    output: mpsc::Sender<wire::Frame>,
}

pub fn growth_wanted(total: u64, available: u64) -> io::Result<Option<u64>> {
    if total == 0 {
        return Ok(None);
    }
    let reserve = (total / 10).max((256 * 1024 * 1024).min(total / 4));
    if available >= reserve {
        return Ok(None);
    }
    total
        .checked_mul(2)
        .filter(|bytes| *bytes <= 9_007_199_254_740_991)
        .map(Some)
        .ok_or_else(|| io::Error::other("volume growth exceeds the protocol size limit"))
}

impl Volumes {
    pub fn new(
        blocks: Vec<ManagedVolume>,
        output: mpsc::Sender<wire::Frame>,
        stop: CancellationToken,
    ) -> Self {
        Self {
            blocks,
            output,
            stop,
            pending: HashMap::new(),
            checks: JoinSet::new(),
            syncs: TaskTracker::new(),
            sync_capacity: Arc::new(Semaphore::new(4)),
        }
    }

    pub fn sync(&self, id: String) -> io::Result<()> {
        let capacity = self.sync_capacity.clone();
        let mounts: Vec<_> = self
            .blocks
            .iter()
            .map(|block| block.mount.clone())
            .collect();
        let output = self.output.clone();
        let stop = self.stop.clone();
        self.syncs.spawn(async move {
            // Past the capacity a sync waits for a slot (`runner.md` § Load).
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

    /// Checks each volume without an outstanding request, and asks the
    /// manager to grow one that runs short.
    pub fn poll(&mut self) {
        for volume in &self.blocks {
            if self.pending.contains_key(&volume.name) {
                continue;
            }
            self.pending.insert(volume.name.clone(), Pending::Checking);
            let volume = volume.clone();
            self.checks.spawn(async move {
                let mount = volume.mount.clone();
                let result = tokio::task::spawn_blocking(move || {
                    usage(&mount).and_then(|(total, available)| growth_wanted(total, available))
                })
                .await
                .map_err(io::Error::other)
                .and_then(|result| result);
                let wanted = result.unwrap_or_else(|error| {
                    tracing::warn!("volume capacity check: {error}");
                    None
                });
                (volume.name, wanted)
            });
        }
    }

    /// Takes the next capacity check that ended and asks for growth when it
    /// found the volume short, recording the request before sending it so
    /// its answer finds it; `None` while no check runs.
    pub async fn checked(&mut self) -> Option<()> {
        let (volume, wanted) = self
            .checks
            .join_next()
            .await?
            .expect("capacity checks do not panic");
        let Some(bytes) = wanted else {
            self.pending.remove(&volume);
            return Some(());
        };
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.pending.insert(volume.clone(), Pending::Requested(id.clone()));
        let request = wire::encode(&wire::Outbound::VolumeGrow { id, volume, bytes });
        send(&self.output, request, &self.stop).await;
        Some(())
    }

    pub fn grown(
        &mut self,
        id: &str,
        name: VolumeName,
        bytes: u64,
        error: Option<String>,
    ) -> io::Result<()> {
        if !self.blocks.iter().any(|volume| volume.name == name) {
            return Err(io::Error::other("unknown managed volume"));
        }
        if !matches!(self.pending.get(&name), Some(Pending::Requested(request)) if request == id) {
            return Err(io::Error::other("unexpected volume growth response"));
        }
        self.pending.remove(&name);
        if let Some(error) = error {
            tracing::warn!("{name} growth to {bytes} bytes failed: {error}");
        }
        Ok(())
    }

    pub async fn close(&mut self) {
        self.stop.cancel();
        self.syncs.close();
        self.checks.shutdown().await;
        self.syncs.wait().await;
        self.pending.clear();
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

/// A mounted filesystem's size and the space an unprivileged writer has left.
fn usage(mount: &std::path::Path) -> io::Result<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        let usage = rustix::fs::statvfs(mount)?;
        let total = usage
            .f_blocks
            .checked_mul(usage.f_frsize)
            .ok_or_else(|| io::Error::other("volume capacity overflow"))?;
        let free = usage
            .f_bavail
            .checked_mul(usage.f_frsize)
            .ok_or_else(|| io::Error::other("volume free-space overflow"))?;
        Ok((total, free))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = mount;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "managed block volumes require Linux",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn growth_reserve_has_fraction_floor_and_small_volume_cap() {
        let mib = 1024 * 1024;
        assert_eq!(growth_wanted(128 * mib, 32 * mib).unwrap(), None);
        assert_eq!(growth_wanted(128 * mib, 31 * mib).unwrap(), Some(256 * mib));
        assert_eq!(
            growth_wanted(2048 * mib, 255 * mib).unwrap(),
            Some(4096 * mib)
        );
        assert_eq!(growth_wanted(8192 * mib, 820 * mib).unwrap(), None);
        assert_eq!(
            growth_wanted(8192 * mib, 819 * mib).unwrap(),
            Some(16384 * mib)
        );
        assert!(growth_wanted(u64::MAX, 0).is_err());
    }
}
