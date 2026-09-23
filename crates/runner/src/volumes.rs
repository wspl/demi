//! Managed volume growth and connection-owned filesystem flush work.

use crate::connection::wire::{self, VolumeName};
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{Semaphore, mpsc};
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

pub struct Volumes {
    blocks: Vec<ManagedVolume>,
    pending: Arc<Mutex<HashMap<VolumeName, Pending>>>,
    tasks: TaskTracker,
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
            pending: Arc::new(Mutex::new(HashMap::new())),
            tasks: TaskTracker::new(),
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
        self.tasks.spawn(async move {
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

    /// At most one outstanding request per volume, until the manager reports filesystem growth.
    pub fn poll(&self) {
        for volume in &self.blocks {
            let id = uuid::Uuid::new_v4().simple().to_string();
            let mut pending = self.pending.lock().unwrap();
            if pending.contains_key(&volume.name) {
                continue;
            }
            pending.insert(volume.name.clone(), Pending::Checking);
            drop(pending);
            let pending = self.pending.clone();
            let output = self.output.clone();
            let stop = self.stop.clone();
            let volume = volume.clone();
            self.tasks.spawn(async move {
                let mount = volume.mount.clone();
                let result = tokio::task::spawn_blocking(move || {
                    usage(&mount).and_then(|(total, available)| growth_wanted(total, available))
                })
                .await
                .map_err(io::Error::other)
                .and_then(|result| result);
                match result {
                    Ok(Some(bytes)) => {
                        pending
                            .lock()
                            .unwrap()
                            .insert(volume.name.clone(), Pending::Requested(id.clone()));
                        send(&output, wire::encode(&wire::Outbound::VolumeGrow {
                            id,
                            volume: volume.name,
                            bytes,
                        }), &stop).await;
                    }
                    outcome => {
                        pending.lock().unwrap().remove(&volume.name);
                        if let Err(error) = outcome {
                            crate::host_log::runner(format_args!("volume capacity check: {error}"));
                        }
                    }
                }
            });
        }
    }

    pub fn grown(
        &self,
        id: &str,
        name: VolumeName,
        bytes: u64,
        error: Option<String>,
    ) -> io::Result<()> {
        if !self.blocks.iter().any(|volume| volume.name == name) {
            return Err(io::Error::other("unknown managed volume"));
        }
        let mut pending = self.pending.lock().unwrap();
        if !matches!(pending.get(&name), Some(Pending::Requested(request)) if request == id) {
            return Err(io::Error::other("unexpected volume growth response"));
        }
        pending.remove(&name);
        if let Some(error) = error {
            crate::host_log::runner(format_args!(
                "{name} growth to {bytes} bytes failed: {error}"
            ));
        }
        Ok(())
    }

    pub async fn close(&self) {
        self.stop.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        self.pending.lock().unwrap().clear();
    }
}

impl Drop for Volumes {
    fn drop(&mut self) {
        self.stop.cancel();
        self.tasks.close();
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
            crate::host_log::runner(format_args!("volume response encoding: {error}"));
            stop.cancel();
        }
    }
}

fn sync_filesystems(mounts: &[PathBuf]) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if !mounts.is_empty() {
        use std::os::fd::AsRawFd;
        for mount in mounts {
            let file = std::fs::File::open(mount)?;
            if unsafe { libc::syncfs(file.as_raw_fd()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    let _ = mounts;
    #[cfg(unix)]
    {
        unsafe {
            libc::sync();
        }
        Ok(())
    }
    #[cfg(windows)]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "whole-filesystem sync is unavailable for a directory runner on Windows",
    ))
}

fn usage(mount: &std::path::Path) -> io::Result<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let path =
            std::ffi::CString::new(mount.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let mut usage = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), usage.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let usage = unsafe { usage.assume_init() };
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
