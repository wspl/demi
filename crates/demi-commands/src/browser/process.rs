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
    #[cfg(unix)]
    installation: std::path::PathBuf,
}

impl ChromeProcess {
    pub fn new(profile: &Path, executable: &Path) -> Self {
        #[cfg(unix)]
        {
            let mut marker = std::ffi::OsString::from(format!("{PROFILE_ENV}="));
            marker.push(profile);
            Self {
                group: None,
                marker,
                processes: System::new(),
                // macOS helpers live in the app's Frameworks directory; Linux
                // helpers live beside the main Chrome executable.
                installation: executable
                    .ancestors()
                    .find(|path| path.extension().is_some_and(|extension| extension == "app"))
                    .or_else(|| executable.parent())
                    .expect("absolute Chrome executable has a parent")
                    .to_owned(),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (profile, executable);
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
    pub fn observe(&mut self) {
        #[cfg(unix)]
        {
            self.processes.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
            );
            let candidates: Vec<_> = self
                .processes
                .processes()
                .iter()
                .filter(|(_, process)| {
                    process
                        .exe()
                        .is_some_and(|exe| exe.starts_with(&self.installation))
                })
                .map(|(pid, _)| *pid)
                .collect();
            self.processes.refresh_processes_specifics(
                ProcessesToUpdate::Some(&candidates),
                false,
                ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
            );
        }
    }

    /// Terminate and drain the group after chromiumoxide has reaped the leader.
    pub async fn terminate(&mut self) -> Result<()> {
        #[cfg(unix)]
        if let Some(group) = self.group {
            tokio::time::timeout(super::operation::CONTROL_TIMEOUT, async {
                loop {
                    // Reap any members adopted by this process, without consuming
                    // exit statuses belonging to other Chrome environments/jobs.
                    loop {
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
                    if !self.signal_owned(group)? {
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
    fn signal_owned(&mut self, group: libc::pid_t) -> io::Result<bool> {
        // ECHILD alone is insufficient: grandchildren can still be exiting or
        // awaiting reaping by their parent or the system's init process.
        let group_alive = signal_group(group, libc::SIGKILL)?;
        // sysinfo's Process::wait is synchronous and unbounded for non-children;
        // refreshing keeps the drain under the existing control deadline.
        self.observe();
        let mut helpers_alive = false;
        for process in self.processes.processes().values() {
            if process
                .exe()
                .is_some_and(|exe| exe.starts_with(&self.installation))
                && process.environ().contains(&self.marker)
            {
                helpers_alive = true;
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
        Ok(group_alive || helpers_alive)
    }
}

#[cfg(unix)]
impl Drop for ChromeProcess {
    fn drop(&mut self) {
        // Explicit retirement awaits termination. Abrupt owner disposal can only
        // signal synchronously; its profile remains retained for safety.
        if let Some(group) = self.group
            && let Err(error) = self.signal_owned(group)
        {
            eprintln!("could not terminate Chrome process tree {group}: {error}");
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
        owner.observe();
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
