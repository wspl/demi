//! A real runner process: the `demi-runner` the workspace built, with a
//! temporary home and state of its own, registered with the backend at a
//! URL. It prints its pairing codes when it waits to be paired, and keeps
//! its device token in its state across restarts.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_command_protocol::testing::{BUILD_PROGRAMS, built_program};
use demi_host_interface::SpawnEnv;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Child;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use demi_runner_protocol::console::PAIRING_CODE;

/// How long a runner may take to print a pairing code.
const PAIRING: Duration = Duration::from_secs(15);

/// The runner tests start: `DEMI_RUNNER_TEST_BINARY`, else the `demi-runner`
/// the workspace built beside the test.
pub fn runner_binary() -> PathBuf {
    std::env::var_os("DEMI_RUNNER_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| built_program("demi-runner"))
}

/// The runner's native fixture service, which the one test selection builds
/// (`--features demi-runner/test-fixtures`).
pub fn native_fixture_binary() -> PathBuf {
    built_program("demi-native-fixture")
}

/// The file beside the stand-in in which it records the service-manager
/// commands it was given, one line each.
pub const SERVICE_MANAGER_CALLS: &str = "service-manager-calls";

/// Writes in `directory` the program that stands in for the service
/// manager's commands of the runners a test starts
/// ([`demi_runner_protocol::SERVICE_MANAGER_STAND_IN`]) and returns its
/// path. On Unix it records each command line in [`SERVICE_MANAGER_CALLS`]
/// beside it and keeps whether a LaunchAgent is loaded as launchd would;
/// asked to start the runner, by `launchctl kickstart` of a loaded agent or
/// `systemctl --user restart`, it starts the
/// launcher of the installation `DEMI_HOME` names, when there is one, in
/// the background, as the manager's runner; loading an agent starts none,
/// where launchd's would find the lock held and end, so that no runner of
/// the stand-in's outlives the test's. With `DEMI_TEST_MANAGER_REFUSES`
/// set, it refuses every command that would start the runner (a kickstart,
/// loading an agent, `systemctl --user start` or `restart`) and does every
/// other command, as a
/// manager that registered the installation but cannot start it: refusing
/// every command would fail the registration on Linux, where the runner
/// then never asks for a start. On Windows it answers every command with
/// success.
pub fn service_manager_stand_in(directory: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        const SCRIPT: &str = r#"#!/bin/sh
here=$(dirname "$0")
printf '%s\n' "$*" >> "$here/service-manager-calls"
# A test names a manager that refuses to start the runner: launchd's
# loading starts the agent's runner, as its kickstart does.
refuses() { [ -n "$DEMI_TEST_MANAGER_REFUSES" ] && exit 1; }
# The runner is told its installation on its command line too, which
# `DEMI_HOME` already names to it, so that `DetachedRunners` finds it there:
# a test's launcher may run the built runner, whose path names no
# installation, and such a runner was never stopped.
start() {
  refuses
  if [ -x "$DEMI_HOME/run" ]; then
    DEMI_SUPERVISED=1 "$DEMI_HOME/run" run --log --home "$DEMI_HOME" < /dev/null > /dev/null 2>&1 &
  fi
}
case "$1 $2" in
  'launchctl print') test -e "$here/loaded" ;;
  'launchctl bootstrap') refuses; : > "$here/loaded" ;;
  'launchctl bootout') rm -f "$here/loaded" ;;
  'launchctl kickstart') test -e "$here/loaded" && start ;;
  'systemctl --user') case "$3" in start|restart) start ;; esac ;;
esac
"#;
        executable(&directory.join("service-manager"), SCRIPT)
    }
    #[cfg(windows)]
    {
        let path = directory.join("service-manager.cmd");
        std::fs::write(&path, "@exit /b 0\r\n").expect("the service manager's stand-in can be written");
        path
    }
}

/// Writes in `directory` a login shell that reads no file of the user's and
/// runs the command it is given as `-i -l -c <command>` with the
/// environment it was given, and returns its path: the `SHELL` the runners
/// a test starts read their device environment from (`runner.md` § Host
/// operations), so that none reads the test machine's shell files.
#[cfg(unix)]
pub fn login_shell_stand_in(directory: &Path) -> PathBuf {
    const SCRIPT: &str = r#"#!/bin/sh
while [ "$#" -gt 1 ] && [ "$1" != -c ]; do shift; done
exec /bin/sh -c "$2"
"#;
    executable(&directory.join("login-shell"), SCRIPT)
}

