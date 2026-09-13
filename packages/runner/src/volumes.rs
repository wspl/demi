//! Managed volume growth and connection-owned filesystem flush work.

use crate::process::{ChildProcess, SpawnOptions};
use demi_runner_protocol as wire;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone)]
pub struct BlockVolume {
    pub name: String,
    pub device: PathBuf,
    pub mount: PathBuf,
}

enum Pending {
    Checking,
    Requested(String),
    Resizing,
}

pub struct Volumes {
    blocks: Vec<BlockVolume>,
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    tasks: TaskTracker,
    sync_capacity: Arc<Semaphore>,
    stop: CancellationToken,
    output: mpsc::Sender<wire::Outbound>,
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
        blocks: Vec<BlockVolume>,
        output: mpsc::Sender<wire::Outbound>,
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
        let permit = match self.sync_capacity.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let message = wire::sync_done(id, Some("filesystem sync capacity reached".into()))
                    .map_err(io::Error::other)?;
                return self.output.try_send(message).map_err(io::Error::other);
            }
        };
        let mounts: Vec<_> = self
            .blocks
            .iter()
            .map(|block| block.mount.clone())
            .collect();
        let output = self.output.clone();
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            let _permit = permit;
            let result = tokio::task::spawn_blocking(move || sync_filesystems(&mounts))
                .await
                .map_err(io::Error::other)
                .and_then(|result| result);
            let message = wire::sync_done(id, result.err().map(|error| error.to_string()));
            send(&output, message, &stop).await;
        });
        Ok(())
    }

    /// At most one outstanding request per volume, including filesystem resize.
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
                        send(&output, wire::volume_grow(id, volume.name, bytes), &stop).await;
                    }
                    outcome => {
                        pending.lock().unwrap().remove(&volume.name);
                        if let Err(error) = outcome {
                            eprintln!("demi-runner: volume capacity check: {error}");
                        }
                    }
                }
            });
        }
    }

    pub fn grown(&self, id: &str, name: &str, bytes: u64, error: Option<String>) -> io::Result<()> {
        let volume = self
            .blocks
            .iter()
            .find(|volume| volume.name == name)
            .ok_or_else(|| io::Error::other("unknown managed volume"))?
            .clone();
        let mut pending = self.pending.lock().unwrap();
        let Some(phase) = pending.get_mut(name) else {
            return Err(io::Error::other("unexpected volume growth response"));
        };
        if !matches!(phase, Pending::Requested(request) if request == id) {
            return Err(io::Error::other("unexpected volume growth response"));
        }
        *phase = Pending::Resizing;
        drop(pending);
        let pending = self.pending.clone();
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            let result = match error {
                Some(error) => Err(io::Error::other(error)),
                None => resize(&volume, &stop).await,
            };
            pending.lock().unwrap().remove(&volume.name);
            if let Err(error) = result {
                eprintln!(
                    "demi-runner: {} resize to {bytes} bytes failed: {error}",
                    volume.name
                );
            }
        });
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
    output: &mpsc::Sender<wire::Outbound>,
    message: Result<wire::Outbound, wire::WireError>,
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
            eprintln!("demi-runner: volume response encoding: {error}");
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

async fn resize(volume: &BlockVolume, stop: &CancellationToken) -> io::Result<()> {
    // The image grants the guest user exactly these resize2fs commands through
    // sudo. The long-lived runner keeps no root privileges or capabilities.
    if stop.is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "volume resize cancelled",
        ));
    }
    let mut child = ChildProcess::spawn(SpawnOptions {
        command: "sudo".into(),
        args: vec![
            "-n".into(),
            "resize2fs".into(),
            volume.device.to_string_lossy().into_owned(),
        ],
        cwd: "/".into(),
        env: BTreeMap::from([("PATH".into(), "/usr/sbin:/usr/bin:/sbin:/bin".into())]),
        process_group: true,
    })
    .await
    .map_err(|error| io::Error::other(error.message))?;
    child
        .input
        .send(crate::process::ProcessInput::End)
        .await
        .map_err(io::Error::other)?;
    let result = async {
        let mut diagnostics = Vec::new();
        while let Some(chunk) = child.output.recv().await {
            let remaining = (16 * 1024_usize).saturating_sub(diagnostics.len());
            diagnostics.extend_from_slice(&chunk.bytes[..chunk.bytes.len().min(remaining)]);
        }
        let exit = child.wait().await;
        if exit.code == Some(0) {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "resize2fs failed: {}",
                String::from_utf8_lossy(&diagnostics)
            )))
        }
    };
    tokio::select! {
        result = result => result,
        _ = stop.cancelled() => {
            child.cancel();
            child.wait().await;
            Err(io::Error::new(io::ErrorKind::Interrupted, "volume resize cancelled"))
        }
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
