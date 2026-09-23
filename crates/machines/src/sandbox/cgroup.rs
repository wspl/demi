//! Cgroups (`managed-hosts.md` § Lifecycle and capacity): every sandbox's
//! Sentry and Gofer live in `demi-cloud/<sandbox>` under the cgroup v2 root,
//! which limits them together and lets the manager kill all of them.

use std::{io, path::PathBuf, time::Duration};

use super::files::SandboxId;
use crate::blocking;

const ROOT: &str = "/sys/fs/cgroup";
const CONTROLLERS: [&str; 3] = ["cpu", "memory", "pids"];

/// Writers get this long to exit once killed.
const FENCE_DEADLINE: Duration = Duration::from_secs(5);
const FENCE_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum CgroupError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Missing cgroup v2 {0} controller")]
    Missing(&'static str),
    #[error("Cloud runtime writers did not terminate")]
    Writers,
}

fn sandboxes() -> PathBuf {
    PathBuf::from(ROOT).join("demi-cloud")
}

/// Requires the CPU, memory and PID controllers and enables them for the
/// sandboxes' cgroups.
pub async fn prepare() -> Result<(), CgroupError> {
    blocking::run(|_| {
        let available = fs_err::read_to_string(PathBuf::from(ROOT).join("cgroup.controllers"))?;
        let available: Vec<_> = available.split_whitespace().collect();
        for controller in CONTROLLERS {
            if !available.contains(&controller) {
                return Err(CgroupError::Missing(controller));
            }
        }
        let enable = "+cpu +memory +pids";
        fs_err::write(PathBuf::from(ROOT).join("cgroup.subtree_control"), enable)?;
        fs_err::create_dir_all(sandboxes())?;
        fs_err::write(sandboxes().join("cgroup.subtree_control"), enable)?;
        Ok(())
    })
    .await
}

/// Kills whatever runs in `sandbox`'s cgroup, waits for it to empty and
/// removes it: a partially created runtime is fenced even when runsc never
/// recorded it.
pub async fn fence(sandbox: &SandboxId) -> Result<(), CgroupError> {
    let cgroup = sandboxes().join(sandbox.as_str());
    let killed = blocking::run({
        let cgroup = cgroup.clone();
        move |_| -> io::Result<bool> {
            match fs_err::write(cgroup.join("cgroup.kill"), "1") {
                Ok(()) => Ok(true),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error),
            }
        }
    })
    .await?;
    if !killed {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + FENCE_DEADLINE;
    loop {
        let events = blocking::run({
            let cgroup = cgroup.clone();
            move |_| fs_err::read_to_string(cgroup.join("cgroup.events"))
        })
        .await?;
        if events.lines().any(|line| line == "populated 0") {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(CgroupError::Writers);
        }
        tokio::time::sleep(FENCE_POLL).await;
    }
    blocking::run(move |_| fs_err::remove_dir(cgroup)).await?;
    Ok(())
}
