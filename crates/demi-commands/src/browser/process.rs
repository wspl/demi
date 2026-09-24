use std::{io, path::Path};

#[cfg(unix)]
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use tokio::process::{Child, Command};

use super::Result;

#[cfg(unix)]
const PROFILE_ENV: &str = "DEMI_BROWSER_PROFILE";

/// Own Chrome's Unix process group, including helpers left after the leader exits.
pub(super) struct ChromeProcess {
    #[cfg(unix)]
    group: Option<libc::pid_t>,
    #[cfg(unix)]
    marker: std::ffi::OsString,
    #[cfg(unix)]
    processes: System,
    /// Where Chrome executables that may carry the marker live.
    #[cfg(unix)]
    installations: Vec<std::path::PathBuf>,
}

impl ChromeProcess {
    pub fn new(profile: &Path, executable: &Path) -> Self {
        // macOS helpers live in the app's Frameworks directory; Linux helpers
        // live beside the main Chrome executable.
        let installation = executable
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .or_else(|| executable.parent())
            .expect("absolute Chrome executable has a parent")
            .to_owned();
        Self::marked(profile, vec![installation])
    }

    /// The processes marked with `profile` whose executable lies under one
    /// of `installations`: a browser's own, or those an orphaned browser left
    /// when its service ended without retiring it.
    pub fn marked(profile: &Path, installations: Vec<std::path::PathBuf>) -> Self {
        #[cfg(unix)]
        {
            let mut marker = std::ffi::OsString::from(format!("{PROFILE_ENV}="));
            marker.push(profile);
            Self {
                group: None,
                marker,
                processes: System::new(),
                installations,
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (profile, installations);
            Self {}
        }
    }

    pub fn spawn(&mut self, command: &mut Command) -> io::Result<Child> {
        #[cfg(unix)]
        {
            command.process_group(0);
            // Crashpad deliberately creates another session. This inherited,
            // environment-owned marker also identifies helpers outside our group.
            let (_, profile) = self
                .marker
                .as_encoded_bytes()
                .split_at(PROFILE_ENV.len() + 1);
            use std::os::unix::ffi::OsStrExt;
            command.env(PROFILE_ENV, std::ffi::OsStr::from_bytes(profile));
        }
        #[cfg(target_os = "linux")]
        {
            // The kernel ends Chrome's leader when the thread that started it
            // ends: a runtime worker, which lives as long as the service
            // (`browser.md` § Native driver).
            let service = std::process::id();
            // SAFETY: the hook runs in the child between fork and exec and
            // calls only async-signal-safe functions.
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    // The service may have ended before the signal was set.
                    if libc::getppid() as u32 != service {
                        return Err(io::Error::other("the service ended before Chrome started"));
                    }
                    Ok(())
                });
            }
        }
        let child = command.spawn()?;
        #[cfg(unix)]
        {
            self.group = Some(
                child
                    .id()
                    .and_then(|id| i32::try_from(id).ok())
                    .expect("a newly spawned Unix child has a positive pid_t ID"),
            );
        }
        Ok(child)
    }

    /// Capture helper identities before shutdown, retaining metadata while they exit.
    pub async fn observe(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            let installations = self.installations.clone();
            self.scan(move |processes| observe(processes, &installations))
                .await?;
        }
        Ok(())
    }

    /// Runs `work` over the process table on the blocking pool: refreshing
    /// every process's executable and environment takes milliseconds of CPU
    /// and waits on the kernel (`concurrency.md` § Blocking work). A scan
    /// whose waiter left finishes there; the next one reads the table anew.
    #[cfg(unix)]
    async fn scan<T: Send + 'static>(
        &mut self,
        work: impl FnOnce(&mut System) -> T + Send + 'static,
    ) -> io::Result<T> {
        let mut processes = std::mem::replace(&mut self.processes, System::new());
        let (processes, result) = tokio::task::spawn_blocking(move || {
            let result = work(&mut processes);
            (processes, result)
        })
        .await
        .map_err(io::Error::other)?;
        self.processes = processes;
        Ok(result)
    }

    /// Terminate and drain the group after chromiumoxide has reaped the
    /// leader, and the marked helpers; an orphan's have no group here.
    pub async fn terminate(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            tokio::time::timeout(super::operation::CONTROL_TIMEOUT, async {
                loop {
                    // Reap any members adopted by this process, without consuming
                    // exit statuses belonging to other Chrome environments/jobs.
                    while let Some(group) = self.group {
                        let result =
                            unsafe { libc::waitpid(-group, std::ptr::null_mut(), libc::WNOHANG) };
                        if result == 0 {
                            break;
                        }
                        if result == -1 {
                            let error = io::Error::last_os_error();
                            match error.raw_os_error() {
                                Some(libc::EINTR) => continue,
                                Some(libc::ECHILD) => break,
                                _ => return Err(error),
                            }
                        }
                    }
                    if !self.signal_owned().await? {
                        return Ok(());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "Chrome process tree did not exit")
            })??;
            self.group = None;
        }
        Ok(())
    }

    /// Signal Chrome's group and detached helpers; only disappearance completes retirement.
    #[cfg(unix)]
    async fn signal_owned(&mut self) -> io::Result<bool> {
        // ECHILD alone is insufficient: grandchildren can still be exiting or
        // awaiting reaping by their parent or the system's init process.
        let group_alive = match self.group {
            Some(group) => signal_group(group, libc::SIGKILL)?,
            None => false,
        };
        let installations = self.installations.clone();
        let marker = self.marker.clone();
        let helpers_alive = self
            .scan(move |processes| kill_helpers(processes, &installations, &marker))
            .await??;
        Ok(group_alive || helpers_alive)
    }
}