/// The launcher an installer writes into an installation
/// (`builds-and-releases.md` § Packaging), which runs the installation's
/// release for its backend.
#[cfg(unix)]
const LAUNCHER: &str = r#"#!/bin/sh
set -eu
state=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IFS= read -r backend < "$state/backend-url"
IFS= read -r release < "$state/release-id"
export DEMI_HOME="$state" DEMI_RELEASE_ID="$release"
action=${1:-run}
[ "$#" -eq 0 ] || shift
exec "$state/releases/$release/demi-runner" "$action" --backend "$backend" "$@"
"#;

/// Writes `script` to `path` as a program and returns the path.
#[cfg(unix)]
pub fn executable(path: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, script).expect("a test program can be written");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("a test program can be made executable");
    path.to_owned()
}

/// How a runner process starts.
#[derive(Debug, Clone)]
pub struct RunnerProcessOptions {
    /// The device name the runner reports.
    pub name: String,
    /// The runner's own environment, the device's: the test process's
    /// (`Inherit`), with changes (`Overlay`), or exactly these variables, as
    /// a Cloud's image gives its runner (`Exactly`). The runner's home, state
    /// and name are set on top.
    pub env: SpawnEnv,
    /// A device token in its state, so it connects as that device instead
    /// of waiting to be paired.
    pub token: Option<String>,
    /// Whether the runner says it is a managed host, as a Cloud's runner
    /// does.
    pub managed: bool,
    /// The release of the installation the runner runs in, as an
    /// installer's launcher starts it: its executable lies in the state's
    /// `releases/<release>/`, so it downloads releases ahead and updates
    /// itself. None runs it as a build.
    pub release: Option<String>,
}

impl Default for RunnerProcessOptions {
    fn default() -> Self {
        Self {
            name: "fixture".into(),
            env: SpawnEnv::Inherit,
            token: None,
            managed: false,
            release: None,
        }
    }
}

/// A runner process with its home and state.
pub struct RunnerProcess {
    /// The runner executable it starts, again after each stop.
    program: PathBuf,
    home: tempfile::TempDir,
    home_path: String,
    state: tempfile::TempDir,
    backend: String,
    options: RunnerProcessOptions,
    child: Option<Child>,
    output: Arc<Mutex<String>>,
    codes: watch::Sender<Vec<String>>,
    readers: Vec<JoinHandle<()>>,
}

impl RunnerProcess {
    /// Starts a runner for the backend at `backend`, such as
    /// `http://127.0.0.1:3271`.
    pub fn start(backend: &str, options: RunnerProcessOptions) -> Self {
        Self::start_program(runner_binary(), backend, options)
    }

    /// [`RunnerProcess::start`] with `program` as the runner, which a caller
    /// that starts runners from a task of its own, as a Cloud stand-in does,
    /// resolves on the test's thread first: there [`runner_binary`]'s refusal
    /// of a stale program stops the test at once, where in the task it would
    /// leave whoever waits on the task waiting.
    pub fn start_program(program: PathBuf, backend: &str, options: RunnerProcessOptions) -> Self {
        let home = tempfile::tempdir().expect("a temporary home");
        let home_path = std::fs::canonicalize(home.path())
            .expect("the home has a real path")
            .to_string_lossy()
            .into_owned();
        let state = tempfile::tempdir().expect("a temporary runner state");
        std::fs::create_dir(state.path().join("tmp"))
            .expect("a temporary directory for the runner");
        if let Some(token) = &options.token {
            write_token(state.path(), token);
        }
        if let Some(release) = &options.release {
            let directory = state.path().join("releases").join(release);
            std::fs::create_dir_all(&directory).expect("the release's directory");
            let installed = directory.join(program.file_name().expect("the program has a name"));
            #[cfg(unix)]
            std::os::unix::fs::symlink(&program, installed).expect("the installed runner");
            #[cfg(windows)]
            std::fs::hard_link(&program, installed).expect("the installed runner");
            // The launcher an installer writes, which the service manager's
            // stand-in starts the runner with.
            #[cfg(unix)]
            {
                std::fs::write(state.path().join("backend-url"), format!("{backend}\n"))
                    .expect("the installation's backend");
                std::fs::write(state.path().join("release-id"), format!("{release}\n"))
                    .expect("the installation's release");
                executable(&state.path().join("run"), LAUNCHER);
            }
        }
        let mut process = Self {
            program,
            home,
            home_path,
            state,
            backend: backend.to_owned(),
            options,
            child: None,
            output: Arc::default(),
            codes: watch::Sender::new(Vec::new()),
            readers: Vec::new(),
        };
        process.spawn();
        process
    }

