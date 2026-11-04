//! Cgroups: with the resource limits on, every sandbox's Sentry and Gofer
//! live in `demi-cloud/<sandbox>`, which limits them together and lets the
//! manager kill all of them. With the limits off, nothing here runs.
//!
//! runsc makes the cgroup from the bundle's `cgroupsPath` on either
//! hierarchy: under the cgroup v2 root, or in each of the cgroup v1 `cpu`,
//! `memory` and `pids` hierarchies, as on a host booted with
//! `systemd.unified_cgroup_hierarchy=0`. The manager enables the v2
//! controllers for it; a v1 hierarchy has each controller at once.
//!
//! A fence kills the cgroup's processes with `cgroup.kill` from Linux 5.14,
//! and on an older kernel or a v1 hierarchy signals each process the cgroup
//! lists until none is left, as runc does.

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
    #[error(
        "Cloud resource limits need the cgroup cpu, memory and pids controllers at /sys/fs/cgroup; missing: {}. \
         DEMI_MANAGED_LIMITS=off runs Clouds without limits",
        .0.join(", ")
    )]
    Missing(Vec<&'static str>),
    #[error("Cloud runtime writers did not terminate")]
    Writers,
}

/// The hierarchy the host mounts at the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hierarchy {
    /// One unified hierarchy.
    V2,
    /// A hierarchy for each controller, `/sys/fs/cgroup/<controller>`.
    V1,
}

fn hierarchy() -> Hierarchy {
    if PathBuf::from(ROOT).join("cgroup.controllers").exists() {
        Hierarchy::V2
    } else {
        Hierarchy::V1
    }
}

/// The directory of the sandboxes' cgroups under the v2 root.
fn sandboxes() -> PathBuf {
    PathBuf::from(ROOT).join("demi-cloud")
}

/// Requires the CPU, memory and PID controllers and enables them for the
/// sandboxes' cgroups. Nothing under the root changes before all three are
/// there; an error names every one that is missing.
pub async fn prepare() -> Result<(), CgroupError> {
    blocking::run(|_| {
        let hierarchy = require_controllers()?;
        if hierarchy == Hierarchy::V2 {
            let enable = "+cpu +memory +pids";
            fs_err::write(PathBuf::from(ROOT).join("cgroup.subtree_control"), enable)?;
            fs_err::create_dir_all(sandboxes())?;
            fs_err::write(sandboxes().join("cgroup.subtree_control"), enable)?;
        }
        Ok(())
    })
    .await
}

/// Requires the CPU, memory and PID controllers, changing nothing.
pub async fn check() -> Result<(), CgroupError> {
    blocking::run(|_| require_controllers().map(|_| ())).await
}

/// The hierarchy, or an error naming every controller it does not offer:
/// one the v2 root does not list, or a v1 hierarchy that is not mounted.
fn require_controllers() -> Result<Hierarchy, CgroupError> {
    let hierarchy = hierarchy();
    let missing: Vec<_> = match hierarchy {
        Hierarchy::V2 => {
            let available = fs_err::read_to_string(PathBuf::from(ROOT).join("cgroup.controllers"))?;
            let available: Vec<_> = available.split_whitespace().collect();
            CONTROLLERS
                .into_iter()
                .filter(|controller| !available.contains(controller))
                .collect()
        }
        Hierarchy::V1 => CONTROLLERS
            .into_iter()
            .filter(|controller| !PathBuf::from(ROOT).join(controller).join("cgroup.procs").exists())
            .collect(),
    };
    if !missing.is_empty() {
        return Err(CgroupError::Missing(missing));
    }
    Ok(hierarchy)
}

/// Kills whatever runs in `sandbox`'s cgroup, waits for it to empty and
/// removes it: a partially created runtime is fenced even when runsc never
/// recorded it.
pub async fn fence(sandbox: &SandboxId) -> Result<(), CgroupError> {
    let name = sandbox.as_str().to_owned();
    let cgroups = blocking::run(move |_| cgroups(&name)).await?;
    let Some(kill) = cgroups.first().map(|cgroup| cgroup.join("cgroup.kill")) else {
        return Ok(());
    };
    let kills = blocking::run({
        let kill = kill.clone();
        move |_| kill.exists()
    })
    .await;
    if kills {
        blocking::run(move |_| fs_err::write(kill, "1")).await?;
    }
    let deadline = tokio::time::Instant::now() + FENCE_DEADLINE;
    loop {
        let left = blocking::run({
            let cgroups = cgroups.clone();
            move |_| -> io::Result<usize> {
                // A v1 hierarchy runsc had not reached lists no process.
                let mut processes = std::collections::BTreeSet::new();
                for cgroup in &cgroups {
                    match fs_err::read_to_string(cgroup.join("cgroup.procs")) {
                        Ok(listed) => processes.extend(listed.lines().filter_map(|line| line.parse::<i32>().ok())),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                if !kills {
                    for process in &processes {
                        if let Some(pid) = rustix::process::Pid::from_raw(*process) {
                            // A process that exited meanwhile is gone already.
                            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
                        }
                    }
                }
                Ok(processes.len())
            }
        })
        .await?;
        if left == 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(CgroupError::Writers);
        }
        tokio::time::sleep(FENCE_POLL).await;
    }
    blocking::run(move |_| -> io::Result<()> {
        for cgroup in cgroups {
            match fs_err::remove_dir(&cgroup) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    })
    .await?;
    Ok(())
}

/// `sandbox`'s cgroup in each hierarchy that has one: under the v2 root, or
/// in any v1 hierarchy, since runsc makes one in every v1 hierarchy the host
/// mounts, not only in those of the limits, and stops wherever it failed. A
/// v1 hierarchy of two controllers is listed again by each one's link to it.
fn cgroups(sandbox: &str) -> io::Result<Vec<PathBuf>> {
    let candidates = match hierarchy() {
        Hierarchy::V2 => vec![sandboxes().join(sandbox)],
        Hierarchy::V1 => fs_err::read_dir(ROOT)?
            .map(|entry| entry.map(|entry| entry.path().join("demi-cloud").join(sandbox)))
            .collect::<io::Result<_>>()?,
    };
    Ok(candidates.into_iter().filter(|cgroup| cgroup.exists()).collect())
}