/// Reads which processes run a Chrome executable of `installations`, and the
/// environment of those, the only processes whose marker matters.
#[cfg(unix)]
fn observe(processes: &mut System, installations: &[std::path::PathBuf]) {
    processes.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let candidates: Vec<_> = processes
        .processes()
        .iter()
        .filter(|(_, process)| process.exe().is_some_and(|exe| installed(installations, exe)))
        .map(|(pid, _)| *pid)
        .collect();
    processes.refresh_processes_specifics(
        ProcessesToUpdate::Some(&candidates),
        false,
        ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
    );
}

/// Kills the helpers that carry `marker`; whether any still ran.
#[cfg(unix)]
fn kill_helpers(
    processes: &mut System,
    installations: &[std::path::PathBuf],
    marker: &std::ffi::OsStr,
) -> io::Result<bool> {
    // sysinfo's Process::wait is synchronous and unbounded for non-children;
    // refreshing keeps the drain under the existing control deadline.
    observe(processes, installations);
    let mut alive = false;
    for process in processes.processes().values() {
        if process.exe().is_some_and(|exe| installed(installations, exe))
            && process.environ().iter().any(|variable| variable == marker)
        {
            alive = true;
            if !process.kill() {
                let error = io::Error::last_os_error();
                // An already exited helper cannot write; refresh still
                // verifies disappearance before completion.
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error);
                }
            }
        }
    }
    Ok(alive)
}

/// Whether `executable` lies in one of the Chrome installations, the only
/// processes whose environment is read for the marker.
#[cfg(unix)]
fn installed(installations: &[std::path::PathBuf], executable: &Path) -> bool {
    installations
        .iter()
        .any(|installation| executable.starts_with(installation))
}

#[cfg(unix)]
impl Drop for ChromeProcess {
    fn drop(&mut self) {
        // Explicit retirement awaits termination. Abrupt owner disposal can only
        // signal synchronously; its profile remains retained for safety. Drop
        // cannot hand the scan to the blocking pool and wait for it, so this
        // one scan, when retirement did not run, stays on the dropping thread.
        if let Some(group) = self.group {
            let alive = signal_group(group, libc::SIGKILL).and_then(|_| {
                kill_helpers(&mut self.processes, &self.installations, &self.marker)
            });
            if let Err(error) = alive {
                tracing::warn!("could not terminate Chrome process tree {group}: {error}");
            }
        }
    }
}

/// Signal only this Chrome environment's group; ESRCH confirms the group is gone.
#[cfg(unix)]
fn signal_group(group: libc::pid_t, signal: libc::c_int) -> io::Result<bool> {
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    // XNU filters zombies from killpg's recipients and can return EPERM when
    // only exiting/zombie members remain. This means "still present", never
    // successful retirement: keep waiting for ESRCH under the same deadline.
    #[cfg(target_os = "macos")]
    if error.raw_os_error() == Some(libc::EPERM) {
        return Ok(true);
    }
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn detached_helper_fixture() {
        if std::env::var_os("DEMI_BROWSER_HELPER_FIXTURE").is_some() {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }

    #[tokio::test]
    async fn retirement_includes_marked_helpers_in_another_session_only() {
        let profile = tempfile::tempdir().unwrap();
        let mut owner = ChromeProcess::new(profile.path(), &std::env::current_exe().unwrap());
        let mut leader = owner
            .spawn(Command::new("sleep").arg("60").kill_on_drop(true))
            .unwrap();
        let mut helper_command = Command::new(std::env::current_exe().unwrap());
        helper_command
            .args([
                "--exact",
                "browser::process::tests::detached_helper_fixture",
            ])
            .env("DEMI_BROWSER_HELPER_FIXTURE", "1")
            .env(PROFILE_ENV, profile.path())
            .kill_on_drop(true);
        // Model Crashpad's deliberate escape from its Chrome client's session.
        unsafe {
            helper_command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut helper = helper_command.spawn().unwrap();
        let mut unrelated = Command::new("sleep")
            .arg("60")
            .env(PROFILE_ENV, profile.path())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        owner.observe().await.unwrap();
        assert!(
            owner
                .processes
                .process(sysinfo::Pid::from_u32(helper.id().unwrap()))
                .unwrap()
                .environ()
                .contains(&owner.marker)
        );
        leader.kill().await.unwrap();
        let (retired, reaped) = tokio::join!(owner.terminate(), helper.wait());
        retired.unwrap();
        assert!(reaped.unwrap().code().is_none());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().await.unwrap();
    }
}
