//! A real runner process: the `demi-runner` the workspace built, with a
//! temporary home and state of its own, registered with the backend at a
//! URL. It prints its pairing codes when it waits to be paired, and keeps
//! its device token in its state across restarts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Child;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// What the runner prints before each pairing code.
const PAIRING_CODE: &str = "demi-runner: pairing code: ";

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

/// A program Cargo built beside the test, as every test finds one.
pub use demi_command_service::testing::built_program;

/// How a runner process starts.
#[derive(Debug, Clone)]
pub struct RunnerProcessOptions {
    /// The device name the runner reports.
    pub name: String,
    /// Variables of the runner's own environment: the device's.
    pub env: BTreeMap<String, String>,
    /// A device token in its state, so it connects as that device instead
    /// of waiting to be paired.
    pub token: Option<String>,
    /// Whether the runner says it is a managed host, as a Cloud's runner
    /// does.
    pub managed: bool,
}

impl Default for RunnerProcessOptions {
    fn default() -> Self {
        Self {
            name: "fixture".into(),
            env: BTreeMap::new(),
            token: None,
            managed: false,
        }
    }
}

/// A runner process with its home and state.
pub struct RunnerProcess {
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
        let home = tempfile::tempdir().expect("a temporary home");
        let home_path = std::fs::canonicalize(home.path())
            .expect("the home has a real path")
            .to_string_lossy()
            .into_owned();
        let state = tempfile::tempdir().expect("a temporary runner state");
        std::fs::create_dir(state.path().join("tmp")).expect("a temporary directory for the runner");
        if let Some(token) = &options.token {
            write_token(state.path(), token);
        }
        let mut process = Self {
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

    pub fn home_dir(&self) -> &Path {
        self.home.path()
    }

    /// The runner's installation state, which holds its device token.
    pub fn state_dir(&self) -> &Path {
        self.state.path()
    }

    /// Another runner set up as this one: the same home, state and backend.
    pub fn command(&self) -> tokio::process::Command {
        runner_command(&self.home_path, self.state.path(), &self.backend, &self.options)
    }

    /// What the runner printed.
    pub fn output(&self) -> String {
        self.output.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The `index`th pairing code the runner printed, once it has.
    pub async fn pairing_code(&self, index: usize) -> String {
        let mut codes = self.codes.subscribe();
        let printed = tokio::time::timeout(PAIRING, codes.wait_for(|codes| codes.len() > index)).await;
        match printed {
            Ok(Ok(codes)) => codes[index].clone(),
            _ => panic!("the runner printed no pairing code {index}:\n{}", self.output()),
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
        assert!(self.child.is_none(), "the runner still runs");
        for entry in std::fs::read_dir(self.state.path()).expect("the runner state can be listed") {
            let path = entry.expect("a runner state entry can be read").path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path).expect("a runner state directory can be removed");
            } else {
                std::fs::remove_file(&path).expect("a runner state file can be removed");
            }
        }
        std::fs::create_dir(self.state.path().join("tmp")).expect("a temporary directory for the runner");
    }

    fn spawn(&mut self) {
        let mut command = self.command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().unwrap_or_else(|error| {
            panic!(
                "start {}: {error}; build it with cargo build --workspace --features demi-runner/test-fixtures",
                runner_binary().display()
            )
        });
        let outputs = [
            child.stdout.take().map(|stream| Box::new(stream) as Box<dyn AsyncRead + Send + Unpin>),
            child.stderr.take().map(|stream| Box::new(stream) as Box<dyn AsyncRead + Send + Unpin>),
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

/// The runner's command line for a device: its home, its state and its
/// backend.
fn runner_command(home: &str, state: &Path, backend: &str, options: &RunnerProcessOptions) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(runner_binary());
    command
        .args(["run", "--backend", backend])
        .current_dir(home)
        .envs(&options.env)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("DEMI_HOME", state)
        // What the runner leaves in its temporary directory goes with the
        // process's state, even when it is killed.
        .env("TMPDIR", state.join("tmp"))
        .env("DEMI_RUNNER_NAME", &options.name)
        .stdin(Stdio::null())
        .kill_on_drop(true);
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
