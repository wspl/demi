//! A real runner process for a scripted Cloud sandbox: the `demi-runner` the
//! workspace built, with a temporary home and state of its own, connecting to
//! the backend as the device its token names. Its state, and so its token,
//! survives a stop.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use demi_command_protocol::testing::{BUILD_PROGRAMS, built_program};
use demi_host_interface::SpawnEnv;
use tokio::process::Child;

/// The `demi-runner` the workspace built beside this example.
pub fn runner_binary() -> PathBuf {
    built_program("demi-runner")
}

/// How a runner process starts.
#[derive(Debug, Clone)]
pub struct RunnerProcessOptions {
    /// The device name the runner reports.
    pub name: String,
    /// The runner's own environment, the device's: this process's
    /// (`Inherit`), with changes (`Overlay`), or exactly these variables, as
    /// a Cloud's image gives its runner (`Exactly`). The runner's home, state
    /// and name are set on top.
    pub env: SpawnEnv,
    /// The device token in its state, so it connects as that device.
    pub token: String,
}

/// A runner process with its home and state.
pub struct RunnerProcess {
    /// The runner executable it starts, again after each stop.
    program: PathBuf,
    #[allow(dead_code, reason = "held for its drop, which removes the home")]
    home: tempfile::TempDir,
    home_path: String,
    state: tempfile::TempDir,
    backend: String,
    options: RunnerProcessOptions,
    child: Option<Child>,
}

impl RunnerProcess {
    /// Starts `program` as the runner of the backend at `backend`, such as
    /// `http://127.0.0.1:3271`.
    pub fn start_program(program: PathBuf, backend: &str, options: RunnerProcessOptions) -> Self {
        let home = tempfile::tempdir().expect("a temporary home");
        let home_path = std::fs::canonicalize(home.path())
            .expect("the home has a real path")
            .to_string_lossy()
            .into_owned();
        let state = tempfile::tempdir().expect("a temporary runner state");
        std::fs::create_dir(state.path().join("tmp")).expect("a temporary directory for the runner");
        write_token(state.path(), &options.token);
        let mut process = Self {
            program,
            home,
            home_path,
            state,
            backend: backend.to_owned(),
            options,
            child: None,
        };
        process.spawn();
        process
    }

    /// Whether the runner process still runs.
    /// Its home, where the Cloud's `~` is.
    pub fn home(&self) -> &str {
        &self.home_path
    }

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
    /// (`crates/machine-manager/src/storage/mod.rs`), and its home stay.
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
        std::fs::create_dir(self.state.path().join("tmp")).expect("a temporary directory for the runner");
    }

    fn spawn(&mut self) {
        let mut command = runner_command(
            &self.program,
            &self.home_path,
            self.state.path(),
            &self.backend,
            &self.options,
        );
        // The runner writes its own log in its state.
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let child = command.spawn().unwrap_or_else(|error| {
            panic!(
                "start {}: {error}; build it with {BUILD_PROGRAMS}",
                self.program.display()
            )
        });
        self.child = Some(child);
    }
}

impl Drop for RunnerProcess {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}

fn write_token(state: &Path, token: &str) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(state.join("runner-token"))
        .expect("the runner token can be written");
    writeln!(file, "{token}").expect("the runner token can be written");
}

/// The runner's command line for a device: its program, its home, its state
/// and its backend.
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
        .env("DEMI_RUNNER_MANAGED", "1")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    // The service manager's runner, which hands nothing over, unless the
    // script says otherwise.
    let named = match &options.env {
        SpawnEnv::Inherit => false,
        SpawnEnv::Overlay(changes) => changes.contains_key(demi_runner_protocol::SUPERVISED),
        SpawnEnv::Exactly(_) => true,
    };
    if !named {
        command.env(demi_runner_protocol::SUPERVISED, "1");
    }
    command
}

/// Asks the runner to stop and waits for it, killing it after five seconds.
async fn terminate(mut runner: Child) {
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