    /// The runner's home, where its Hosts start work.
    pub fn home(&self) -> &str {
        &self.home_path
    }

    /// The runner's home as a path, named as the runner names it: links
    /// followed, as macOS's temporary folders are under `/private`.
    pub fn home_dir(&self) -> &Path {
        Path::new(&self.home_path)
    }

    /// The runner's installation state, which holds its device token.
    pub fn state_dir(&self) -> &Path {
        self.state.path()
    }

    /// The service-manager commands the runner gave, one line each.
    pub fn service_manager_calls(&self) -> String {
        std::fs::read_to_string(self.home.path().join(SERVICE_MANAGER_CALLS)).unwrap_or_default()
    }

    /// How the runner process ended, once it has.
    pub fn exit_code(&mut self) -> Option<i32> {
        self.child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten())
            .and_then(|status| status.code())
    }

    /// Another runner set up as this one: the same home, state and backend.
    pub fn command(&self) -> tokio::process::Command {
        runner_command(
            &self.program,
            &self.home_path,
            self.state.path(),
            &self.backend,
            &self.options,
        )
    }

    /// What the runner printed.
    pub fn output(&self) -> String {
        self.output
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The `index`th pairing code the runner printed, once it has.
    pub async fn pairing_code(&self, index: usize) -> String {
        let mut codes = self.codes.subscribe();
        let printed =
            tokio::time::timeout(PAIRING, codes.wait_for(|codes| codes.len() > index)).await;
        match printed {
            Ok(Ok(codes)) => codes[index].clone(),
            _ => panic!(
                "the runner printed no pairing code {index}:\n{}",
                self.output()
            ),
        }
    }

    /// Whether the runner process still runs.
    pub fn running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    }

    /// Stops the runner, asking first and killing it after five seconds.
    pub async fn stop(&mut self) {
        if let Some(child) = self.child.take() {
            terminate(child).await;
        }
        for reader in self.readers.drain(..) {
            // A reader ends with the process's output.
            let _ = reader.await;
        }
    }

    /// The running runner's process id, which a script can signal.
    pub fn pid(&self) -> u32 {
        self.child
            .as_ref()
            .and_then(Child::id)
            .expect("the runner runs")
    }

    /// Stops the runner with `SIGSTOP`: it keeps its connection and answers
    /// nothing until [`RunnerProcess::resume`], as a Host whose reads never
    /// end.
    #[cfg(unix)]
    pub fn suspend(&self) {
        let pid = rustix::process::Pid::from_raw(self.pid() as i32).expect("a process id");
        rustix::process::kill_process(pid, rustix::process::Signal::STOP)
            .expect("the runner can be signalled");
    }

    /// Lets a runner that a script stopped with `SIGSTOP` go on.
    #[cfg(unix)]
    pub fn resume(&self) {
        let pid = rustix::process::Pid::from_raw(self.pid() as i32).expect("a process id");
        rustix::process::kill_process(pid, rustix::process::Signal::CONT)
            .expect("the runner can be signalled");
    }

    /// Kills the runner at once, as a crash would.
    pub async fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            // A runner that already exited has nothing to kill.
            let _ = child.kill().await;
        }
        for reader in self.readers.drain(..) {
            let _ = reader.await;
        }
    }

    /// Starts the runner again after a stop, with the same home and state.
    pub fn start_again(&mut self) {
        assert!(self.child.is_none(), "the runner still runs");
        self.spawn();
    }

    /// Starts the runner again after a stop for the backend at `backend`
    /// with `token` as its device token, as a Cloud boots with the boot
    /// record of that boot; its home and the rest of its state stay.
    pub fn start_again_with_token(&mut self, backend: &str, token: &str) {
        assert!(self.child.is_none(), "the runner still runs");
        backend.clone_into(&mut self.backend);
        write_token(self.state.path(), token);
        self.spawn();
    }

    /// Empties the runner's state while it is stopped, as a Cloud reset
    /// replaces the system the runner's state lives on; its home stays.
    pub fn clear_state(&mut self) {
        self.clear_state_but(&[]);
    }

    /// Empties the runner's state while it is stopped as a Cloud's stop
    /// does: its temporary state, `/run/demi` on a Cloud, goes, while its
    /// log and its job directories, which a Cloud keeps on its system image
    /// (`managed-hosts.md` § Images), and its home stay.
    pub fn clear_run_state(&mut self) {
        self.clear_state_but(&["log", "jobs"]);
    }

    fn clear_state_but(&mut self, kept: &[&str]) {
        assert!(self.child.is_none(), "the runner still runs");
        for entry in std::fs::read_dir(self.state.path()).expect("the runner state can be listed") {
            let entry = entry.expect("a runner state entry can be read");
            if kept.iter().any(|kept| entry.file_name() == *kept) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path).expect("a runner state directory can be removed");
            } else {
                std::fs::remove_file(&path).expect("a runner state file can be removed");
            }
        }
        std::fs::create_dir(self.state.path().join("tmp"))
            .expect("a temporary directory for the runner");
    }

    fn spawn(&mut self) {
        let mut command = self.command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().unwrap_or_else(|error| {
            panic!(
                "start {}: {error}; build it with {BUILD_PROGRAMS}",
                self.program.display()
            )
        });
        let outputs = [
            child
                .stdout
                .take()
                .map(|stream| Box::new(stream) as Box<dyn AsyncRead + Send + Unpin>),
            child
                .stderr
                .take()
                .map(|stream| Box::new(stream) as Box<dyn AsyncRead + Send + Unpin>),
        ];
        for output in outputs.into_iter().flatten() {
            let printed = self.output.clone();
            let codes = self.codes.clone();
            self.readers.push(tokio::spawn(async move {
                let mut lines = BufReader::new(output).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(code) = line.strip_prefix(PAIRING_CODE) {
                        codes.send_modify(|codes| codes.push(code.trim().to_owned()));
                    }
                    let mut printed = printed.lock().unwrap_or_else(PoisonError::into_inner);
                    printed.push_str(&line);
                    printed.push('\n');
                }
            }));
        }
        self.child = Some(child);
    }
}

