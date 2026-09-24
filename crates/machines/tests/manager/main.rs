//! The manager as a process (`managed-hosts.md` § Startup and recovery): the
//! built executable, started the way its unit starts it, in a stand-in
//! execution host. Root runs these with `--ignored`. They leave the machine
//! as they found it: the host is PID 1 of new PID, mount and network
//! namespaces with its own `/proc`, `/run` and cgroup root, so the managers'
//! namespace handle, locks and firewall exist only there, and its end kills
//! everything inside.
#![cfg(target_os = "linux")]

use std::{
    io::{BufRead, BufReader, ErrorKind, Read},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::UnixDatagram,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use demi_machines::{
    config::RUNTIME_DIRECTORY,
    sandbox::runsc::RuntimeRelease,
    testing::{CloudImage, RUNNER, TINI, entries},
};
use demi_machines_protocol::image::Architecture;
use rustix::process::{Pid, Signal, kill_process};

const MANAGER: &str = env!("CARGO_BIN_EXE_demi-machines");

/// The handle and its owner record in the runtime directory
/// (`namespace.rs`).
const HANDLE: &str = "mount-namespace";
const OWNER: &str = "mount-namespace-owner.json";

/// A manager starts well within this.
const READY_DEADLINE: Duration = Duration::from_secs(60);

/// The host's init: the host's mounts are shared, as systemd's are, and its
/// `/run` is its own. Its cgroup root is a stand-in that offers the
/// controllers the manager enables, since no sandbox runs here.
const INIT: &str = "set -e
mount --make-rshared /
mount -t tmpfs -o mode=0755 tmpfs /run
mount -t tmpfs tmpfs /sys/fs/cgroup
echo 'cpu memory pids' > /sys/fs/cgroup/cgroup.controllers
echo ready
exec sleep infinity";

/// A mount namespace's identity: its device and inode.
type Namespace = (u64, u64);

fn identity(path: &Path) -> Option<Namespace> {
    match std::fs::metadata(path) {
        Ok(metadata) => Some((metadata.dev(), metadata.ino())),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => panic!("{}: {error}", path.display()),
    }
}

/// The manager's settings (`setup.md` § Configuration) for a state
/// directory, an image release and a runsc that only reports the pinned
/// version.
struct Settings {
    /// Where the manager reports readiness, as systemd's `NOTIFY_SOCKET`.
    notify: PathBuf,
    data: PathBuf,
    variables: Vec<(&'static str, String)>,
}

impl Settings {
    fn new(directory: &Path, image: &Path) -> Self {
        let runsc = directory.join("runsc");
        let version = RuntimeRelease::pinned().version();
        std::fs::write(
            &runsc,
            format!("#!/bin/sh\n[ \"$1\" = --version ] || {{ echo \"runsc $*\" >&2; exit 1; }}\necho 'runsc version {version}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&runsc, std::fs::Permissions::from_mode(0o755)).unwrap();
        let notify = directory.join("notify");
        let data = directory.join("data");
        Self {
            variables: vec![
                ("PATH", "/usr/sbin:/usr/bin:/sbin:/bin".to_owned()),
                ("NOTIFY_SOCKET", notify.display().to_string()),
                ("DEMI_MACHINES_SOCKET", directory.join("machines.sock").display().to_string()),
                ("DEMI_MACHINES_DATA", data.display().to_string()),
                ("DEMI_MANAGED_RUNSC", runsc.display().to_string()),
                ("DEMI_MANAGED_IMAGE", image.display().to_string()),
                ("DEMI_MANAGED_BACKEND_URL", "http://203.0.113.10:3271".to_owned()),
                ("DEMI_MANAGED_DNS", "1.1.1.1".to_owned()),
                ("DEMI_MANAGED_SLOTS", "4".to_owned()),
            ],
            notify,
            data,
        }
    }
}

/// The stand-in execution host: `unshare`'s child is its init, PID 1.
struct Host {
    unshare: Child,
}

impl Host {
    fn start() -> Self {
        let mut unshare = Command::new("unshare")
            .args(["--mount", "--net", "--pid", "--fork", "--mount-proc", "--kill-child"])
            .args(["--", "sh", "-c", INIT])
            .stdout(Stdio::piped())
            .spawn()
            .expect("unshare from util-linux");
        let mut line = String::new();
        BufReader::new(unshare.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(line, "ready\n", "the host did not start");
        Self { unshare }
    }

    /// A file of the runtime directory in the host's mount namespace, which
    /// `unshare` shares with its init.
    fn runtime_file(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("/proc/{}/root{RUNTIME_DIRECTORY}/{name}", self.unshare.id()))
    }

    /// The namespace the host's handle holds; `None` without a handle.
    fn handle(&self) -> Option<Namespace> {
        identity(&self.runtime_file(HANDLE))
    }

    /// Starts the manager as its unit does, with PrivateMounts=yes: in the
    /// host's PID and network namespaces and in a mount namespace of its own
    /// that receives the host's mounts.
    fn manager(&self, settings: &Settings, args: &[&str]) -> Manager {
        let unshare = self.unshare.id();
        let mut nsenter = Command::new("nsenter")
            .arg(format!("--mount=/proc/{unshare}/ns/mnt"))
            .arg(format!("--net=/proc/{unshare}/ns/net"))
            .arg(format!("--pid=/proc/{unshare}/ns/pid_for_children"))
            .args(["--", "unshare", "--mount", "--propagation", "slave", "--", MANAGER])
            .args(args)
            .env_clear()
            .envs(settings.variables.iter().map(|(name, value)| (name, value)))
            .stderr(Stdio::piped())
            .spawn()
            .expect("nsenter from util-linux");
        // nsenter forks the child that enters the PID namespace, and that
        // child becomes the manager.
        let children = format!("/proc/{0}/task/{0}/children", nsenter.id());
        let deadline = Instant::now() + READY_DEADLINE;
        let pid = loop {
            let listed = std::fs::read_to_string(&children).unwrap_or_default();
            if let Some(pid) = listed.split_whitespace().next() {
                break pid.parse().unwrap();
            }
            if let Some(status) = nsenter.try_wait().unwrap() {
                panic!("nsenter exited {status}: {}", stderr(&mut nsenter));
            }
            assert!(Instant::now() < deadline, "nsenter started no manager");
            std::thread::sleep(Duration::from_millis(10));
        };
        Manager {
            nsenter,
            pid: Pid::from_raw(pid).expect("a process id"),
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // unshare's end kills the init (--kill-child), and with it every
        // process of the host. Killing an unshare that already exited fails
        // with nothing left to end.
        let _ = self.unshare.kill();
        let _ = self.unshare.wait();
    }
}

fn stderr(child: &mut Child) -> String {
    let mut text = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut text).unwrap();
    }
    text
}

struct Manager {
    nsenter: Child,
    pid: Pid,
}

impl Manager {
    /// Waits for the readiness the unit's Type=notify waits for.
    fn wait_ready(&mut self, notify: &UnixDatagram) {
        notify.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let deadline = Instant::now() + READY_DEADLINE;
        let mut message = [0; 256];
        loop {
            match notify.recv(&mut message) {
                Ok(length) => {
                    let text = std::str::from_utf8(&message[..length]).unwrap();
                    if text.lines().any(|line| line == "READY=1") {
                        return;
                    }
                }
                Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(error) => panic!("reading readiness: {error}"),
            }
            if let Some(status) = self.nsenter.try_wait().unwrap() {
                panic!("the manager exited {status} before it was ready: {}", stderr(&mut self.nsenter));
            }
            assert!(Instant::now() < deadline, "the manager was not ready within {READY_DEADLINE:?}");
        }
    }

    fn namespace(&self) -> Namespace {
        identity(Path::new(&format!("/proc/{}/ns/mnt", self.pid))).expect("a running manager")
    }

    /// Ends the manager as a crash does: no drain and no stop-post recovery.
    fn kill(&mut self) {
        kill_process(self.pid, Signal::KILL).unwrap();
        let status = self.nsenter.wait().unwrap();
        assert!(!status.success(), "{status}");
    }

    /// Waits for a manager that exits by itself.
    fn finish(mut self) -> (std::process::ExitStatus, String) {
        let text = stderr(&mut self.nsenter);
        (self.nsenter.wait().unwrap(), text)
    }
}

/// A manager killed while it served leaves its namespace pinned by the
/// handle. The next start recovers through it, releases it and becomes
/// ready with its own; the unit's stop-post recovery of a killed manager
/// releases the handle as well.
#[test]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
fn the_next_start_recovers_and_releases_a_killed_managers_namespace() {
    let directory = tempfile::tempdir().unwrap();
    let image = CloudImage::new(&entries(), &[RUNNER, TINI], Architecture::host().unwrap());
    let settings = Settings::new(directory.path(), image.path());
    let notify = UnixDatagram::bind(&settings.notify).unwrap();
    let host = Host::start();

    let mut first = host.manager(&settings, &[]);
    first.wait_ready(&notify);
    let pinned = first.namespace();
    assert_eq!(host.handle(), Some(pinned), "a serving manager pins its namespace");
    let owner: serde_json::Value = serde_json::from_slice(&std::fs::read(host.runtime_file(OWNER)).unwrap()).unwrap();
    assert_eq!(owner, serde_json::json!({ "dataDir": settings.data }));
    first.kill();
    assert_eq!(host.handle(), Some(pinned), "a killed manager's namespace stays pinned");

    let mut second = host.manager(&settings, &[]);
    second.wait_ready(&notify);
    let own = second.namespace();
    assert_ne!(own, pinned);
    assert_eq!(host.handle(), Some(own), "the next start released the handle it recovered through");
    second.kill();

    let (status, errors) = host.manager(&settings, &["--recover"]).finish();
    assert!(status.success(), "the stop-post recovery exited {status}: {errors}");
    assert_eq!(host.handle(), None, "the stop-post recovery released the handle");
    assert!(!host.runtime_file(OWNER).exists());
}