impl Drop for RunnerProcess {
    fn drop(&mut self) {
        // A test that failed before `stop` still ends its runner.
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}

fn write_token(state: &Path, token: &str) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(state.join("runner-token"))
        .expect("the runner token can be written");
    writeln!(file, "{token}").expect("the runner token can be written");
}

/// The runner's command line for a device: its program, its home, its
/// state and its backend.
fn runner_command(
    program: &Path,
    home: &str,
    state: &Path,
    backend: &str,
    options: &RunnerProcessOptions,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    match &options.env {
        SpawnEnv::Inherit => {}
        SpawnEnv::Exactly(variables) => {
            command.env_clear().envs(variables);
        }
        SpawnEnv::Overlay(changes) => {
            for (name, value) in changes {
                match value {
                    Some(value) => command.env(name, value),
                    None => command.env_remove(name),
                };
            }
        }
    }
    command
        .args(["run", "--backend", backend])
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("DEMI_HOME", state)
        // What the runner leaves in its temporary directory goes with the
        // process's state, even when it is killed.
        .env("TMPDIR", state.join("tmp"))
        .env("DEMI_RUNNER_NAME", &options.name)
        .env(
            demi_runner_protocol::SERVICE_MANAGER_STAND_IN,
            service_manager_stand_in(Path::new(home)),
        )
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let named = |name: &str| match &options.env {
        SpawnEnv::Inherit => false,
        SpawnEnv::Overlay(changes) => changes.contains_key(name),
        SpawnEnv::Exactly(_) => true,
    };
    // The service manager's runner, which hands nothing over, unless the
    // test says otherwise.
    if !named(demi_runner_protocol::SUPERVISED) {
        command.env(demi_runner_protocol::SUPERVISED, "1");
    }
    // A login shell that reads none of the test machine's files, unless the
    // test names its own or gives the runner exactly its environment.
    #[cfg(unix)]
    if !named("SHELL") {
        command.env("SHELL", login_shell_stand_in(Path::new(home)));
    }
    if let Some(release) = &options.release {
        command.env(demi_runner_protocol::release::RELEASE_ENV, release);
    }
    if options.managed {
        command.env("DEMI_RUNNER_MANAGED", "1");
    } else {
        command.env_remove("DEMI_RUNNER_MANAGED");
    }
    command
}

/// Asks the runner to stop and waits for it, killing it after five seconds.
async fn terminate(mut runner: Child) {
    #[cfg(unix)]
    if let Some(pid) = runner
        .id()
        .and_then(|pid| rustix::process::Pid::from_raw(pid as i32))
    {
        // A runner that already exited has nothing to stop.
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    }
    if tokio::time::timeout(Duration::from_secs(5), runner.wait())
        .await
        .is_err()
    {
        // A runner that ignored the request is killed.
        let _ = runner.kill().await;
    }
}
